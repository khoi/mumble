use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::Instant;

use color_eyre::eyre::{Result, eyre};

use crate::dictation::DictationClip;
use crate::dictation::context::ContextSnapshot;
use crate::dictation::processor::ProcessingObservation;
use crate::history::{History, HistoryDraft, HistoryKind};
use crate::paste::{PasteMode, Paster};
use crate::suppression::InputActivity;
use crate::transcription::engine::{PendingTranscript, TranscriptionEngine};

const MAX_PENDING_OUTPUTS: usize = 16;

pub enum WorkerEvent {
    ModelFailed(String),
    Completed {
        job_id: DictationJobId,
        target: TranscriptionTarget,
        result: Result<String, String>,
        processing: Option<ProcessingObservation>,
    },
    Stage {
        job_id: DictationJobId,
        stage: DictationJobStage,
    },
    Cancelled {
        job_id: DictationJobId,
    },
    Pasted {
        result: Result<String, String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DictationJobId(u64);

impl DictationJobId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DictationJobStage {
    Transcribing,
    Processing,
}

#[derive(Clone, Copy, Debug)]
pub enum TranscriptionTarget {
    Paste,
    /// Dictate without processing: pasted as heard, skipping the Output
    /// pane's steps.
    RawPaste,
    Service,
    /// `hex dictation stop --print`: processed as a paste, then handed to
    /// the waiting command instead of pasted.
    Print,
    /// A printing stop of a raw dictation: the transcript as heard.
    RawPrint,
}

struct InferenceJob {
    job_id: DictationJobId,
    control: Arc<JobControl>,
    submitted_at: Instant,
    transcript: PendingTranscript,
    audio_ms: u64,
    prepare_ms: u128,
    target: TranscriptionTarget,
    context: ContextSnapshot,
}

struct ProcessorJob {
    job_id: DictationJobId,
    control: Arc<JobControl>,
    target: TranscriptionTarget,
    text: String,
    context: ContextSnapshot,
    timings: JobTimings,
}

/// Pipeline timings carried from inference through output for the final log.
#[derive(Clone, Copy)]
struct JobTimings {
    total_started: Instant,
    queue_ms: u128,
    audio_ms: u64,
    prepare_ms: u128,
    inference_ms: u128,
}

struct CompletedTranscript {
    text: String,
    /// Corrected local transcript before mode processing.
    raw: String,
    application: Option<String>,
    timings: JobTimings,
    processing: Option<ProcessingObservation>,
}

enum OutputJob {
    PreparePaste,
    Completed {
        job_id: DictationJobId,
        control: Arc<JobControl>,
        target: TranscriptionTarget,
        result: Box<Result<CompletedTranscript, String>>,
    },
    Cancelled {
        job_id: DictationJobId,
    },
    Paste {
        sequence: u64,
    },
}

impl OutputJob {
    fn sequence(&self) -> u64 {
        match self {
            Self::PreparePaste => u64::MAX,
            Self::Completed { job_id, .. } | Self::Cancelled { job_id } => job_id.0,
            Self::Paste { sequence, .. } => *sequence,
        }
    }
}

#[derive(Default)]
struct JobControl {
    cancelled: AtomicBool,
    output_started: Mutex<bool>,
    /// A printing stop's reply; dropping the job unanswered tells it the
    /// dictation was cancelled or failed.
    print: Mutex<Option<crate::control::PrintReply>>,
}

impl JobControl {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn answer_print(&self, transcript: crate::control::PrintedTranscript) {
        if let Some(reply) = self
            .print
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            reply.send(transcript);
        }
    }

    fn begin_output(&self) -> bool {
        let mut output_started = self
            .output_started
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.is_cancelled() {
            return false;
        }
        *output_started = true;
        true
    }
}

#[derive(Default)]
struct OrderedOutputs {
    waiting: BTreeMap<u64, OutputJob>,
    next_sequence: u64,
}

impl OrderedOutputs {
    fn push(&mut self, job: OutputJob) -> Vec<OutputJob> {
        if job.sequence() < self.next_sequence {
            return Vec::new();
        }
        self.waiting.insert(job.sequence(), job);
        let mut ready = Vec::new();
        while let Some(job) = self.waiting.remove(&self.next_sequence) {
            ready.push(job);
            self.next_sequence += 1;
        }
        ready
    }
}

pub struct DictationWorker {
    engine: Option<Arc<TranscriptionEngine>>,
    inference_jobs: Option<SyncSender<InferenceJob>>,
    output_jobs: Option<SyncSender<OutputJob>>,
    events: Receiver<WorkerEvent>,
    state: Arc<Mutex<WorkerState>>,
    inference_worker: Option<thread::JoinHandle<()>>,
    processor_workers: Vec<thread::JoinHandle<()>>,
    output_worker: Option<thread::JoinHandle<()>>,
}

struct WorkerState {
    next_sequence: u64,
    jobs: BTreeMap<DictationJobId, Arc<JobControl>>,
    pending_pastes: usize,
}

impl WorkerState {
    fn next_output_sequence(&self) -> Result<u64, &'static str> {
        // Count accepted work even after it leaves a channel for ordered buffering.
        if self.jobs.len() + self.pending_pastes >= MAX_PENDING_OUTPUTS {
            return Err("dictation queue is full");
        }
        Ok(self.next_sequence)
    }
}

impl DictationWorker {
    pub fn start(
        activity: InputActivity,
        transformations: Arc<crate::personal_commands::TransformationClient>,
        history: Option<History>,
    ) -> Self {
        const PROCESSOR_WORKERS: usize = 2;
        // Admission caps accepted jobs, so a submitted job always has a slot.
        let (inference_jobs, inference_receiver) =
            mpsc::sync_channel::<InferenceJob>(MAX_PENDING_OUTPUTS);
        let (processor_jobs, processor_receiver) = mpsc::sync_channel::<ProcessorJob>(4);
        let (output_jobs, output_receiver) = mpsc::sync_channel::<OutputJob>(8);
        let (event_sender, events) = mpsc::channel();
        let state = Arc::new(Mutex::new(WorkerState {
            next_sequence: 0,
            jobs: BTreeMap::new(),
            pending_pastes: 0,
        }));

        let engine = match TranscriptionEngine::start() {
            Ok(engine) => Some(Arc::new(engine)),
            Err(error) => {
                let _ = event_sender.send(WorkerEvent::ModelFailed(error.to_string()));
                None
            }
        };

        let output_worker = thread::spawn({
            let state = state.clone();
            let events = event_sender.clone();
            move || run_output_worker(output_receiver, activity, history, &state, &events)
        });

        let processor_receiver = Arc::new(Mutex::new(processor_receiver));
        let processor_workers = (0..PROCESSOR_WORKERS)
            .map(|_| {
                let jobs = processor_receiver.clone();
                let output = output_jobs.clone();
                let events = event_sender.clone();
                let transformations = transformations.clone();
                thread::spawn(move || {
                    run_processor_worker(&jobs, &output, &events, &transformations)
                })
            })
            .collect();

        let inference_worker = thread::spawn({
            let output = output_jobs.clone();
            let events = event_sender.clone();
            move || run_inference_worker(inference_receiver, &processor_jobs, &output, &events)
        });

        Self {
            engine,
            inference_jobs: Some(inference_jobs),
            output_jobs: Some(output_jobs),
            events,
            state,
            inference_worker: Some(inference_worker),
            processor_workers,
            output_worker: Some(output_worker),
        }
    }

    /// Hands the clip to the engine at once, so its live stream finishes
    /// before any later capture begins one; the job then waits in order.
    pub fn transcribe(
        &self,
        clip: DictationClip,
        target: TranscriptionTarget,
        context: ContextSnapshot,
    ) -> Result<DictationJobId, &'static str> {
        self.submit(clip, target, context, None)
    }

    /// Like a paste, but the transcript (processed unless `raw`) goes to
    /// `print`.
    pub fn transcribe_printed(
        &self,
        clip: DictationClip,
        print: crate::control::PrintReply,
        raw: bool,
        context: ContextSnapshot,
    ) -> Result<DictationJobId, &'static str> {
        let target = if raw {
            TranscriptionTarget::RawPrint
        } else {
            TranscriptionTarget::Print
        };
        self.submit(clip, target, context, Some(print))
    }

    fn submit(
        &self,
        mut clip: DictationClip,
        target: TranscriptionTarget,
        context: ContextSnapshot,
        print: Option<crate::control::PrintReply>,
    ) -> Result<DictationJobId, &'static str> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let job_id = DictationJobId(state.next_output_sequence()?);
        let (Some(engine), Some(jobs)) = (&self.engine, &self.inference_jobs) else {
            return Err("dictation worker is unavailable");
        };
        let control = Arc::new(JobControl {
            print: Mutex::new(print),
            ..JobControl::default()
        });
        let submitted_at = Instant::now();
        let audio_ms = clip.duration_ms();
        let stream = clip.take_stream();
        let _capture = clip.take_capture_marker();
        let samples = clip.into_transcription_samples();
        crate::dictation::diagnostics::persist(&samples);
        let prepare_ms = submitted_at.elapsed().as_millis();
        let transcript = engine
            .finish(stream, samples)
            .map_err(|_| "dictation worker is unavailable")?;
        jobs.try_send(InferenceJob {
            job_id,
            control: control.clone(),
            submitted_at,
            transcript,
            audio_ms,
            prepare_ms,
            target,
            context,
        })
        .map(|()| {
            state.next_sequence += 1;
            state.jobs.insert(job_id, control);
            job_id
        })
        .map_err(queue_error)
    }

    /// The engine that streams hotkey captures while they are recorded.
    pub fn engine(&self) -> Option<Arc<TranscriptionEngine>> {
        self.engine.clone()
    }

    pub fn prepare_paste(&self) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.jobs.is_empty() || state.pending_pastes > 0 {
            return;
        }
        drop(state);
        let Some(output_jobs) = &self.output_jobs else {
            return;
        };
        let _ = output_jobs.try_send(OutputJob::PreparePaste);
    }

    pub fn paste_last(&self) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let sequence = state.next_output_sequence()?;
        self.output_jobs
            .as_ref()
            .ok_or("dictation worker is unavailable")?
            .try_send(OutputJob::Paste { sequence })
            .map(|()| {
                state.next_sequence += 1;
                state.pending_pastes += 1;
            })
            .map_err(queue_error)
    }

    pub fn try_recv(&self) -> Option<WorkerEvent> {
        self.events.try_recv().ok()
    }

    pub fn is_busy(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        !state.jobs.is_empty() || state.pending_pastes > 0
    }

    pub fn pending_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .jobs
            .len()
    }

    fn shutdown(&mut self) {
        // Accepted jobs still finish: the inference worker drains its queue.
        self.inference_jobs.take();
        join_worker(self.inference_worker.take(), "dictation inference");
        for worker in self.processor_workers.drain(..) {
            join_worker(Some(worker), "dictation processing");
        }
        self.output_jobs.take();
        join_worker(self.output_worker.take(), "dictation output");
    }
}

impl Drop for DictationWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_output_worker(
    jobs: Receiver<OutputJob>,
    activity: InputActivity,
    history: Option<History>,
    state: &Mutex<WorkerState>,
    events: &mpsc::Sender<WorkerEvent>,
) {
    let mut paster = Paster::new(activity);
    let mut last_transcript = None;
    let mut ordered = OrderedOutputs::default();
    while let Ok(job) = jobs.recv() {
        if matches!(job, OutputJob::PreparePaste) {
            paster.prepare();
            continue;
        }
        for job in ordered.push(job) {
            let event = finish_output(
                job,
                &mut |text, mode, commit| paster.paste(text, mode, commit),
                &mut last_transcript,
                history.as_ref(),
            );
            let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
            match &event {
                WorkerEvent::Completed { job_id, .. } | WorkerEvent::Cancelled { job_id } => {
                    state.jobs.remove(job_id);
                }
                WorkerEvent::Pasted { .. } => {
                    state.pending_pastes = state.pending_pastes.saturating_sub(1);
                }
                WorkerEvent::ModelFailed(_) | WorkerEvent::Stage { .. } => {}
            }
            drop(state);
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

fn run_processor_worker(
    jobs: &Mutex<Receiver<ProcessorJob>>,
    output: &SyncSender<OutputJob>,
    events: &mpsc::Sender<WorkerEvent>,
    transformations: &crate::personal_commands::TransformationClient,
) {
    loop {
        let job = jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recv();
        let Ok(job) = job else { break };
        if job.control.is_cancelled() {
            let _ = output.send(OutputJob::Cancelled { job_id: job.job_id });
            continue;
        }
        let mut processed =
            process_job_text(&job, crate::dictation::config::dictation_profiles, || {
                let _ = events.send(WorkerEvent::Stage {
                    job_id: job.job_id,
                    stage: DictationJobStage::Processing,
                });
            });
        processed.apply_transformations(transformations, &job.context, &job.control.cancelled);
        if job.control.is_cancelled() {
            let _ = output.send(OutputJob::Cancelled { job_id: job.job_id });
            continue;
        }
        if let Some(observation) = &processed.observation
            && let Some(error) = &observation.fallback
        {
            tracing::warn!(
                profile = observation.profile,
                %error,
                "dictation processing fell back to the previous pipeline output"
            );
        }
        if output
            .send(OutputJob::Completed {
                job_id: job.job_id,
                control: job.control,
                target: job.target,
                result: Box::new(Ok(CompletedTranscript {
                    text: processed.text,
                    raw: job.text,
                    application: job.context.application,
                    timings: job.timings,
                    processing: processed.observation,
                })),
            })
            .is_err()
        {
            break;
        }
    }
}

/// Select processing when the worker reaches the job; the stage precedes processing.
fn process_job_text(
    job: &ProcessorJob,
    profiles: impl FnOnce() -> crate::dictation::processor::Profiles,
    processing: impl FnOnce(),
) -> crate::dictation::processor::Processed {
    let profiles = profiles();
    if profiles.processes(&job.context) {
        processing();
    }
    profiles.process_cancellable(&job.text, &job.context, &job.control.cancelled)
}

/// Waits for each accepted job's transcript in submission order, then hands
/// it to processing. The engine thread runs the inference itself.
fn run_inference_worker(
    jobs: Receiver<InferenceJob>,
    processor_jobs: &SyncSender<ProcessorJob>,
    output: &SyncSender<OutputJob>,
    events: &mpsc::Sender<WorkerEvent>,
) {
    while let Ok(job) = jobs.recv() {
        if job.control.is_cancelled() {
            let _ = output.send(OutputJob::Cancelled { job_id: job.job_id });
            continue;
        }
        let _ = events.send(WorkerEvent::Stage {
            job_id: job.job_id,
            stage: DictationJobStage::Transcribing,
        });
        let total_started = job.submitted_at;
        let queue_ms = total_started.elapsed().as_millis();
        let inference_started = Instant::now();
        let audio_ms = job.audio_ms;
        let result = job.transcript.wait().map(|transcribed| {
            tracing::info!(
                finish_ms = transcribed.finish_ms,
                audio_ms,
                "transcribed dictation"
            );
            let text = transcribed.text;
            let corrected = if matches!(job.target, TranscriptionTarget::Service) {
                text.clone()
            } else {
                text.trim().to_string()
            };
            tracing::debug!(
                raw_transcript = text,
                corrected_transcript = corrected,
                "prepared local transcript"
            );
            corrected
        });
        let timings = JobTimings {
            total_started,
            queue_ms,
            audio_ms,
            prepare_ms: job.prepare_ms,
            inference_ms: inference_started.elapsed().as_millis(),
        };
        if job.control.is_cancelled() {
            let _ = output.send(OutputJob::Cancelled { job_id: job.job_id });
            continue;
        }
        let target = job.target;
        match result {
            Ok(text)
                if matches!(
                    target,
                    TranscriptionTarget::Paste | TranscriptionTarget::Print
                ) && !text.trim().is_empty() =>
            {
                if processor_jobs
                    .send(ProcessorJob {
                        job_id: job.job_id,
                        control: job.control,
                        target,
                        text,
                        context: job.context,
                        timings,
                    })
                    .is_err()
                {
                    break;
                }
            }
            result => {
                let application = job.context.application;
                let result = result.map(|text| CompletedTranscript {
                    raw: text.clone(),
                    text,
                    application,
                    timings,
                    processing: None,
                });
                if output
                    .send(OutputJob::Completed {
                        job_id: job.job_id,
                        control: job.control,
                        target,
                        result: Box::new(result),
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

fn join_worker(worker: Option<thread::JoinHandle<()>>, name: &str) {
    if let Some(worker) = worker
        && worker.join().is_err()
    {
        tracing::error!(worker = name, "worker panicked during shutdown");
    }
}

fn queue_error(error: TrySendError<impl Sized>) -> &'static str {
    match error {
        TrySendError::Full(_) => "dictation queue is full",
        TrySendError::Disconnected(_) => "dictation worker is unavailable",
    }
}

fn finish_output(
    job: OutputJob,
    paste: &mut impl FnMut(&str, PasteMode, &dyn Fn() -> bool) -> Result<()>,
    last_transcript: &mut Option<String>,
    history: Option<&History>,
) -> WorkerEvent {
    match job {
        OutputJob::PreparePaste => unreachable!("paste preparation bypasses ordered output"),
        OutputJob::Completed {
            job_id,
            control,
            target,
            result,
        } if !control.is_cancelled() => {
            let result = *result;
            let processing = result
                .as_ref()
                .ok()
                .and_then(|result| result.processing.clone());
            let result = result.and_then(|completed| {
                if control.is_cancelled() {
                    return Err("dictation was cancelled".into());
                }
                let paste_started = Instant::now();
                if !completed.text.trim().is_empty() {
                    let commit = || control.begin_output();
                    match target {
                        TranscriptionTarget::Paste | TranscriptionTarget::RawPaste => {
                            // Labs › Copy dictation only, read as the output
                            // commits: dictations go to the clipboard
                            // unpasted; Paste last dictation still pastes.
                            let mode = if crate::labs::copy_only() {
                                PasteMode::CopyOnly
                            } else {
                                PasteMode::Continue
                            };
                            paste(&completed.text, mode, &commit)
                        }
                        TranscriptionTarget::Service
                        | TranscriptionTarget::Print
                        | TranscriptionTarget::RawPrint => commit()
                            .then_some(())
                            .ok_or_else(|| eyre!("dictation was cancelled")),
                    }
                    .map_err(|error| error.to_string())?;
                    if matches!(
                        target,
                        TranscriptionTarget::Paste | TranscriptionTarget::RawPaste
                    ) {
                        *last_transcript = Some(completed.text.clone());
                    }
                    if let Some(history) = history {
                        record_history(history, target, &completed);
                    }
                }
                tracing::info!(
                    audio_ms = completed.timings.audio_ms,
                    queue_ms = completed.timings.queue_ms,
                    prepare_ms = completed.timings.prepare_ms,
                    inference_ms = completed.timings.inference_ms,
                    paste_ms = paste_started.elapsed().as_millis(),
                    total_ms = completed.timings.total_started.elapsed().as_millis(),
                    "dictation pipeline completed"
                );
                Ok(completed.text)
            });
            if matches!(
                target,
                TranscriptionTarget::Print | TranscriptionTarget::RawPrint
            ) && !control.is_cancelled()
            {
                control.answer_print(match &result {
                    Ok(text) if text.trim().is_empty() => {
                        crate::control::PrintedTranscript::NoSpeech
                    }
                    Ok(text) => crate::control::PrintedTranscript::Text(text.clone()),
                    Err(error) => crate::control::PrintedTranscript::Failed(error.clone()),
                });
            }
            if control.is_cancelled() {
                WorkerEvent::Cancelled { job_id }
            } else {
                WorkerEvent::Completed {
                    job_id,
                    target,
                    result,
                    processing,
                }
            }
        }
        OutputJob::Completed { job_id, .. } | OutputJob::Cancelled { job_id } => {
            WorkerEvent::Cancelled { job_id }
        }
        OutputJob::Paste { .. } => {
            let result = last_transcript
                .as_deref()
                .ok_or_else(|| "no previous transcript is available".to_string())
                .and_then(|text| {
                    paste(text, PasteMode::Continue, &|| true)
                        .map_err(|error| error.to_string())?;
                    Ok(text.to_string())
                });
            WorkerEvent::Pasted { result }
        }
    }
}

/// Record one successfully pasted result. History failures must never fail
/// the paste that already happened.
fn record_history(history: &History, target: TranscriptionTarget, completed: &CompletedTranscript) {
    let kind = match target {
        TranscriptionTarget::Paste => HistoryKind::Dictation,
        TranscriptionTarget::RawPaste => HistoryKind::Raw,
        TranscriptionTarget::Print | TranscriptionTarget::RawPrint => HistoryKind::Printed,
        TranscriptionTarget::Service => return,
    };
    let draft = HistoryDraft {
        kind,
        raw_text: completed.raw.clone(),
        final_text: completed.text.clone(),
        application: completed.application.clone(),
        processing: completed.processing.clone(),
        audio_ms: completed.timings.audio_ms,
        inference_ms: completed.timings.inference_ms as u64,
        total_ms: completed.timings.total_started.elapsed().as_millis() as u64,
    };
    if let Err(error) = history.record(draft) {
        tracing::warn!(%error, "could not record dictation history");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn processing_job(target: TranscriptionTarget) -> ProcessorJob {
        ProcessorJob {
            job_id: DictationJobId(0),
            control: Arc::new(JobControl::default()),
            target,
            text: "raw instruction".into(),
            context: ContextSnapshot::default(),
            timings: JobTimings {
                total_started: Instant::now(),
                queue_ms: 0,
                audio_ms: 1000,
                prepare_ms: 0,
                inference_ms: 0,
            },
        }
    }

    #[test]
    fn ordinary_processing_snapshots_modes_before_stage_and_keeps_transformations() {
        use crate::dictation::processor::{Profile, Profiles};
        use std::cell::Cell;

        for transformations in [vec![], vec!["fixture-transform".to_string()]] {
            let loaded = Cell::new(false);
            let stage_sent = Cell::new(false);
            let processed = process_job_text(
                &processing_job(TranscriptionTarget::Paste),
                || {
                    assert!(!stage_sent.get());
                    loaded.set(true);
                    Profiles::new(
                        Profile::new("Global", "").transformations(transformations.clone()),
                    )
                },
                || {
                    assert!(loaded.get());
                    stage_sent.set(true);
                },
            );
            assert!(loaded.get());
            assert_eq!(stage_sent.get(), !transformations.is_empty());
            assert_eq!(processed.text, "raw instruction");
            assert_eq!(processed.transformations, transformations);
        }
    }

    #[test]
    fn parallel_processing_results_are_released_in_submission_order() {
        let mut outputs = OrderedOutputs::default();
        let job = |sequence| OutputJob::Paste { sequence };

        assert!(outputs.push(job(1)).is_empty());
        assert_eq!(
            outputs
                .push(job(0))
                .into_iter()
                .map(|job| job.sequence())
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            outputs
                .push(job(2))
                .into_iter()
                .map(|job| job.sequence())
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn ordered_waiting_outputs_remain_bounded_after_channel_drain() {
        let (output_sender, output_receiver) = mpsc::sync_channel(1);
        let (_event_sender, events) = mpsc::channel();
        let worker = DictationWorker {
            engine: None,
            inference_jobs: None,
            output_jobs: Some(output_sender),
            events,
            state: Arc::new(Mutex::new(WorkerState {
                next_sequence: 1,
                jobs: BTreeMap::from([(DictationJobId(0), Arc::new(JobControl::default()))]),
                pending_pastes: 0,
            })),
            inference_worker: None,
            processor_workers: Vec::new(),
            output_worker: None,
        };
        let mut ordered = OrderedOutputs::default();
        for _ in 1..MAX_PENDING_OUTPUTS {
            worker.paste_last().unwrap();
            assert!(ordered.push(output_receiver.try_recv().unwrap()).is_empty());
        }
        assert_eq!(worker.paste_last(), Err("dictation queue is full"));
        assert_eq!(ordered.waiting.len(), MAX_PENDING_OUTPUTS - 1);
        assert_eq!(
            worker.state.lock().unwrap().next_sequence,
            MAX_PENDING_OUTPUTS as u64
        );

        worker.state.lock().unwrap().jobs.remove(&DictationJobId(0));
        worker.paste_last().unwrap();
        assert_eq!(
            output_receiver.try_recv().unwrap().sequence(),
            MAX_PENDING_OUTPUTS as u64
        );
    }

    #[test]
    fn cancellation_replaces_waiting_output_and_ignores_late_completion() {
        let mut outputs = OrderedOutputs::default();
        let completed = |sequence| OutputJob::Completed {
            job_id: DictationJobId(sequence),
            control: Arc::new(JobControl::default()),
            target: TranscriptionTarget::Paste,
            result: Box::new(Err("late result".into())),
        };

        assert!(outputs.push(completed(1)).is_empty());
        assert!(
            outputs
                .push(OutputJob::Cancelled {
                    job_id: DictationJobId(1),
                })
                .is_empty()
        );
        let ready = outputs.push(OutputJob::Paste { sequence: 0 });
        assert_eq!(ready.len(), 2);
        assert!(matches!(ready[1], OutputJob::Cancelled { .. }));
        assert!(outputs.push(completed(1)).is_empty());
    }
}
