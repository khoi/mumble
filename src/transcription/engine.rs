//! The dictation coordinator. It owns no model: a capture streams to the
//! cloud service while the shortcut is held (`begin`), and on release the
//! clip and the stream's released answer are handed to the engine thread,
//! which runs the transcription in submission order (`finish`). Finishes run
//! one at a time on one thread, with a per-job deadline, cancellation, and
//! submission-order replies. There is no local fallback: a dictation the
//! service can't transcribe fails with its reason.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, eyre};

use crate::transcription::cloud::CloudAnswer;
use crate::transcription::{CancelToken, TranscriptionSelection};

/// How long a job may run once the engine starts it, before the clip's own
/// share. Time queued behind other work is never counted: the deadline only
/// catches a wedged engine, never a slow one.
const JOB_DEADLINE: Duration = Duration::from_secs(180);
/// Run time allowed per second of audio, so a long clip gets proportionally
/// longer before its job is declared stuck.
const JOB_SECONDS_PER_AUDIO_SECOND: u32 = 8;
/// How often a waiter whose job has not started checks whether the job ahead
/// of it timed out, leaving the engine wedged.
const QUEUE_POLL: Duration = Duration::from_millis(250);
/// How long dropping the engine waits for its thread.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);
/// Audio fed past the clip's trimmed end (a release handled late) the stream
/// may still keep. Beyond it, the whole clip is sent instead.
const MAX_OVERFED_SAMPLES: usize = 16_000 / 4;

#[derive(Clone, Copy)]
struct Deadlines {
    job: Duration,
    job_seconds_per_audio_second: u32,
    shutdown: Duration,
}

impl Deadlines {
    const PRODUCTION: Self = Self {
        job: JOB_DEADLINE,
        job_seconds_per_audio_second: JOB_SECONDS_PER_AUDIO_SECOND,
        shutdown: SHUTDOWN_DEADLINE,
    };

    fn job_for(&self, samples: usize) -> Duration {
        self.job
            + Duration::from_secs_f64(samples as f64 / 16_000.0) * self.job_seconds_per_audio_second
    }
}

pub(crate) enum Command {
    Finish {
        clip: Vec<f32>,
        job: Job,
        /// The capture's released cloud stream, whose answer comes first;
        /// `None` when no stream was open (its service refused one).
        cloud: Option<CloudAnswer>,
        /// The provider and language this one dictation uses, captured when
        /// the capture began, so a mid-dictation settings change never mixes a
        /// stream and its whole-clip retry across providers or languages.
        selection: TranscriptionSelection,
    },
    Shutdown,
}

/// A live stream's text so far, as the service reports it after each feed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Partial {
    /// The prefix the service has committed; it only grows within one stream.
    pub stable: String,
    /// The service's volatile hypothesis after `stable`.
    pub tentative: String,
}

/// One stream's latest partial text, shared between the stream's thread that
/// publishes it and the stream's owner. Each stream has its own, so a new
/// capture never reads an older one's text.
#[derive(Clone, Default)]
pub(crate) struct Partials(Arc<Mutex<Published>>);

#[derive(Default)]
struct Published {
    /// Bumped by each change, so an owner can tell what it has not seen.
    seq: u64,
    partial: Option<Partial>,
}

impl Partials {
    /// Publishes `partial`; returns whether it changed the text.
    pub(crate) fn publish(&self, partial: Partial) -> bool {
        let mut published = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if published.partial.as_ref() == Some(&partial) {
            return false;
        }
        published.seq += 1;
        published.partial = Some(partial);
        true
    }

    /// The latest partial when it is newer than `seen`, which it advances.
    fn newer_than(&self, seen: &mut u64) -> Option<Partial> {
        let published = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if published.seq <= *seen {
            return None;
        }
        *seen = published.seq;
        published.partial.clone()
    }
}

/// One finish request as the worker sees it: its own cancellation, so a
/// timeout aborts this job and never a later capture, and its reply.
pub(crate) struct Job {
    cancel: CancelToken,
    reply: Sender<Reply>,
}

impl Job {
    fn start(&self) {
        let _ = self.reply.send(Reply::Started);
    }

    fn finish(self, result: Result<Transcribed, String>) {
        let _ = self.reply.send(Reply::Done(result));
    }
}

enum Reply {
    /// The engine began this job; its deadline starts now.
    Started,
    Done(Result<Transcribed, String>),
}

#[derive(Debug)]
pub struct Transcribed {
    pub text: String,
    /// Time from the finish request to the final text.
    pub finish_ms: u64,
}

/// The token of the job the worker is running, so dropping the engine aborts
/// it. Work entered after the engine stops starts cancelled.
#[derive(Clone, Default)]
struct Running(Arc<Mutex<RunningState>>);

#[derive(Default)]
struct RunningState {
    stopping: bool,
    cancel: Option<CancelToken>,
}

impl Running {
    fn enter(&self, cancel: &CancelToken) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopping {
            cancel.cancel();
        }
        state.cancel = Some(cancel.clone());
    }

    fn stop(&self) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.stopping = true;
        if let Some(cancel) = &state.cancel {
            cancel.cancel();
        }
    }
}

pub struct TranscriptionEngine {
    commands: Sender<Command>,
    running: Running,
    /// Set when a job misses its deadline and cleared when the worker starts
    /// another: jobs queued behind a wedged run fail instead of waiting.
    stalled: Arc<AtomicBool>,
    /// Whether dictation streams publish their text as they go, asked as
    /// each stream begins: Labs › Live text, or a
    /// [`crate::dictation::live_text::Demand`].
    live_text: fn() -> bool,
    deadlines: Deadlines,
    thread: Option<JoinHandle<()>>,
}

/// A live stream for one capture: a connection to the cloud service. Feed it
/// the capture's 16 kHz audio in order, then either
/// [`TranscriptionEngine::finish`] or drop it to cancel.
pub struct LiveStream {
    fed: usize,
    /// The stream's text so far, with live text on, and the last version
    /// [`Self::take_partial`] returned.
    partials: Option<(Partials, u64)>,
    cloud: crate::transcription::cloud::CloudStream,
    /// The selection captured when this capture began, carried to the finish
    /// so one dictation stays on one provider and language throughout. Boxed to
    /// keep `LiveStream`, and the `DictationClip` that carries it, small.
    selection: Box<TranscriptionSelection>,
}

impl LiveStream {
    /// Total samples handed to the stream so far.
    pub fn fed(&self) -> usize {
        self.fed
    }

    /// The service's text for the audio fed so far, when it changed since the
    /// last call. Always `None` without live text. It is provisional: the
    /// finish may replace it entirely, so it is only ever shown, never pasted
    /// or kept.
    pub fn take_partial(&mut self) -> Option<Partial> {
        let (partials, seen) = self.partials.as_mut()?;
        partials.newer_than(seen)
    }

    pub fn feed(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        self.fed += samples.len();
        self.cloud.feed(samples);
    }
}

#[cfg(test)]
impl LiveStream {
    /// A live stream over a fake cloud boundary carrying a captured selection.
    pub(crate) fn for_test(
        cloud: crate::transcription::cloud::CloudStream,
        selection: TranscriptionSelection,
    ) -> Self {
        Self {
            fed: 0,
            partials: None,
            cloud,
            selection: Box::new(selection),
        }
    }
}

/// A finish in flight. Waiting bounds the job once the engine starts it, so a
/// wedged run fails instead of holding the paste forever.
pub struct PendingTranscript {
    reply: Receiver<Reply>,
    cancel: CancelToken,
    stalled: Arc<AtomicBool>,
    /// How long the job may run once started: its allowance plus the clip's.
    job: Duration,
}

impl PendingTranscript {
    pub fn wait(self) -> Result<Transcribed, String> {
        let mut job_deadline: Option<Instant> = None;
        loop {
            let now = Instant::now();
            let timeout = job_deadline.map_or(QUEUE_POLL, |deadline| {
                deadline.saturating_duration_since(now).min(QUEUE_POLL)
            });
            match self.reply.recv_timeout(timeout) {
                Ok(Reply::Done(result)) => return result,
                Ok(Reply::Started) => job_deadline = Some(Instant::now() + self.job),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("transcription engine stopped".into());
                }
                Err(RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    if job_deadline.is_some_and(|deadline| now >= deadline) {
                        tracing::error!(
                            deadline_ms = self.job.as_millis() as u64,
                            "transcription missed its deadline; failing the job"
                        );
                        self.stalled.store(true, Ordering::Release);
                        self.cancel.cancel();
                        return Err("transcription timed out".into());
                    }
                    if job_deadline.is_none() && self.stalled.load(Ordering::Acquire) {
                        tracing::error!("transcription engine is stuck; failing a queued job");
                        self.cancel.cancel();
                        return Err("transcription engine is stuck".into());
                    }
                }
            }
        }
    }
}

impl TranscriptionEngine {
    pub fn start() -> Result<Self> {
        let (commands, receiver) = mpsc::channel();
        let running = Running::default();
        let stalled = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            running: running.clone(),
            stalled: stalled.clone(),
        };
        let thread = thread::Builder::new()
            .name("transcription-engine".into())
            .spawn(move || worker.run(receiver))?;
        Ok(Self {
            commands,
            running,
            stalled,
            live_text: || crate::labs::live_text() || crate::dictation::live_text::demanded(),
            deadlines: Deadlines::PRODUCTION,
            thread: Some(thread),
        })
    }

    /// An engine without a worker; tests read the commands it would handle.
    #[cfg(test)]
    pub(crate) fn for_test() -> (Self, Receiver<Command>) {
        let (commands, receiver) = mpsc::channel();
        let engine = Self {
            commands,
            running: Running::default(),
            stalled: Arc::default(),
            live_text: || false,
            deadlines: Deadlines::PRODUCTION,
            thread: None,
        };
        (engine, receiver)
    }

    /// Opens a live stream to the selected provider's service, or `None` when
    /// a stream can't open (its service is refusing streams for this account).
    pub fn begin(&self) -> Option<LiveStream> {
        let partials = (self.live_text)().then(Partials::default);
        let selection = crate::app_settings::transcription_selection().1;
        let cloud = crate::transcription::cloud::CloudStream::open(
            selection.provider,
            selection.language.clone(),
            partials.clone(),
        )?;
        Some(LiveStream {
            fed: 0,
            partials: partials.map(|partials| (partials, 0)),
            cloud,
            selection: Box::new(selection),
        })
    }

    /// Finishes a capture with its complete 16 kHz clip. A stream's released
    /// answer is waited for first; without a stream, or when it heard past the
    /// clip, the whole clip is sent to the service.
    pub fn finish(&self, stream: Option<LiveStream>, clip: Vec<f32>) -> Result<PendingTranscript> {
        let fed = stream.as_ref().map_or(0, |stream| stream.fed);
        // The stream carries the selection captured when the capture began;
        // without one, this is the only attempt, so a single read is consistent.
        let selection = stream.as_ref().map_or_else(
            || crate::app_settings::transcription_selection().1,
            |stream| stream.selection.as_ref().clone(),
        );
        // Released at once, so the service finishes while earlier work runs.
        let cloud = stream.map(|stream| {
            let overfed = fed > clip.len() + MAX_OVERFED_SAMPLES;
            let tail = &clip[fed.min(clip.len())..];
            stream.cloud.end(tail, overfed)
        });
        self.request(clip.len(), |job| Command::Finish {
            clip,
            job,
            cloud,
            selection,
        })
    }

    fn request(
        &self,
        samples: usize,
        command: impl FnOnce(Job) -> Command,
    ) -> Result<PendingTranscript> {
        let (reply, receiver) = mpsc::channel();
        let cancel = CancelToken::new();
        let job = self.deadlines.job_for(samples);
        self.commands
            .send(command(Job {
                cancel: cancel.clone(),
                reply,
            }))
            .map_err(|_| eyre!("transcription engine stopped"))?;
        Ok(PendingTranscript {
            reply: receiver,
            cancel,
            stalled: self.stalled.clone(),
            job,
        })
    }
}

impl Drop for TranscriptionEngine {
    fn drop(&mut self) {
        self.running.stop();
        // A live stream's sender may outlive the engine; stop explicitly.
        let _ = self.commands.send(Command::Shutdown);
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now() + self.deadlines.shutdown;
        while !thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            tracing::error!("transcription engine did not stop in time; leaving it running");
        }
    }
}

struct Worker {
    running: Running,
    stalled: Arc<AtomicBool>,
}

impl Worker {
    fn run(self, commands: Receiver<Command>) {
        while let Ok(command) = commands.recv() {
            match command {
                Command::Finish {
                    clip,
                    job,
                    cloud,
                    selection,
                } => {
                    self.run_dictation(clip, job, cloud, selection);
                }
                Command::Shutdown => return,
            }
        }
    }

    /// Transcribes a finished dictation through the cloud service, in order.
    fn run_dictation(
        &self,
        clip: Vec<f32>,
        job: Job,
        cloud: Option<CloudAnswer>,
        selection: TranscriptionSelection,
    ) {
        let Some((job, started)) = self.admit(job) else {
            return;
        };
        self.running.enter(&job.cancel);
        let result = crate::transcription::cloud::dictation(selection, &clip, &job.cancel, cloud)
            .map(|text| Transcribed {
                text: text.trim().to_string(),
                finish_ms: started.elapsed().as_millis() as u64,
            });
        job.finish(result);
    }

    /// Starts `job` and its clock, unless its waiter already gave up on it.
    fn admit(&self, job: Job) -> Option<(Job, Instant)> {
        if job.cancel.is_cancelled() {
            job.finish(Err("transcription was abandoned".into()));
            return None;
        }
        let started = Instant::now();
        // The engine is evidently moving again.
        self.stalled.store(false, Ordering::Release);
        job.start();
        Some((job, started))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcription::CloudProvider;
    use crate::transcription::cloud;

    #[test]
    fn finishing_hands_the_clip_to_the_worker() {
        let (engine, commands) = TranscriptionEngine::for_test();
        let pending = engine.finish(None, vec![0.0; 7]).unwrap();
        let received = commands.try_iter().collect::<Vec<_>>();
        assert!(matches!(
            received.as_slice(),
            [Command::Finish { clip, cloud: None, .. }] if clip.len() == 7
        ));
        drop(pending);
    }

    #[test]
    fn a_finish_uses_the_streams_captured_selection_not_the_live_setting() {
        let (engine, commands) = TranscriptionEngine::for_test();
        // Not the default selection (Claude/en), so matching it proves finish
        // read the stream's captured selection, not the live global setting.
        let captured = TranscriptionSelection {
            provider: CloudProvider::Codex,
            language: "de".into(),
        };
        let (cloud, _heard) = cloud::fake(CloudProvider::Codex, Ok("x".into()));
        let stream = LiveStream::for_test(cloud, captured.clone());
        let pending = engine.finish(Some(stream), vec![0.0; 4]).unwrap();
        let received = commands.try_iter().collect::<Vec<_>>();
        match received.as_slice() {
            [Command::Finish { selection, .. }] => assert_eq!(*selection, captured),
            _ => panic!("expected one finish command carrying the captured selection"),
        }
        drop(pending);
    }

    fn selection(provider: CloudProvider) -> TranscriptionSelection {
        TranscriptionSelection {
            provider,
            language: "en".into(),
        }
    }

    fn live(reply: Result<String, cloud::CloudError>) -> LiveStream {
        let (cloud, _heard) = cloud::fake(CloudProvider::Claude, reply);
        LiveStream::for_test(cloud, selection(CloudProvider::Claude))
    }

    #[test]
    fn the_real_worker_returns_the_clouds_transcript() {
        let engine = TranscriptionEngine::start().unwrap();
        let text = engine
            .finish(Some(live(Ok(" hello there ".into()))), vec![0.0; 4])
            .unwrap()
            .wait()
            .unwrap()
            .text;
        assert_eq!(text, "hello there");
    }

    #[test]
    fn the_real_worker_finishes_jobs_in_submission_order() {
        let engine = TranscriptionEngine::start().unwrap();
        let first = engine
            .finish(Some(live(Ok("first".into()))), vec![0.0; 4])
            .unwrap();
        let second = engine
            .finish(Some(live(Ok("second".into()))), vec![0.0; 4])
            .unwrap();
        assert_eq!(first.wait().unwrap().text, "first");
        assert_eq!(second.wait().unwrap().text, "second");
    }

    #[test]
    fn the_real_worker_surfaces_a_cloud_failure_as_an_error() {
        let engine = TranscriptionEngine::start().unwrap();
        let result = engine
            .finish(
                Some(live(Err(cloud::CloudError::Failed("dropped".into())))),
                vec![0.0; 4],
            )
            .unwrap()
            .wait();
        assert!(result.is_err());
    }

    #[test]
    fn a_partial_reads_once_until_it_changes() {
        let partials = Partials::default();
        let mut seen = 0;
        assert_eq!(partials.newer_than(&mut seen), None);
        assert!(partials.publish(Partial {
            stable: "hello".into(),
            tentative: String::new(),
        }));
        assert_eq!(
            partials.newer_than(&mut seen),
            Some(Partial {
                stable: "hello".into(),
                tentative: String::new(),
            })
        );
        assert_eq!(partials.newer_than(&mut seen), None);
        assert!(!partials.publish(Partial {
            stable: "hello".into(),
            tentative: String::new(),
        }));
        assert_eq!(partials.newer_than(&mut seen), None);
    }
}
