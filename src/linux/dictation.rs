use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use color_eyre::Result;
use color_eyre::eyre::eyre;

use crate::app_settings::AppSettings;
use crate::audio::{CaptureInstant, RecoveringAudioInput, RecoveringAudioInputEvent};
use crate::control::{PrintReply, PrintedTranscript};
use crate::desktop::events::{
    DictationPhase, EventLog, TranscriptPhase, VoiceEvent, VoiceState, now_ms,
};
use crate::dictation::service::{DictationControl, Effect, MICROPHONE_LOST, ServiceDictations};
use crate::dictation::{DictationCapture, Finish, ReleaseWait};
use crate::feedback::{self, Tone};
use crate::linux::input::{
    CommandQueue, HotkeyEvent, HotkeyOptions, HotkeyStatus, LinuxHotkey, LinuxHotkeyMonitor,
};
use crate::linux::paste::LinuxPaster;
use crate::linux::rune_hud::LinuxIndicator;
use crate::transcription::engine::{PendingTranscript, TranscriptionEngine};

const UPDATE_INTERVAL: Duration = Duration::from_millis(20);

struct Job {
    transcript: PendingTranscript,
    audio_ms: u64,
    /// `hex dictation stop --print`: the text goes here, never pasted.
    print: Option<PrintReply>,
    /// Dictate without processing: paste the transcript as heard.
    raw: bool,
}

enum OutputJob {
    Dictation(Job),
    /// Pastes the newest completed dictation again.
    PasteLast,
}

#[derive(Clone, Copy, PartialEq)]
enum OutputKind {
    Dictation,
    /// A dictation printed by `hex dictation stop --print`.
    Printed,
    PasteLast,
}

/// A release whose capture is still collecting its post-release tail.
struct Finishing {
    release: ReleaseWait,
    print: Option<PrintReply>,
    raw: bool,
    released: Instant,
}

struct OutputWorker {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for OutputWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The local API's `/dictations` controls. The host keeps one queue for
/// whichever listener runs; while none does, a control goes unanswered and
/// the API answers `service-capture-unavailable`.
#[derive(Clone)]
pub(crate) struct DictationQueue(Arc<Mutex<mpsc::Receiver<DictationControl>>>);

impl From<mpsc::Receiver<DictationControl>> for DictationQueue {
    fn from(receiver: mpsc::Receiver<DictationControl>) -> Self {
        Self(Arc::new(Mutex::new(receiver)))
    }
}

/// A queue nothing sends to: a listener without the local API.
impl Default for DictationQueue {
    fn default() -> Self {
        mpsc::sync_channel(0).1.into()
    }
}

impl DictationQueue {
    fn drain(&self) -> Vec<DictationControl> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .try_iter()
            .collect()
    }
}

/// What the host hands a running listener. Settings apply in place while no
/// dictation records, so changing one never stops listening.
pub(crate) enum ListenerUpdate {
    Settings(Box<AppSettings>),
    /// True while the window records a shortcut: global matching stops.
    SuspendShortcuts(bool),
}

/// The shortcuts a listener registers for `settings`.
fn shortcuts(settings: &AppSettings) -> (LinuxHotkey, HotkeyOptions) {
    let options = HotkeyOptions {
        double_tap: settings.double_tap_lock,
        // Double-tap-only needs the double-tap window and a non-modifier key.
        double_tap_only: settings.double_tap_lock
            && settings.double_tap_only
            && settings.dictation_hotkey.key.is_some(),
        toggle: settings.shortcut_mode == crate::dictation::ShortcutMode::Toggle,
        // Labs › Tap or hold, off by default; it acts only in Toggle mode.
        tap_or_hold: settings.shortcut_mode == crate::dictation::ShortcutMode::Toggle
            && crate::labs::Feature::TapOrHold.on(settings),
        paste_last: settings.paste_last_hotkey.as_ref().map(LinuxHotkey::from),
        raw_dictation: settings
            .raw_dictation_hotkey
            .as_ref()
            .map(LinuxHotkey::from),
        cancel: settings.active_cancel_hotkey().map(LinuxHotkey::from),
    };
    (LinuxHotkey::from(&settings.dictation_hotkey), options)
}

/// Opens `--device`, else the microphone `settings` select, else the system
/// default, or keeps it released until a dictation starts.
fn microphone_input(settings: &AppSettings, device: Option<&str>) -> RecoveringAudioInput {
    let microphone = settings.microphone.as_deref();
    if settings.release_microphone_while_idle {
        RecoveringAudioInput::closed(device, 0, microphone)
    } else {
        RecoveringAudioInput::open(device, 0, microphone)
    }
}

/// What happens to a transcript before and after it is pasted: the modes
/// that process it and the history that keeps it. The host shares it
/// with the listener so saved settings apply without a restart.
#[derive(Clone)]
pub(crate) struct DictationOutput {
    pub(crate) history: Option<crate::history::History>,
    pub(crate) modes: Arc<RwLock<crate::dictation::modes::ModeRuntime>>,
    pub(crate) transformations: Arc<crate::personal_commands::TransformationClient>,
}

impl DictationOutput {
    pub(crate) fn from_settings(
        settings: &crate::app_settings::AppSettings,
        history: Option<crate::history::History>,
    ) -> Self {
        Self {
            history,
            modes: Arc::new(RwLock::new(
                crate::dictation::modes::ModeRuntime::from_settings(settings),
            )),
            transformations: Arc::new(crate::personal_commands::TransformationClient::default()),
        }
    }

    /// Processes (unless `raw`) and pastes one transcript, or only
    /// processes it when there is no paster (a printing stop), then records
    /// it.
    #[allow(clippy::too_many_arguments)]
    fn deliver(
        &self,
        raw_text: String,
        raw: bool,
        paster: Option<&mut LinuxPaster>,
        audio_ms: u64,
        inference_ms: u64,
        started: Instant,
    ) -> Result<String> {
        let context = crate::linux::context::capture();
        let processed = if raw {
            crate::dictation::processor::Processed::unchanged(raw_text.trim())
        } else {
            let modes = self
                .modes
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            modes.process(&raw_text, &context, &self.transformations)
        };
        if processed.text.trim().is_empty() {
            return Err(eyre!("text replacements produced empty output"));
        }
        let printed = paster.is_none();
        if let Some(paster) = paster {
            paster.paste(&processed.text)?;
        }
        if let Some(history) = &self.history {
            let mut draft = crate::dictation::modes::history_draft(
                raw_text,
                &processed,
                context.application,
                audio_ms,
                inference_ms,
                started.elapsed().as_millis() as u64,
            );
            if printed {
                draft.kind = crate::history::HistoryKind::Printed;
            } else if raw {
                draft.kind = crate::history::HistoryKind::Raw;
            }
            if let Err(error) = history.record(draft) {
                tracing::warn!(%error, "could not retain Linux dictation history");
            }
        }
        Ok(processed.text)
    }
}

/// Runs hotkey dictation until `shutdown`. `updates` change settings and
/// shortcut registration in place. `dictations` records a local API client's
/// dictation beside the shortcut: one capture at a time, and neither ends the
/// other.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    event_path: &Path,
    device: Option<&str>,
    shutdown: &AtomicBool,
    settings: &AppSettings,
    output_settings: DictationOutput,
    hotkey_status: HotkeyStatus,
    commands: CommandQueue,
    dictations: DictationQueue,
    updates: mpsc::Receiver<ListenerUpdate>,
) -> Result<()> {
    let mut settings = settings.clone();
    // The engine outlives the output worker, which waits on its transcripts.
    let engine = Arc::new(TranscriptionEngine::start()?);
    // Drop audio/hotkeys and close the job queue before joining slow output on
    // every exit, including failures during startup or recording.
    let mut output = OutputWorker {
        stop: Arc::new(AtomicBool::new(false)),
        worker: None,
    };
    // A missing microphone falls back to the default one. A released
    // microphone opens only while a dictation records.
    let mut input = microphone_input(&settings, device);
    let mut release_while_idle = settings.release_microphone_while_idle;
    let (binding, options) = shortcuts(&settings);
    let hotkey_label = binding.label();
    let mut toggle = options.toggle;
    let start_shortcuts = |(binding, options): (LinuxHotkey, HotkeyOptions)| {
        LinuxHotkeyMonitor::start(binding, options, hotkey_status.clone(), commands.clone())
    };
    let mut hotkey = Some(start_shortcuts((binding, options))?);
    let mut shortcuts_suspended = false;
    let mut indicator = LinuxIndicator::new(settings.indicator_position);
    let mut suppressor =
        crate::linux::audio_control::AudioSuppressor::dictation(settings.recording_audio_behavior);
    let paste_with_shift = Arc::new(AtomicBool::new(settings.paste_with_shift));
    let worker_paste_with_shift = paste_with_shift.clone();
    let paste_keys = hotkey_status.keys();
    let (jobs, job_receiver) = mpsc::sync_channel::<OutputJob>(2);
    let (result_sender, results) = mpsc::channel();
    let worker_stop = output.stop.clone();
    output.worker = Some(thread::spawn(move || {
        let mut paster = LinuxPaster::new(worker_stop.clone(), worker_paste_with_shift, paste_keys);
        let mut last_dictation: Option<String> = None;
        while let Ok(job) = job_receiver.recv() {
            if worker_stop.load(Ordering::Acquire) {
                break;
            }
            let (mut job, kind) = match job {
                OutputJob::Dictation(job) if job.print.is_some() => (job, OutputKind::Printed),
                OutputJob::Dictation(job) => (job, OutputKind::Dictation),
                OutputJob::PasteLast => {
                    let result = last_dictation
                        .clone()
                        .ok_or_else(|| eyre!("there is no completed dictation to paste"))
                        .and_then(|text| {
                            paster
                                .as_mut()
                                .map_err(|error| eyre!("{error:#}"))?
                                .paste(&text)
                        })
                        .map(|()| String::new())
                        .map_err(|error| format!("{error:#}"));
                    if result_sender.send((OutputKind::PasteLast, result)).is_err() {
                        break;
                    }
                    continue;
                }
            };
            let started = Instant::now();
            let result = (|| -> Result<String> {
                let transcribed = job.transcript.wait().map_err(|error| eyre!(error))?;
                let inference_ms = started.elapsed().as_millis() as u64;
                tracing::info!(
                    finish_ms = transcribed.finish_ms,
                    audio_ms = job.audio_ms,
                    "transcribed Linux dictation"
                );
                let text = transcribed.text;
                if text.is_empty() {
                    if let Some(print) = job.print.take() {
                        print.send(PrintedTranscript::NoSpeech);
                    }
                    return Err(eyre!("transcription was empty"));
                }
                if kind == OutputKind::Printed {
                    return output_settings.deliver(
                        text,
                        job.raw,
                        None,
                        job.audio_ms,
                        inference_ms,
                        started,
                    );
                }
                let paster = paster.as_mut().map_err(|error| eyre!("{error:#}"))?;
                output_settings.deliver(
                    text,
                    job.raw,
                    Some(paster),
                    job.audio_ms,
                    inference_ms,
                    started,
                )
            })()
            .map_err(|error| format!("{error:#}"));
            if let Some(print) = job.print.take() {
                print.send(match &result {
                    Ok(text) => PrintedTranscript::Text(text.clone()),
                    Err(error) => PrintedTranscript::Failed(error.clone()),
                });
            }
            tracing::info!(
                audio_ms = job.audio_ms,
                elapsed_ms = started.elapsed().as_millis(),
                "completed Linux dictation job"
            );
            if let Ok(text) = &result
                && kind == OutputKind::Dictation
            {
                last_dictation = Some(text.clone());
            }
            if result_sender.send((kind, result)).is_err() {
                break;
            }
        }
    }));

    let mut events = EventLog::create(event_path)?;
    let mut audio_ready = input.is_open();
    // A released microphone's sample rate is unknown until it opens; the
    // first chunk replaces this placeholder capture.
    let mut capture = DictationCapture::new(if audio_ready {
        input.sample_rate()
    } else {
        16_000
    });
    let released_label = |settings: &AppSettings| {
        device
            .or(settings.microphone.as_deref())
            .unwrap_or("System default")
            .to_string()
    };
    let mut device_label = if audio_ready {
        input.device_name().to_string()
    } else {
        released_label(&settings)
    };
    // Updates wait for a moment without a recording.
    let mut next_settings: Option<AppSettings> = None;
    let mut next_suspended: Option<bool> = None;
    // A press that arrived while the released microphone was opening;
    // recording starts at its first chunk.
    let mut start_pending = false;
    let mut captured_through = CaptureInstant::ZERO;
    let mut finishing: Option<Finishing> = None;
    // The recording skips processing (Dictate without processing).
    let mut recording_raw = false;
    let mut pending = 0_usize;
    // A local API client's dictation, and its transcripts as they finish.
    let mut service = ServiceDictations::default();
    let (service_results, service_finished) = mpsc::channel::<(u64, Result<String, String>)>();
    events.emit(&VoiceEvent::SessionStarted {
        timestamp_ms: now_ms(),
    })?;
    emit_state(&mut events, VoiceState::Listening, &device_label)?;
    println!(
        "Hex dictation is ready on {device_label}. Hold {hotkey_label} to dictate; Ctrl-C stops."
    );

    while !shutdown.load(Ordering::Relaxed) {
        // No microphone could be opened (none connected, or the default
        // went away): record why while it keeps retrying in the background.
        if let Some(error) = input.take_failure() {
            events.dictation(
                DictationPhase::Failed(format!("could not open the microphone: {error}")),
                "",
            )?;
        }
        for update in updates.try_iter() {
            match update {
                ListenerUpdate::Settings(next) => next_settings = Some(*next),
                ListenerUpdate::SuspendShortcuts(suspended) => next_suspended = Some(suspended),
            }
        }
        if !capture.is_recording() && !start_pending && finishing.is_none() && !service.is_active()
        {
            let mut register = next_suspended.take().is_some_and(|suspended| {
                std::mem::replace(&mut shortcuts_suspended, suspended) != suspended
            });
            if let Some(next) = next_settings.take() {
                register |= shortcuts(&next) != shortcuts(&settings);
                toggle = next.shortcut_mode == crate::dictation::ShortcutMode::Toggle;
                paste_with_shift.store(next.paste_with_shift, Ordering::Relaxed);
                if next.indicator_position != settings.indicator_position {
                    indicator = LinuxIndicator::new(next.indicator_position);
                }
                if next.recording_audio_behavior != settings.recording_audio_behavior {
                    suppressor = crate::linux::audio_control::AudioSuppressor::dictation(
                        next.recording_audio_behavior,
                    );
                }
                if device.is_none() && next.microphone != settings.microphone
                    || next.release_microphone_while_idle != release_while_idle
                {
                    drop(input);
                    input = microphone_input(&next, device);
                    release_while_idle = next.release_microphone_while_idle;
                    audio_ready = input.is_open();
                    if audio_ready {
                        capture = DictationCapture::new(input.sample_rate());
                        device_label = input.device_name().to_string();
                    } else {
                        device_label = released_label(&next);
                    }
                    emit_state(&mut events, active_state(false, pending), &device_label)?;
                }
                settings = next;
            }
            if register {
                // The old registration goes first: X11 grabs the same keys again.
                drop(hotkey.take());
                if !shortcuts_suspended {
                    hotkey = Some(start_shortcuts(shortcuts(&settings))?);
                }
            }
        }
        if let Some(hotkey) = &hotkey
            && let Ok(error) = hotkey.errors.try_recv()
        {
            return Err(eyre!("Linux hotkey monitor stopped: {error}"));
        }
        // A release keeps recording through its tail; shortcut events wait,
        // so a press inside the tail starts after this clip is submitted.
        if let Some(finish) = &finishing
            && let Some(tail) = finish.release.tail(&capture, Instant::now())
        {
            tracing::debug!(
                waited_ms = finish.released.elapsed().as_millis(),
                tail_ms = tail.as_millis(),
                "post-release tail captured"
            );
            let Finishing {
                release,
                print,
                raw,
                ..
            } = finishing.take().expect("a release is finishing");
            submit_capture(
                &mut capture,
                release.ended_at(),
                tail,
                &engine,
                &jobs,
                &mut events,
                &mut pending,
                print,
                raw,
            )?;
            suppressor.restore();
            if release_while_idle {
                input.close();
                audio_ready = false;
            }
            emit_state(
                &mut events,
                active_state(capture.is_recording(), pending),
                &device_label,
            )?;
        }
        // `/dictations`: a client's dictation never ends a shortcut capture,
        // and only its owner (or its lapsed lease) ends it.
        let now = Instant::now();
        let mut service_ended = service.expire(now);
        for control in dictations.drain() {
            let busy = capture.is_recording() || start_pending || finishing.is_some();
            match service.control(control, busy, now) {
                Effect::None => {}
                Effect::Start if audio_ready => {
                    start_service_capture(
                        &mut capture,
                        &mut service,
                        &suppressor,
                        captured_through,
                        input.sample_rate(),
                    );
                    service_ended |= !service.is_active();
                }
                // Recording starts at the microphone's first chunk.
                Effect::Start => {
                    if release_while_idle && !input.is_active() {
                        input.request_open();
                    }
                }
                Effect::Finish(id) => {
                    // The client's finish is its exact end: no tail.
                    match capture.finish_with_tail(captured_through, Duration::ZERO) {
                        Finish::Discard => service.complete(id, Err("capture-discarded".into())),
                        Finish::Transcribe(mut clip) => {
                            service.transcribing(id, clip.duration_ms());
                            match engine
                                .finish(clip.take_stream(), clip.into_transcription_samples())
                            {
                                Ok(transcript) => {
                                    let results = service_results.clone();
                                    thread::spawn(move || {
                                        let result =
                                            transcript.wait().map(|transcribed| transcribed.text);
                                        let _ = results.send((id, result));
                                    });
                                }
                                Err(error) => service.complete(id, Err(format!("{error:#}"))),
                            }
                        }
                    }
                    service_ended = true;
                }
                Effect::Cancel => service_ended = true,
            }
        }
        if service_ended && !service.is_active() {
            capture.cancel();
            suppressor.restore();
            if release_while_idle {
                input.close();
                audio_ready = false;
            }
        }
        for (id, result) in service_finished.try_iter() {
            service.complete(id, result);
        }
        while finishing.is_none()
            && let Some(action) = hotkey
                .as_ref()
                .and_then(|hotkey| hotkey.events.try_recv().ok())
        {
            // A printing stop finishes as a stop does; its reply follows.
            let (action, mut print) = match action {
                HotkeyEvent::FinishPrinting(reply) => (HotkeyEvent::Finish, Some(reply)),
                action => (action, None),
            };
            match action {
                // A client's dictation records: the shortcut and `hex
                // dictation` neither start nor end anything.
                HotkeyEvent::Start { .. } | HotkeyEvent::Finish | HotkeyEvent::Cancel
                    if service.is_active() =>
                {
                    if let Some(print) = print.take() {
                        print.send(PrintedTranscript::NotRecording);
                    }
                }
                HotkeyEvent::Start { raw } if !capture.is_recording() && !start_pending => {
                    recording_raw = raw;
                    if audio_ready {
                        start_capture(&mut capture, captured_through, || {
                            feedback::play_provisional_start();
                        });
                        stream_when_due(&mut capture, &engine, toggle);
                    } else if release_while_idle {
                        start_pending = true;
                        input.request_open();
                        feedback::prepare();
                    } else {
                        continue;
                    }
                    events.dictation(DictationPhase::Started, "")?;
                    emit_state(&mut events, VoiceState::Dictating, &device_label)?;
                }
                HotkeyEvent::Finish | HotkeyEvent::Cancel if start_pending => {
                    // Released before the opening microphone produced audio:
                    // nothing was captured.
                    start_pending = false;
                    if let Some(print) = print.take() {
                        print.send(PrintedTranscript::NoSpeech);
                    }
                    suppressor.restore();
                    input.close();
                    audio_ready = false;
                    events.dictation(
                        if action == HotkeyEvent::Cancel {
                            DictationPhase::Cancelled
                        } else {
                            DictationPhase::Discarded
                        },
                        "",
                    )?;
                    emit_state(&mut events, active_state(false, pending), &device_label)?;
                }
                HotkeyEvent::Finish if capture.is_recording() => {
                    finishing = Some(Finishing {
                        raw: recording_raw,
                        ..begin_finishing(&capture, captured_through, print.take(), || {
                            suppressor.restore_then(|| feedback::play(Tone::DictationStop))
                        })
                    });
                }
                HotkeyEvent::PasteLast if !capture.is_recording() => {
                    match jobs.try_send(OutputJob::PasteLast) {
                        Ok(()) | Err(TrySendError::Full(_)) => {}
                        Err(TrySendError::Disconnected(_)) => {
                            return Err(eyre!("dictation worker is unavailable"));
                        }
                    }
                }
                HotkeyEvent::Cancel if capture.is_recording() => {
                    capture.cancel();
                    feedback::retract_start();
                    feedback::play(Tone::Cancel);
                    suppressor.restore();
                    if release_while_idle {
                        input.close();
                        audio_ready = false;
                    }
                    events.dictation(DictationPhase::Cancelled, "")?;
                    emit_state(
                        &mut events,
                        active_state(capture.is_recording(), pending),
                        &device_label,
                    )?;
                }
                _ => {}
            }
        }
        while let Ok((kind, result)) = results.try_recv() {
            let phase = if kind == OutputKind::Printed {
                DictationPhase::Printed
            } else {
                DictationPhase::Pasted
            };
            // A repeated paste has no transcript and is not pending work.
            if kind == OutputKind::PasteLast {
                if let Err(error) = result {
                    fail(&events, error)?;
                }
                continue;
            }
            pending = pending.saturating_sub(1);
            indicator.finished(result.is_ok());
            match result {
                Ok(text) => {
                    feedback::play(Tone::DictationDone);
                    events.emit(&VoiceEvent::Transcript {
                        timestamp_ms: now_ms(),
                        phase: TranscriptPhase::Completed,
                        latency_ms: 0,
                        text: text.clone(),
                    })?;
                    events.dictation(phase, text)?;
                }
                Err(error) => fail(&events, error)?,
            }
            emit_state(
                &mut events,
                active_state(capture.is_recording(), pending),
                &device_label,
            )?;
        }

        indicator.update(capture.is_recording() || start_pending, pending);
        match input.recv_timeout(
            UPDATE_INTERVAL,
            crate::audio::capture_idle(
                capture.is_recording(),
                start_pending || service.is_starting(),
                finishing.is_some(),
            ),
        ) {
            RecoveringAudioInputEvent::Chunk {
                samples,
                captured_through: chunk_captured_through,
            } => {
                captured_through = chunk_captured_through;
                if !audio_ready {
                    capture = DictationCapture::new(input.sample_rate());
                    audio_ready = true;
                    device_label = input.device_name().to_string();
                    if start_pending {
                        // The on-demand open finished while the shortcut is
                        // held: recording starts here, without pre-roll.
                        start_pending = false;
                        start_capture(&mut capture, captured_through, || {
                            feedback::play_provisional_start();
                        });
                    }
                }
                if service.is_starting() && !capture.is_recording() {
                    start_service_capture(
                        &mut capture,
                        &mut service,
                        &suppressor,
                        captured_through,
                        input.sample_rate(),
                    );
                }
                crate::audio::input_level::record(&samples);
                capture.ingest(&samples, captured_through);
                if capture.is_recording() {
                    indicator.meter(&samples);
                    service.audio(&samples, Instant::now());
                }
                if capture.become_intentional(captured_through) && finishing.is_none() {
                    // Ordinary shortcut chords and short taps never mute or pause.
                    suppressor.suppress();
                }
                stream_when_due(&mut capture, &engine, toggle);
                capture.report_live_text();
            }
            RecoveringAudioInputEvent::Timeout => {}
            RecoveringAudioInputEvent::Interrupted => {
                // A release still in its tail finishes with the audio it has.
                if let Some(finish) = &mut finishing {
                    finish.release.stop_waiting();
                }
                audio_ready = false;
                if service.is_active() {
                    capture.cancel();
                    suppressor.restore();
                    service.lost(MICROPHONE_LOST);
                } else if capture.is_recording() && finishing.is_none() {
                    capture.cancel();
                    feedback::retract_start();
                    suppressor.restore();
                    events.dictation(DictationPhase::Cancelled, "")?;
                }
                events.dictation(
                    DictationPhase::Failed("microphone stream interrupted; reopening".into()),
                    "",
                )?;
                emit_state(
                    &mut events,
                    active_state(capture.is_recording(), pending),
                    &device_label,
                )?;
            }
            RecoveringAudioInputEvent::Reopened => {
                audio_ready = false;
                device_label = input.device_name().to_string();
                // A press waiting on the released microphone is already
                // Dictating; its first chunk starts the capture.
                emit_state(
                    &mut events,
                    active_state(capture.is_recording() || start_pending, pending),
                    &device_label,
                )?;
            }
            RecoveringAudioInputEvent::OpenFailed(error) => {
                if !release_while_idle {
                    return Err(eyre!("could not open the microphone: {error}"));
                }
                // An on-demand open failed (device busy or unplugged): report
                // it and stay released rather than stopping the listener.
                start_pending = false;
                service.lost(MICROPHONE_LOST);
                suppressor.restore();
                fail(&events, format!("could not open microphone: {error}"))?;
                emit_state(&mut events, active_state(false, pending), &device_label)?;
            }
        }
    }

    if capture.is_recording() {
        capture.cancel();
        events.dictation(DictationPhase::Cancelled, "")?;
    }
    indicator.update(false, pending);
    suppressor.restore();
    emit_state(&mut events, VoiceState::Stopping, &device_label)?;
    output.stop.store(true, Ordering::Release);
    drop(hotkey);
    drop(input);
    drop(jobs);
    if output
        .worker
        .take()
        .expect("dictation output worker exists")
        .join()
        .is_err()
    {
        return Err(eyre!("Linux dictation worker panicked"));
    }
    println!("Stopped.");
    Ok(())
}

/// Applies the saved sound settings and preloads the tones.
pub(crate) fn initialize_feedback(settings: &crate::app_settings::AppSettings) {
    feedback::set_enabled(settings.sound_effects);
    feedback::set_volume(settings.sound_effect_volume.clamp(0.0, 1.0));
    feedback::set_sound_set(settings.sound_theme);
    if let Err(error) = feedback::preload() {
        tracing::warn!(%error, "recording sounds are unavailable; continuing without feedback");
    }
}

fn start_capture(capture: &mut DictationCapture, started_at: CaptureInstant, play: impl FnOnce()) {
    capture.start(started_at);
    play();
}

/// Starts a client's dictation, which counts at once (no hold threshold),
/// and answers its start. A client that already gave up gets no recording.
fn start_service_capture(
    capture: &mut DictationCapture,
    service: &mut ServiceDictations,
    suppressor: &crate::linux::audio_control::AudioSuppressor,
    started_at: CaptureInstant,
    sample_rate: u32,
) {
    capture.start_programmatic_at(started_at);
    if service.started(sample_rate) {
        suppressor.suppress();
    } else {
        capture.cancel();
    }
}

/// Streams once the hold counts, so short taps and chords never reach the
/// model; a Toggle press is deliberate and streams at once.
fn stream_when_due(capture: &mut DictationCapture, engine: &TranscriptionEngine, toggle: bool) {
    if capture.is_recording() && (toggle || capture.is_intentional()) {
        capture.begin_stream(engine);
    }
}

/// Starts collecting a release's tail. A release that will be transcribed
/// is announced at once rather than after its tail; a short tap is discarded
/// on its release alone, needs no tail, and stays silent.
///
/// The release restores any muted output before playing the stop tone so it is
/// heard while the tail is still captured. The silence gate ignores the tail,
/// so the tone itself never makes a silent clip transcribe.
fn begin_finishing(
    capture: &DictationCapture,
    ended_at: CaptureInstant,
    print: Option<PrintReply>,
    announce: impl FnOnce(),
) -> Finishing {
    let release = ReleaseWait::new(capture, ended_at);
    if release.waits() {
        announce();
    }
    Finishing {
        release,
        print,
        raw: false,
        released: Instant::now(),
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_capture(
    capture: &mut DictationCapture,
    ended_at: CaptureInstant,
    tail: Duration,
    engine: &TranscriptionEngine,
    jobs: &SyncSender<OutputJob>,
    events: &mut EventLog,
    pending: &mut usize,
    print: Option<PrintReply>,
    raw: bool,
) -> Result<()> {
    let no_speech = |print: Option<PrintReply>| {
        if let Some(print) = print {
            print.send(PrintedTranscript::NoSpeech);
        }
    };
    match capture.finish_with_tail(ended_at, tail) {
        Finish::Discard => {
            // A short tap or chord withdraws the start tone of its press.
            feedback::retract_start();
            no_speech(print);
            events.dictation(DictationPhase::Discarded, "")?
        }
        Finish::Transcribe(clip) if clip.is_near_silent() => {
            no_speech(print);
            events.dictation(DictationPhase::Discarded, "")?
        }
        Finish::Transcribe(mut clip) => {
            let audio_ms = clip.duration_ms();
            let stream = clip.take_stream();
            let samples = clip.into_transcription_samples();
            let job = OutputJob::Dictation(Job {
                transcript: engine.finish(stream, samples)?,
                audio_ms,
                print,
                raw,
            });
            match jobs.try_send(job) {
                Ok(()) => {
                    *pending += 1;
                    events.dictation(DictationPhase::Transcribing, "")?;
                }
                Err(TrySendError::Full(_)) => fail(events, "dictation queue is full")?,
                Err(TrySendError::Disconnected(_)) => {
                    return Err(eyre!("dictation worker is unavailable"));
                }
            }
        }
    }
    Ok(())
}

fn active_state(recording: bool, pending: usize) -> VoiceState {
    if recording {
        VoiceState::Dictating
    } else if pending > 0 {
        VoiceState::Transcribing
    } else {
        VoiceState::Listening
    }
}

/// Logs a failed dictation or paste with the sound set's error sound.
fn fail(events: &EventLog, error: impl Into<String>) -> Result<()> {
    feedback::play(Tone::Error);
    events.dictation(DictationPhase::Failed(error.into()), "")?;
    Ok(())
}

fn emit_state(events: &mut EventLog, state: VoiceState, device: &str) -> Result<()> {
    events.emit(&VoiceEvent::State {
        timestamp_ms: now_ms(),
        state,
        device: device.into(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_time() -> CaptureInstant {
        CaptureInstant::from_nanos(60_000_000_000)
    }

    #[test]
    fn a_client_dictation_counts_at_once_and_an_abandoned_one_records_nothing() {
        let suppressor = crate::linux::audio_control::AudioSuppressor::for_test;
        let start = |owner: &mut ServiceDictations| {
            let (levels, _) = mpsc::sync_channel(1);
            let (reply, answer) = mpsc::sync_channel(1);
            assert_eq!(
                owner.control(
                    DictationControl::Start {
                        source: "test".into(),
                        owner_token: "hex_a".into(),
                        levels,
                        reply,
                    },
                    false,
                    Instant::now(),
                ),
                Effect::Start
            );
            answer
        };

        let mut owner = ServiceDictations::default();
        let mut capture = DictationCapture::new(16_000);
        let answer = start(&mut owner);
        let started = capture_time();
        start_service_capture(&mut capture, &mut owner, &suppressor(), started, 16_000);
        assert_eq!(answer.try_recv().unwrap().unwrap().sample_rate, 16_000);
        // No hold threshold: a finish 100 ms in still transcribes.
        let through = started + Duration::from_millis(100);
        capture.ingest(&[0.1; 1_600], through);
        assert!(matches!(
            capture.finish_with_tail(through, Duration::ZERO),
            Finish::Transcribe(_)
        ));

        let mut owner = ServiceDictations::default();
        drop(start(&mut owner));
        start_service_capture(&mut capture, &mut owner, &suppressor(), started, 16_000);
        assert!(!capture.is_recording());
        assert!(!owner.is_active());
    }

    #[test]
    fn start_sound_precedes_audio_and_does_not_change_short_tap_discard() {
        let mut capture = DictationCapture::new(16_000);
        let started_at = capture_time();
        let mut sounds = Vec::new();

        start_capture(&mut capture, started_at, || {
            sounds.push(Tone::DictationStart)
        });

        assert!(capture.is_recording());
        assert_eq!(sounds, [Tone::DictationStart]);
        assert!(matches!(
            capture.finish(started_at + Duration::from_millis(100)),
            Finish::Discard
        ));
    }

    #[test]
    fn cancelled_capture_keeps_pending_output_visible() {
        assert_eq!(active_state(true, 2), VoiceState::Dictating);
        assert_eq!(active_state(false, 2), VoiceState::Transcribing);
        assert_eq!(active_state(false, 1), VoiceState::Transcribing);
        assert_eq!(active_state(false, 0), VoiceState::Listening);
        assert_eq!(active_state(true, 0), VoiceState::Dictating);
    }

    #[test]
    fn output_guard_cancels_and_joins_even_on_listener_error() {
        let stopped = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let result: Result<()> = {
            let mut output = OutputWorker {
                stop: stopped.clone(),
                worker: None,
            };
            let (_jobs, receiver) = mpsc::channel::<()>();
            let stop = stopped.clone();
            let completed = finished.clone();
            output.worker = Some(thread::spawn(move || {
                // The sender must close before the guard joins this worker.
                assert!(receiver.recv().is_err());
                while !stop.load(Ordering::Acquire) {
                    thread::yield_now();
                }
                completed.store(true, Ordering::Release);
            }));
            Err(eyre!("listener failure"))
        };
        assert!(result.is_err());
        assert!(stopped.load(Ordering::Acquire));
        assert!(finished.load(Ordering::Acquire));
    }
}
