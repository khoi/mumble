#[cfg(target_os = "macos")]
pub(crate) mod audio;
#[cfg(target_os = "macos")]
pub(crate) mod config;
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub(crate) mod context;
#[cfg(target_os = "macos")]
pub(crate) mod diagnostics;
pub(crate) mod filler_words;
pub(crate) mod indicator;
pub(crate) mod live_text;
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) mod modes;
#[cfg_attr(any(target_os = "linux", target_os = "windows"), allow(dead_code))]
pub(crate) mod processor;
pub(crate) mod service;
pub(crate) mod speech_frames;
pub(crate) mod spoken_formatting;
pub(crate) mod spoken_symbols;
pub(crate) mod text_replacements;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) mod vocabulary;

use std::collections::VecDeque;
use std::time::Duration;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler, WindowFunction};

use crate::audio::CaptureInstant;
use crate::transcription::engine::{LiveStream, Partial, TranscriptionEngine};

#[cfg(target_os = "macos")]
use crate::audio::recording_environment::{
    RecordingEnvironmentController, RecordingEnvironmentSession,
};

const TIMELINE_BUFFER_DURATION: Duration = Duration::from_secs(10);
const PRE_ROLL_DURATION: Duration = Duration::from_millis(450);
pub const MINIMUM_HOLD_DURATION: Duration = Duration::from_millis(300);
/// Audio kept after a shortcut release that finishes a capture, because a
/// release often lands while the last word is still sounding.
pub const POST_RELEASE_TAIL: Duration = Duration::from_millis(200);
/// The longest a release tail grows while speech carries on past
/// [`POST_RELEASE_TAIL`] (a word still being said as the shortcut goes up).
pub const MAX_RELEASE_TAIL: Duration = Duration::from_millis(600);
/// Speech frames in a row (20 ms each) that count as still talking: a key
/// click is far shorter.
const TAIL_SPEECH_FRAMES: usize = 5;
/// Quiet frames in a row that end a growing tail (160 ms).
const TAIL_QUIET_FRAMES: usize = 8;
/// Quiet frames a run of speech may skip over, as between syllables.
const TAIL_SPEECH_GAP_FRAMES: usize = 2;
/// How long after its own length the stop tone may still reach the
/// microphone (output and input latency).
const STOP_TONE_MARGIN: Duration = Duration::from_millis(60);
/// 20 ms frames of recent speech flags kept for the release tail.
const VOICE_HISTORY_FRAMES: usize = 150;
/// How far apart the two keys of a both-sides chord (both Options) may go
/// down and still count as one press. Karabiner-Elements' simultaneous
/// keys default to 50 ms, tuned against typing rollover on letter keys;
/// two thumbs on two modifiers don't roll during typing, so a little more
/// room costs no false presses and saves missed ones.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub const BOTH_SIDES_WINDOW: Duration = Duration::from_millis(80);
const INITIAL_RECORDING_CAPACITY: Duration = Duration::from_secs(10);
const PARAKEET_SAMPLE_RATE: u32 = 16_000;
/// A capture whose loudest 30 ms window stays below this level is treated as
/// empty rather than transcribed, because models hallucinate closing phrases
/// such as "thank you" on silence. Deliberately conservative: whispered speech
/// sits well above it, so only a quiet room's noise floor is discarded.
const SILENCE_THRESHOLD_DBFS: f32 = -50.0;
const SILENCE_WINDOW_SAMPLES: usize = 480;
/// Live audio is handed to a capture's stream in blocks of at least this many
/// 16 kHz samples (160 ms), well under the model's chunk, to keep per-feed
/// overhead low.
const STREAM_FEED_SAMPLES: usize = 2_560;

pub enum Finish {
    Discard,
    Transcribe(DictationClip),
}

/// Where a shortcut finish's tail stands, from the audio recorded so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseTail {
    /// More audio is needed. `growing` once speech has carried past
    /// [`POST_RELEASE_TAIL`], so the wait is bounded by
    /// [`MAX_RELEASE_TAIL`] rather than the base tail.
    Waiting { growing: bool },
    /// Finish keeping this much audio after the release.
    Through(Duration),
}

impl ReleaseTail {
    /// The longest tail a wait in this state may still end with.
    pub fn bound(self) -> Duration {
        match self {
            Self::Waiting { growing: true } => MAX_RELEASE_TAIL,
            Self::Waiting { growing: false } => POST_RELEASE_TAIL,
            Self::Through(tail) => tail,
        }
    }
}

/// A shortcut release waiting for its tail, for capture loops that poll
/// between microphone chunks (Linux and Windows; the macOS audio owner
/// drains its tail directly).
#[cfg(not(target_os = "macos"))]
pub struct ReleaseWait {
    ended_at: CaptureInstant,
    started: std::time::Instant,
    tone: Duration,
    waits: bool,
}

#[cfg(not(target_os = "macos"))]
impl ReleaseWait {
    /// How long past a tail itself the loop waits for its audio.
    const GRACE: Duration = Duration::from_millis(100);

    /// A release at `ended_at`. Only one that will transcribe waits; its
    /// stop tone (sounding from now) is never taken for speech.
    pub fn new(capture: &DictationCapture, ended_at: CaptureInstant) -> Self {
        let waits = capture.would_transcribe(ended_at);
        Self {
            ended_at,
            started: std::time::Instant::now(),
            tone: if waits {
                crate::feedback::stop_tone_length()
            } else {
                Duration::ZERO
            },
            waits,
        }
    }

    pub fn ended_at(&self) -> CaptureInstant {
        self.ended_at
    }

    pub fn waits(&self) -> bool {
        self.waits
    }

    /// Finishes with the audio already captured, as after a lost microphone.
    pub fn stop_waiting(&mut self) {
        self.waits = false;
    }

    /// The tail to finish with once it is decided, or once the wait for it
    /// is over (the base tail plus the grace, or the longest tail plus the
    /// grace while speech grows it); `None` while it is still coming.
    pub fn tail(&self, capture: &DictationCapture, now: std::time::Instant) -> Option<Duration> {
        if !self.waits {
            return Some(POST_RELEASE_TAIL);
        }
        match capture.release_tail(self.ended_at, self.tone) {
            ReleaseTail::Through(tail) => Some(tail),
            waiting if now >= self.started + waiting.bound() + Self::GRACE => Some(waiting.bound()),
            ReleaseTail::Waiting { .. } => None,
        }
    }
}

/// Speech flags of a recording's 16 kHz audio, one per 20 ms frame, against
/// the pause detector's adaptive noise floor. Only recent frames are kept.
#[derive(Default)]
struct VoiceTrack {
    speech: crate::dictation::speech_frames::SpeechFrames,
    frame: Vec<f32>,
    /// Clip samples classified or waiting in `frame`.
    consumed: usize,
    flags: VecDeque<bool>,
    /// The frame index of `flags[0]`.
    first: usize,
}

impl VoiceTrack {
    const FRAME: usize = crate::dictation::speech_frames::FRAME_SAMPLES;

    fn extend(&mut self, mut samples: &[f32]) {
        self.consumed += samples.len();
        while !samples.is_empty() {
            let take = samples.len().min(Self::FRAME - self.frame.len());
            self.frame.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.frame.len() == Self::FRAME {
                self.flags.push_back(self.speech.is_speech(&self.frame));
                self.frame.clear();
                if self.flags.len() > VOICE_HISTORY_FRAMES {
                    self.flags.pop_front();
                    self.first += 1;
                }
            }
        }
    }

    /// Frames classified so far.
    fn frames(&self) -> usize {
        self.first + self.flags.len()
    }

    fn is_speech(&self, frame: usize) -> bool {
        frame
            .checked_sub(self.first)
            .and_then(|index| self.flags.get(index))
            .copied()
            .unwrap_or(false)
    }

    /// Whether speech runs up to frame `end` (exclusive): its last speech
    /// frame is within a syllable gap of `end`, and at least
    /// [`TAIL_SPEECH_FRAMES`] speech frames, none before `from`, lead up to
    /// it with no gap longer than a syllable's.
    fn talking_until(&self, end: usize, from: usize) -> bool {
        let mut speech = 0;
        let mut gap = 0;
        let mut frame = end;
        while frame > from.max(self.first) {
            frame -= 1;
            if self.is_speech(frame) {
                speech += 1;
                gap = 0;
                if speech >= TAIL_SPEECH_FRAMES {
                    return true;
                }
            } else {
                gap += 1;
                if gap > TAIL_SPEECH_GAP_FRAMES {
                    return false;
                }
            }
        }
        false
    }

    /// The first frame at or after `from`, before `end`, that ends
    /// [`TAIL_QUIET_FRAMES`] quiet frames in a row.
    fn quiet_end(&self, from: usize, end: usize) -> Option<usize> {
        let mut quiet = 0;
        for frame in from..end {
            if self.is_speech(frame) {
                quiet = 0;
            } else {
                quiet += 1;
                if quiet == TAIL_QUIET_FRAMES {
                    return Some(frame + 1);
                }
            }
        }
        None
    }
}

/// How the dictation shortcut controls a capture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShortcutMode {
    /// Hold the shortcut while speaking; release to finish.
    #[default]
    Hold,
    /// Press once to start and again to finish.
    Toggle,
}

/// Where a clip's audio came from, filled in by the audio owner so a
/// discarded clip's log line can tell a dead stream from a quiet room.
#[derive(Clone, Debug, Default)]
pub struct ClipSource {
    pub device: String,
    /// Since the stream last opened or reopened.
    pub stream_age: Option<Duration>,
    /// Since the last timeline gap or stream failure.
    pub since_discontinuity: Option<Duration>,
    /// Since the last overload CoreAudio reported without stopping the stream.
    pub since_glitch: Option<Duration>,
}

/// How the capture's audio lined up with its shortcut events.
#[derive(Clone, Copy, Debug, Default)]
struct ClipTimeline {
    source_rate: u32,
    /// How far the microphone timeline ran past the press when the capture
    /// started; negative when audio had not yet reached the press.
    press_lead_ms: Option<i64>,
    /// How far the recorded audio ran past the release when it finished.
    release_lead_ms: Option<i64>,
    /// The largest step between consecutive chunks, including steps below
    /// the gap that fails a capture.
    max_chunk_gap: Duration,
}

/// Evidence logged when a clip is discarded as near-silent.
struct SilenceEvidence {
    zero_fraction: f32,
    peak: f32,
    pre_press_dbfs: f32,
    post_press_dbfs: f32,
}

fn signed_ms(later: CaptureInstant, earlier: CaptureInstant) -> i64 {
    let nanos = i128::from(later.as_nanos()) - i128::from(earlier.as_nanos());
    (nanos / 1_000_000) as i64
}

pub struct DictationClip {
    samples: Vec<f32>,
    /// How many trailing samples were captured after the release. The stop
    /// tone sounds during them, so the silence gate ignores them.
    tail_samples: usize,
    /// How many leading samples were captured before the press.
    press_samples: usize,
    timeline: ClipTimeline,
    source: Option<Box<ClipSource>>,
    /// The live stream fed with this clip's audio during capture, if any.
    stream: Option<LiveStream>,
    /// Keeps background work deferring until the clip is submitted.
    #[cfg(target_os = "macos")]
    capture: Option<crate::audio::recording_environment::CaptureMarker>,
}

impl DictationClip {
    pub fn duration_ms(&self) -> u64 {
        self.samples.len() as u64 * 1_000 / u64::from(PARAKEET_SAMPLE_RATE)
    }

    /// Takes the clip's live stream; finish it with the complete clip.
    pub fn take_stream(&mut self) -> Option<LiveStream> {
        self.stream.take()
    }

    /// Takes the marker that defers background work; hold it until the clip
    /// is submitted.
    #[cfg(target_os = "macos")]
    pub fn take_capture_marker(
        &mut self,
    ) -> Option<crate::audio::recording_environment::CaptureMarker> {
        self.capture.take()
    }

    pub fn set_source(&mut self, source: ClipSource) {
        self.source = Some(Box::new(source));
    }

    #[cfg(test)]
    pub fn source(&self) -> Option<&ClipSource> {
        self.source.as_deref()
    }

    #[cfg(test)]
    pub fn for_test(samples: Vec<f32>) -> Self {
        Self {
            samples,
            tail_samples: 0,
            press_samples: 0,
            timeline: ClipTimeline::default(),
            source: None,
            stream: None,
            #[cfg(target_os = "macos")]
            capture: None,
        }
    }

    /// The captured audio through the release (before the tail and any
    /// model padding).
    fn released(&self) -> &[f32] {
        &self.samples[..self.samples.len().saturating_sub(self.tail_samples)]
    }

    /// The loudest 30 ms window's DC-removed RMS level in dBFS, over the
    /// captured audio through the release.
    fn loudest_window_dbfs(&self) -> f32 {
        loudest_window_dbfs(self.released())
    }

    fn silence_evidence(&self) -> SilenceEvidence {
        let released = self.released();
        let (pre_press, post_press) = released.split_at(self.press_samples.min(released.len()));
        let zeros = released.iter().filter(|&&sample| sample == 0.0).count();
        SilenceEvidence {
            zero_fraction: if released.is_empty() {
                1.0
            } else {
                zeros as f32 / released.len() as f32
            },
            peak: released
                .iter()
                .fold(0.0, |peak: f32, sample| peak.max(sample.abs())),
            pre_press_dbfs: loudest_window_dbfs(pre_press),
            post_press_dbfs: loudest_window_dbfs(post_press),
        }
    }

    /// Whether every sample through the release is exactly zero. A real
    /// microphone always carries some noise, so this is a stream still
    /// running but delivering digital silence (#1122), not a quiet room.
    #[cfg(target_os = "macos")]
    pub fn is_digital_silence(&self) -> bool {
        let released = self.released();
        !released.is_empty() && released.iter().all(|&sample| sample == 0.0)
    }

    /// Whether the capture's level stays below the silence threshold (a key
    /// click, the start tone, a noisy room, or a microphone turned down too
    /// far). Such a capture is discarded quietly, with no tone or note; the
    /// log keeps the measured level and peak against the threshold.
    pub fn is_near_silent(&self) -> bool {
        let level_dbfs = self.loudest_window_dbfs();
        if level_dbfs >= SILENCE_THRESHOLD_DBFS {
            return false;
        }
        let evidence = self.silence_evidence();
        let source = self.source.as_deref();
        let ms = |duration: Option<Duration>| duration.map(|duration| duration.as_millis() as u64);
        tracing::info!(
            level_dbfs,
            threshold_dbfs = SILENCE_THRESHOLD_DBFS,
            audio_ms = self.duration_ms(),
            pre_press_ms = self.press_samples as u64 * 1_000 / u64::from(PARAKEET_SAMPLE_RATE),
            pre_press_dbfs = evidence.pre_press_dbfs,
            post_press_dbfs = evidence.post_press_dbfs,
            zero_fraction = evidence.zero_fraction,
            peak = evidence.peak,
            device = source.map(|source| source.device.as_str()),
            sample_rate = self.timeline.source_rate,
            press_lead_ms = self.timeline.press_lead_ms,
            release_lead_ms = self.timeline.release_lead_ms,
            max_chunk_gap_ms = self.timeline.max_chunk_gap.as_millis() as u64,
            stream_age_ms = ms(source.and_then(|source| source.stream_age)),
            since_discontinuity_ms = ms(source.and_then(|source| source.since_discontinuity)),
            since_glitch_ms = ms(source.and_then(|source| source.since_glitch)),
            "discarded near-silent dictation without transcribing it"
        );
        true
    }

    pub fn into_transcription_samples(self) -> Vec<f32> {
        self.samples
    }
}

fn loudest_window_dbfs(samples: &[f32]) -> f32 {
    let loudest = samples
        .chunks(SILENCE_WINDOW_SAMPLES)
        .map(|window| {
            let count = window.len() as f64;
            let mean = window.iter().map(|&sample| f64::from(sample)).sum::<f64>() / count;
            let variance = window
                .iter()
                .map(|&sample| (f64::from(sample) - mean).powi(2))
                .sum::<f64>()
                / count;
            variance.sqrt()
        })
        .fold(0.0, f64::max);
    (20.0 * loudest.max(1e-6).log10()) as f32
}

/// Resamples a whole clip to the 16 kHz the cloud service takes, reusing the
/// capture's resampler. Only the Windows CLI's offline commands use it.
#[cfg(target_os = "windows")]
pub fn resample_for_parakeet(samples: &[f32], source_rate: u32) -> Vec<f32> {
    let mut recording = Recording::try_new(CaptureInstant::ZERO, source_rate)
        .expect("microphone sample rate must be resampleable");
    recording.push(samples);
    recording.finish(None)
}

pub struct DictationCapture {
    sample_rate: u32,
    ring: VecDeque<f32>,
    ring_captured_through: Option<CaptureInstant>,
    recording: Option<Recording>,
    #[cfg(target_os = "macos")]
    recording_environment: Option<RecordingEnvironmentController>,
}

struct Recording {
    started_at: CaptureInstant,
    intentional: bool,
    samples: Vec<f32>,
    source_samples: usize,
    source_rate: u32,
    recorded_through: Option<CaptureInstant>,
    /// How much of the recording precedes the press.
    pre_press: Duration,
    press_lead_ms: Option<i64>,
    max_chunk_gap: Duration,
    input_buffer: Vec<f32>,
    resampler: Option<Fft<f32>>,
    stream: Option<LiveStream>,
    voice: VoiceTrack,
    /// This recording's hold on the HUDs' live text, from its first text;
    /// dropping the recording clears it.
    live_text: Option<crate::dictation::live_text::Publisher>,
    #[cfg(target_os = "macos")]
    environment: RecordingEnvironmentState,
    #[cfg(target_os = "macos")]
    capture: Option<crate::audio::recording_environment::CaptureMarker>,
    /// Filters Hex Microphone from the hold threshold until the recording
    /// ends (macOS follows its recording environment instead).
    #[cfg(target_os = "linux")]
    voice_filter: Option<crate::audio::hex_microphone::Dictation>,
}

#[cfg(target_os = "macos")]
enum RecordingEnvironmentState {
    Disabled,
    Pending(RecordingEnvironmentController),
    Active {
        _session: RecordingEnvironmentSession,
    },
}

#[cfg(target_os = "macos")]
impl RecordingEnvironmentState {
    fn activate(&mut self) {
        if let Self::Pending(controller) = self {
            *self = Self::Active {
                _session: controller.begin(),
            };
        }
    }
}

impl Recording {
    const CHUNK_SIZE: usize = 1024;

    fn new(started_at: CaptureInstant, source_rate: u32) -> Self {
        Self::try_new(started_at, source_rate).expect("microphone sample rate must be resampleable")
    }

    fn try_new(started_at: CaptureInstant, source_rate: u32) -> Result<Self, String> {
        let resampler = (source_rate != PARAKEET_SAMPLE_RATE)
            .then(|| {
                Fft::new_custom(
                    source_rate as usize,
                    PARAKEET_SAMPLE_RATE as usize,
                    Self::CHUNK_SIZE,
                    1,
                    1,
                    WindowFunction::BlackmanHarris2,
                    FixedSync::Input,
                )
                .map_err(|error| error.to_string())
            })
            .transpose()?;
        Ok(Self {
            started_at,
            intentional: false,
            samples: Vec::with_capacity(samples_for(
                INITIAL_RECORDING_CAPACITY,
                PARAKEET_SAMPLE_RATE,
            )),
            source_samples: 0,
            source_rate,
            recorded_through: None,
            pre_press: Duration::ZERO,
            press_lead_ms: None,
            max_chunk_gap: Duration::ZERO,
            input_buffer: Vec::with_capacity(Self::CHUNK_SIZE),
            resampler,
            stream: None,
            voice: VoiceTrack::default(),
            live_text: None,
            #[cfg(target_os = "macos")]
            environment: RecordingEnvironmentState::Disabled,
            #[cfg(target_os = "macos")]
            capture: None,
            #[cfg(target_os = "linux")]
            voice_filter: None,
        })
    }

    fn push(&mut self, mut samples: &[f32]) {
        self.source_samples += samples.len();
        let Some(resampler) = &mut self.resampler else {
            self.samples.extend_from_slice(samples);
            return;
        };

        while !samples.is_empty() {
            let take = samples
                .len()
                .min(Self::CHUNK_SIZE - self.input_buffer.len());
            self.input_buffer.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.input_buffer.len() == Self::CHUNK_SIZE {
                self.samples
                    .extend(resample_chunk(resampler, &self.input_buffer, false));
                self.input_buffer.clear();
            }
        }
    }

    fn push_through(&mut self, samples: &[f32], captured_through: CaptureInstant) {
        if let Some(previous) = self.recorded_through {
            let captured_from = captured_through.sub_frames(samples.len(), self.source_rate);
            self.max_chunk_gap = self
                .max_chunk_gap
                .max(captured_from.duration_since(previous));
        }
        self.push(samples);
        self.track_voice();
        self.recorded_through = Some(captured_through);
    }

    /// Classifies the 16 kHz audio the voice track has not seen, in
    /// final-clip coordinates.
    fn track_voice(&mut self) {
        let delay = self.resampler.as_ref().map_or(0, Fft::output_delay);
        let recorded = &self.samples[delay.min(self.samples.len())..];
        let fresh = &recorded[self.voice.consumed.min(recorded.len())..];
        self.voice.extend(fresh);
    }

    /// The 20 ms clip frame `at` falls in.
    fn frame_at(&self, at: CaptureInstant) -> usize {
        let from_start = self.pre_press + at.duration_since(self.started_at);
        samples_for(from_start, PARAKEET_SAMPLE_RATE) / VoiceTrack::FRAME
    }

    /// How far past `ended_at` clip frame `frame` begins.
    fn after(&self, ended_at: CaptureInstant, frame: usize) -> Duration {
        let offset = Duration::from_secs_f64(
            (frame * VoiceTrack::FRAME) as f64 / f64::from(PARAKEET_SAMPLE_RATE),
        );
        offset.saturating_sub(self.pre_press + ended_at.duration_since(self.started_at))
    }

    /// Hands the stream every complete block of audio it has not seen, in
    /// final-clip coordinates (after the resampler delay).
    fn feed_stream(&mut self) {
        let delay = self.resampler.as_ref().map_or(0, Fft::output_delay);
        let Some(stream) = &mut self.stream else {
            return;
        };
        let recorded = &self.samples[delay.min(self.samples.len())..];
        let fresh = &recorded[stream.fed().min(recorded.len())..];
        if fresh.len() >= STREAM_FEED_SAMPLES {
            stream.feed(fresh);
        }
    }

    fn finish(mut self, ended_at: Option<CaptureInstant>) -> Vec<f32> {
        if let (Some(ended_at), Some(recorded_through)) = (ended_at, self.recorded_through)
            && let Some(post_release) = recorded_through.checked_duration_since(ended_at)
        {
            self.source_samples = self
                .source_samples
                .saturating_sub(samples_for(post_release, self.source_rate));
        }
        if let Some(resampler) = &mut self.resampler {
            let expected_samples =
                self.source_samples * PARAKEET_SAMPLE_RATE as usize / self.source_rate as usize;
            if !self.input_buffer.is_empty() {
                self.samples
                    .extend(resample_chunk(resampler, &self.input_buffer, true));
            }
            let output_delay = resampler.output_delay();
            for _ in 0..8 {
                if self.samples.len() >= expected_samples + output_delay {
                    break;
                }
                self.samples.extend(resample_chunk(resampler, &[], true));
            }
            self.samples.drain(..output_delay.min(self.samples.len()));
            self.samples.truncate(expected_samples);
        } else {
            self.samples.truncate(self.source_samples);
        }
        self.samples
    }
}

/// Resamples one mono chunk. A partial chunk is padded with silence, so an
/// empty partial chunk flushes the resampler's delay.
fn resample_chunk(resampler: &mut Fft<f32>, samples: &[f32], partial: bool) -> Vec<f32> {
    let input =
        InterleavedSlice::new(samples, 1, samples.len()).expect("mono chunk fits its slice");
    let indexing = partial.then(|| Indexing {
        partial_len: Some(samples.len()),
        ..Indexing::default()
    });
    resampler
        .process(&input, indexing.as_ref())
        .expect("fixed-rate resampling must succeed")
        .take_data()
}

impl DictationCapture {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            ring: VecDeque::with_capacity(samples_for(TIMELINE_BUFFER_DURATION, sample_rate)),
            ring_captured_through: None,
            recording: None,
            #[cfg(target_os = "macos")]
            recording_environment: None,
        }
    }

    #[cfg(target_os = "macos")]
    pub fn enable_recording_environment(&mut self, controller: RecordingEnvironmentController) {
        self.recording_environment = Some(controller);
    }

    #[cfg(test)]
    pub fn keep_warm(&mut self, samples: &[f32]) {
        self.keep_warm_for(samples, TIMELINE_BUFFER_DURATION);
    }

    fn keep_warm_for(&mut self, samples: &[f32], retention: Duration) {
        let capacity = samples_for(retention, self.sample_rate);
        if samples.len() >= capacity {
            self.ring.clear();
            self.ring.extend(&samples[samples.len() - capacity..]);
            return;
        }
        self.ring.drain(
            ..self
                .ring
                .len()
                .saturating_add(samples.len())
                .saturating_sub(capacity),
        );
        self.ring.extend(samples);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    #[cfg(any(test, target_os = "linux"))]
    pub fn start(&mut self, now: CaptureInstant) {
        self.start_with_pre_roll(now, PRE_ROLL_DURATION, false);
    }

    pub fn start_at(&mut self, occurred_at: CaptureInstant) {
        self.start_from_timeline(occurred_at, PRE_ROLL_DURATION, false)
    }

    pub fn start_programmatic_at(&mut self, occurred_at: CaptureInstant) {
        self.start_from_timeline(occurred_at, PRE_ROLL_DURATION, true)
    }

    fn start_from_timeline(
        &mut self,
        occurred_at: CaptureInstant,
        pre_roll: Duration,
        intentional: bool,
    ) {
        let delayed_audio = self
            .ring_captured_through
            .and_then(|through| through.checked_duration_since(occurred_at))
            .unwrap_or_default();
        self.start_with_pre_roll(
            occurred_at,
            pre_roll.saturating_add(delayed_audio),
            intentional,
        );
        if let (Some(recording), Some(through)) = (&mut self.recording, self.ring_captured_through)
        {
            recording.recorded_through = Some(through);
            recording.pre_press = recording.pre_press.saturating_sub(delayed_audio);
            recording.press_lead_ms = Some(signed_ms(through, occurred_at));
        }
    }

    fn start_with_pre_roll(&mut self, now: CaptureInstant, duration: Duration, intentional: bool) {
        let pre_roll = samples_for(duration, self.sample_rate).min(self.ring.len());
        let mut recording = Recording::new(now, self.sample_rate);
        recording.intentional = intentional;
        #[cfg(target_os = "linux")]
        {
            recording.voice_filter =
                intentional.then(crate::audio::hex_microphone::Dictation::begin);
        }
        #[cfg(target_os = "macos")]
        {
            recording.environment = match &self.recording_environment {
                Some(controller) if intentional => RecordingEnvironmentState::Active {
                    _session: controller.begin(),
                },
                Some(controller) => RecordingEnvironmentState::Pending(controller.clone()),
                None => RecordingEnvironmentState::Disabled,
            };
            recording.capture = self
                .recording_environment
                .is_some()
                .then(crate::audio::recording_environment::CaptureMarker::new);
        }
        recording.pre_press = crate::audio::duration_for_frames(pre_roll, self.sample_rate);
        let skip = self.ring.len() - pre_roll;
        let (front, back) = self.ring.as_slices();
        if skip < front.len() {
            recording.push(&front[skip..]);
            recording.push(back);
        } else {
            recording.push(&back[skip - front.len()..]);
        }
        recording.track_voice();
        self.recording = Some(recording);
    }

    #[cfg(test)]
    pub fn push(&mut self, samples: &[f32], now: CaptureInstant) -> bool {
        let became_intentional = self.become_intentional(now);
        if let Some(recording) = &mut self.recording {
            recording.push(samples);
        }
        became_intentional
    }

    #[cfg(any(test, target_os = "linux"))]
    pub fn ingest(&mut self, samples: &[f32], captured_through: CaptureInstant) {
        self.ingest_with_pending(samples, captured_through, None)
    }

    pub fn ingest_with_pending(
        &mut self,
        samples: &[f32],
        captured_through: CaptureInstant,
        oldest_pending: Option<CaptureInstant>,
    ) {
        if let Some(recording) = &mut self.recording {
            recording.push_through(samples, captured_through);
            recording.feed_stream();
        }
        let retention = oldest_pending
            .map(|pending| {
                captured_through
                    .duration_since(pending)
                    .saturating_add(PRE_ROLL_DURATION)
                    .max(TIMELINE_BUFFER_DURATION)
            })
            .unwrap_or(TIMELINE_BUFFER_DURATION);
        self.keep_warm_for(samples, retention);
        self.ring_captured_through = Some(captured_through);
    }

    pub fn become_intentional(&mut self, now: CaptureInstant) -> bool {
        let Some(recording) = &mut self.recording else {
            return false;
        };
        let became_intentional = !recording.intentional
            && now.duration_since(recording.started_at) >= MINIMUM_HOLD_DURATION;
        recording.intentional |= became_intentional;
        #[cfg(target_os = "macos")]
        if became_intentional {
            recording.environment.activate();
        }
        #[cfg(target_os = "linux")]
        if became_intentional {
            recording.voice_filter = Some(crate::audio::hex_microphone::Dictation::begin());
        }
        became_intentional
    }

    /// Counts the active recording as intentional now, whatever its length.
    pub fn mark_intentional(&mut self) -> bool {
        let Some(started_at) = self
            .recording
            .as_ref()
            .map(|recording| recording.started_at)
        else {
            return false;
        };
        self.become_intentional(started_at + MINIMUM_HOLD_DURATION)
    }

    /// Ends the active recording's environment (mute, paused media, idle-sleep
    /// prevention) before the capture finishes, and runs `then` once a muted
    /// output is audible again. A recording that never became intentional
    /// cannot start the environment afterwards.
    #[cfg(target_os = "macos")]
    pub fn end_recording_environment_then(&mut self, then: impl FnOnce() + Send + Sync + 'static) {
        let environment = self.recording.as_mut().map(|recording| {
            std::mem::replace(
                &mut recording.environment,
                RecordingEnvironmentState::Disabled,
            )
        });
        match environment {
            Some(RecordingEnvironmentState::Active { _session: session }) => session.end_then(then),
            _ => then(),
        }
    }

    /// The start of an active recording that has not yet passed the hold threshold.
    pub fn tentative_start(&self) -> Option<CaptureInstant> {
        self.recording
            .as_ref()
            .filter(|recording| {
                !recording.intentional
                    && recording.recorded_through.is_none_or(|through| {
                        through.duration_since(recording.started_at) < MINIMUM_HOLD_DURATION
                    })
            })
            .map(|recording| recording.started_at)
    }

    pub fn cancel(&mut self) {
        self.recording = None;
    }

    /// When the active recording started, without its pre-roll.
    pub fn started_at(&self) -> Option<CaptureInstant> {
        self.recording
            .as_ref()
            .map(|recording| recording.started_at)
    }

    /// 16 kHz audio recorded so far, from `start` in final-clip coordinates.
    /// The resampler's tail is only flushed by finishing, so the clip may end
    /// slightly later than what this has returned.
    #[cfg(test)]
    pub fn recorded_from(&self, start: usize) -> &[f32] {
        let Some(recording) = &self.recording else {
            return &[];
        };
        let delay = recording.resampler.as_ref().map_or(0, Fft::output_delay);
        let available = &recording.samples[delay.min(recording.samples.len())..];
        &available[start.min(available.len())..]
    }

    /// Whether the active recording has passed the hold threshold (or never
    /// needed one).
    pub fn is_intentional(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|recording| recording.intentional)
    }

    /// The active recording's text so far when its live stream published a
    /// change since the last call: only with live text on, and only for
    /// display. The finished clip's transcript replaces it.
    pub fn take_partial(&mut self) -> Option<Partial> {
        self.recording.as_mut()?.stream.as_mut()?.take_partial()
    }

    /// Shows the live text on the HUDs when it changed, and logs at debug
    /// how far it has got: word counts against the audio fed, never the
    /// words (the engine logs each stream's count of changes). Every OS's
    /// capture loop calls it after ingesting audio; it does nothing without
    /// live text.
    pub fn report_live_text(&mut self) {
        let Some(partial) = self.take_partial() else {
            return;
        };
        let Some(recording) = &mut self.recording else {
            return;
        };
        tracing::debug!(
            fed_ms = recording
                .stream
                .as_ref()
                .map_or(0, |stream| stream.fed() / 16),
            stable_words = partial.stable.split_whitespace().count(),
            tentative_words = partial.tentative.split_whitespace().count(),
            "live text"
        );
        recording
            .live_text
            .get_or_insert_with(crate::dictation::live_text::Publisher::new)
            .show(&partial);
    }

    /// Opens a live stream for the active recording when the engine's model
    /// can stream. The stream receives the recording's 16 kHz audio from its
    /// first sample (pre-roll included) as it arrives, travels with the
    /// finished clip, and is cancelled with the recording.
    pub fn begin_stream(&mut self, engine: &TranscriptionEngine) {
        let Some(recording) = &mut self.recording else {
            return;
        };
        if recording.stream.is_none() {
            recording.stream = engine.begin();
            recording.feed_stream();
        }
    }

    #[cfg(test)]
    pub fn finish(&mut self, now: CaptureInstant) -> Finish {
        self.finish_with_tail(now, Duration::ZERO)
    }

    /// Whether finishing at `ended_at` would produce a clip rather than a
    /// short-tap discard.
    pub fn would_transcribe(&self, ended_at: CaptureInstant) -> bool {
        self.recording.as_ref().is_some_and(|recording| {
            recording.intentional
                || ended_at.duration_since(recording.started_at) >= MINIMUM_HOLD_DURATION
        })
    }

    /// The tail a shortcut finish at `ended_at` keeps, judged on the audio
    /// recorded so far: [`POST_RELEASE_TAIL`], or, while speech carries on
    /// past it, until [`TAIL_QUIET_FRAMES`] of quiet or [`MAX_RELEASE_TAIL`].
    /// Only sustained speech grows it, never a key click, and never the
    /// stop tone of `tone` length that sounds from the release: speech must
    /// also be heard after the tone could have reached the microphone.
    pub fn release_tail(&self, ended_at: CaptureInstant, tone: Duration) -> ReleaseTail {
        let Some(recording) = &self.recording else {
            return ReleaseTail::Through(POST_RELEASE_TAIL);
        };
        let through = recording.recorded_through.unwrap_or(ended_at);
        if through < ended_at + POST_RELEASE_TAIL {
            return ReleaseTail::Waiting { growing: false };
        }
        let voice = &recording.voice;
        let heard = voice.frames();
        let capped = through >= ended_at + MAX_RELEASE_TAIL;
        let base = recording.frame_at(ended_at + POST_RELEASE_TAIL).min(heard);
        if !voice.talking_until(base, 0) {
            return ReleaseTail::Through(POST_RELEASE_TAIL);
        }
        // What runs into the base tail's end may be the tone; speech must
        // carry on once it is over.
        let after_tone = if tone.is_zero() {
            0
        } else {
            recording.frame_at(ended_at + tone + STOP_TONE_MARGIN)
        };
        let decision = base.max(after_tone + TAIL_SPEECH_FRAMES);
        if decision > recording.frame_at(ended_at + MAX_RELEASE_TAIL) {
            return ReleaseTail::Through(POST_RELEASE_TAIL);
        }
        if heard < decision {
            return if capped {
                ReleaseTail::Through(POST_RELEASE_TAIL)
            } else {
                ReleaseTail::Waiting { growing: true }
            };
        }
        if !voice.talking_until(decision, after_tone) {
            return ReleaseTail::Through(POST_RELEASE_TAIL);
        }
        let tail = |frame| {
            recording
                .after(ended_at, frame)
                .clamp(POST_RELEASE_TAIL, MAX_RELEASE_TAIL)
        };
        match voice.quiet_end(decision, heard) {
            Some(end) => ReleaseTail::Through(tail(end)),
            None if capped => ReleaseTail::Through(MAX_RELEASE_TAIL),
            None => ReleaseTail::Waiting { growing: true },
        }
    }

    /// Finishes at the exact `ended_at` event, keeping audio through
    /// `ended_at + tail`. The hold threshold is judged on `ended_at` alone.
    pub fn finish_with_tail(&mut self, ended_at: CaptureInstant, tail: Duration) -> Finish {
        if !self.would_transcribe(ended_at) {
            self.recording = None;
            return Finish::Discard;
        }
        let Some(mut recording) = self.recording.take() else {
            return Finish::Discard;
        };

        let stream = recording.stream.take();
        let kept_tail = recording
            .recorded_through
            .and_then(|through| through.checked_duration_since(ended_at))
            .map_or(Duration::ZERO, |after| after.min(tail));
        let timeline = ClipTimeline {
            source_rate: recording.source_rate,
            press_lead_ms: recording.press_lead_ms,
            release_lead_ms: recording
                .recorded_through
                .map(|through| signed_ms(through, ended_at)),
            max_chunk_gap: recording.max_chunk_gap,
        };
        let pre_press = recording.pre_press;
        #[cfg(target_os = "macos")]
        let capture = recording.capture.take();
        let samples = recording.finish(Some(ended_at + tail));
        Finish::Transcribe(DictationClip {
            tail_samples: samples_for(kept_tail, PARAKEET_SAMPLE_RATE).min(samples.len()),
            press_samples: samples_for(pre_press, PARAKEET_SAMPLE_RATE).min(samples.len()),
            timeline,
            source: None,
            samples,
            stream,
            #[cfg(target_os = "macos")]
            capture,
        })
    }
}

fn samples_for(duration: Duration, sample_rate: u32) -> usize {
    (duration.as_secs_f64() * f64::from(sample_rate)).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, samples: usize) -> impl Iterator<Item = f32> {
        (0..samples).map(move |index| {
            amplitude * (2.0 * std::f32::consts::PI * 220.0 * index as f32 / 16_000.0).sin()
        })
    }

    #[test]
    fn recorded_audio_is_a_prefix_of_the_finished_clip() {
        for rate in [16_000, 44_100, 48_000] {
            let mut capture = DictationCapture::new(rate);
            let origin = CaptureInstant::from_nanos(1_000_000_000);
            capture.start(origin);
            let source: Vec<f32> = (0..rate as usize * 2)
                .map(|index| (index as f32 * 0.021).sin() * 0.3)
                .collect();
            let mut streamed = Vec::new();
            for (index, block) in source.chunks(480).enumerate() {
                let through = origin + crate::audio::duration_for_frames((index + 1) * 480, rate);
                capture.ingest(block, through);
                let fresh = capture.recorded_from(streamed.len()).to_vec();
                streamed.extend(fresh);
            }
            let Finish::Transcribe(clip) = capture.finish(origin + Duration::from_secs(5)) else {
                panic!("a two-second hold transcribes");
            };
            let clip = clip.into_transcription_samples();
            assert!(streamed.len() <= clip.len(), "rate {rate}");
            assert!(clip.len() - streamed.len() < 1_024, "rate {rate}");
            assert_eq!(&clip[..streamed.len()], &streamed[..], "rate {rate}");
        }
    }

    #[test]
    fn silence_gate_discards_only_near_silent_captures() {
        let clip = |samples: Vec<f32>| DictationClip::for_test(samples);

        assert!(clip(vec![0.0; 16_000]).is_near_silent());
        assert!(clip(Vec::new()).is_near_silent());
        // A -66 dBFS noise floor and a constant DC offset carry no speech.
        assert!(clip(tone(0.000_7, 16_000).collect()).is_near_silent());
        assert!(clip(vec![0.2; 16_000]).is_near_silent());

        // 100 ms of whisper-level (-43 dBFS RMS) audio inside a second of
        // silence is kept.
        let mut quiet_speech = vec![0.0; 8_000];
        quiet_speech.extend(tone(0.01, 1_600));
        quiet_speech.extend(vec![0.0; 6_400]);
        let quiet_speech = clip(quiet_speech);
        assert!(!quiet_speech.is_near_silent());
        assert!((quiet_speech.loudest_window_dbfs() - -43.0).abs() < 0.5);
        assert!(!clip(tone(0.1, 16_000).collect()).is_near_silent());
    }

    #[test]
    fn silence_evidence_separates_the_pre_roll_from_audio_after_the_press() {
        let silent = DictationClip::for_test(vec![0.0; 16_000]).silence_evidence();
        assert_eq!(silent.zero_fraction, 1.0);
        assert_eq!(silent.peak, 0.0);

        // The press is handled 200 ms after it happened: the pre-roll still
        // covers 450 ms before it, and the timeline ran 200 ms past it.
        let origin = CaptureInstant::from_nanos(10_000_000_000);
        let press = origin.checked_sub(Duration::from_millis(200)).unwrap();
        let mut capture = DictationCapture::new(16_000);
        capture.ingest(&tone(0.1, 16_000).collect::<Vec<_>>(), origin);
        capture.start_at(press);
        capture.ingest(&[0.0; 8_000], origin + Duration::from_millis(500));
        // 30 ms of audio never arrived: below a capture discontinuity.
        capture.ingest(&[0.0; 8_000], origin + Duration::from_millis(1_030));
        let Finish::Transcribe(clip) = capture.finish(origin + Duration::from_millis(1_000)) else {
            panic!("a held capture transcribes");
        };

        assert_eq!(clip.press_samples, 7_200);
        assert_eq!(clip.timeline.press_lead_ms, Some(200));
        assert_eq!(clip.timeline.release_lead_ms, Some(30));
        assert_eq!(clip.timeline.max_chunk_gap, Duration::from_millis(30));
        let evidence = clip.silence_evidence();
        // A 0.1 sine is -23 dBFS RMS; the 200 ms after the press still carry it.
        assert!((evidence.pre_press_dbfs - -23.0).abs() < 0.5);
        assert!((evidence.post_press_dbfs - -23.0).abs() < 0.5);
        assert!(evidence.zero_fraction > 0.5);
        assert!((evidence.peak - 0.1).abs() < 1e-3);
    }

    fn capture_time() -> CaptureInstant {
        CaptureInstant::from_nanos(60_000_000_000)
    }

    #[test]
    fn quick_option_tap_discards() {
        let mut capture = DictationCapture::new(16_000);
        let start = capture_time();
        capture.start(start);
        let _ = capture.push(&vec![0.5; 1_600], start + Duration::from_millis(100));
        assert!(matches!(
            capture.finish(start + Duration::from_millis(100)),
            Finish::Discard
        ));
    }

    #[test]
    fn explicit_programmatic_capture_is_not_a_shortcut_tap() {
        let mut capture = DictationCapture::new(16_000);
        let start = capture_time();
        capture.start_programmatic_at(start);
        capture.ingest(&[0.5; 1_600], start + Duration::from_millis(100));

        let Finish::Transcribe(clip) = capture.finish(start + Duration::from_millis(100)) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 100);
    }

    #[test]
    fn modifier_capture_becomes_intentional_only_after_the_hold_threshold() {
        let mut capture = DictationCapture::new(16_000);
        let start = capture_time();
        capture.start(start);

        assert!(!capture.push(&[0.5; 80], start + Duration::from_millis(299)));
        assert!(capture.push(&[0.5; 80], start + MINIMUM_HOLD_DURATION));
        assert!(!capture.push(&[0.5; 80], start + Duration::from_millis(301)));
    }

    #[test]
    fn release_can_commit_the_intentional_transition_between_audio_chunks() {
        let mut capture = DictationCapture::new(16_000);
        let start = capture_time();
        capture.start(start);

        assert!(!capture.push(&[0.5; 80], start + Duration::from_millis(299)));
        assert!(capture.become_intentional(start + MINIMUM_HOLD_DURATION));
        assert!(!capture.become_intentional(start + Duration::from_millis(301)));
    }

    #[test]
    fn delayed_press_reconstructs_audio_from_the_original_boundary() {
        let mut capture = DictationCapture::new(16_000);
        let origin = capture_time();
        for index in 1..=16 {
            capture.ingest(
                &vec![index as f32; 1_600],
                origin + Duration::from_millis(index * 100),
            );
        }

        capture.start_at(origin + Duration::from_secs(1));
        for index in 17..=20 {
            capture.ingest(
                &vec![index as f32; 1_600],
                origin + Duration::from_millis(index * 100),
            );
        }

        let Finish::Transcribe(clip) = capture.finish(origin + Duration::from_millis(1_800)) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 1_250);
        assert_eq!(clip.samples.first(), Some(&6.0));
    }

    #[test]
    fn delayed_release_removes_audio_captured_after_the_physical_release() {
        let mut capture = DictationCapture::new(16_000);
        let origin = capture_time();
        capture.ingest(&vec![0.25; 16_000], origin);
        capture.start_at(origin);
        for index in 1..=12 {
            capture.ingest(
                &vec![index as f32; 1_600],
                origin + Duration::from_millis(index * 100),
            );
        }

        let Finish::Transcribe(clip) = capture.finish(origin + Duration::from_secs(1)) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 1_450);
        assert!(!clip.samples.contains(&11.0));
        assert!(!clip.samples.contains(&12.0));
    }

    #[test]
    fn release_tail_keeps_audio_through_the_tail_and_no_further() {
        let mut capture = DictationCapture::new(16_000);
        let origin = capture_time();
        capture.ingest(&vec![0.25; 16_000], origin);
        capture.start_at(origin);
        for index in 1..=14 {
            capture.ingest(
                &vec![index as f32; 1_600],
                origin + Duration::from_millis(index * 100),
            );
        }

        let Finish::Transcribe(clip) =
            capture.finish_with_tail(origin + Duration::from_secs(1), POST_RELEASE_TAIL)
        else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 1_650);
        assert_eq!(clip.samples.last(), Some(&12.0));
        assert!(!clip.samples.contains(&13.0));
        assert_eq!(clip.tail_samples, 3_200);
    }

    /// Records `sound(ms)` (milliseconds from the release, which is 1 s after
    /// the press) in 20 ms chunks and asks for the release tail after each
    /// one, as a capture loop does. Returns the tail it settled on, where the
    /// audio had reached when it did, and the finished clip.
    fn release_tail_for(
        sound: impl Fn(f64) -> f32,
        stop_tone: Duration,
    ) -> (Duration, Duration, DictationClip) {
        let mut capture = DictationCapture::new(16_000);
        let origin = capture_time();
        capture.ingest(&vec![0.0; 16_000], origin);
        capture.start_at(origin);
        let released = origin + Duration::from_secs(1);
        for chunk in 1..=100_u64 {
            let start = (chunk - 1) * 320;
            let samples = (start..start + 320)
                .map(|index| sound(index as f64 / 16.0 - 1_000.0))
                .collect::<Vec<_>>();
            let through = origin + Duration::from_millis(chunk * 20);
            capture.ingest(&samples, through);
            if through < released {
                continue;
            }
            match capture.release_tail(released, stop_tone) {
                ReleaseTail::Waiting { growing } => {
                    assert!(
                        growing || through < released + POST_RELEASE_TAIL,
                        "the base tail waits for its own audio only"
                    );
                }
                ReleaseTail::Through(tail) => {
                    let Finish::Transcribe(clip) = capture.finish_with_tail(released, tail) else {
                        panic!("expected transcription");
                    };
                    return (tail, through.duration_since(released), clip);
                }
            }
        }
        panic!("the tail never settled");
    }

    /// A 180 Hz voice at -13 dBFS between `from` and `to` ms.
    fn voice(from: f64, to: f64) -> impl Fn(f64) -> f32 {
        move |ms| {
            if (from..to).contains(&ms) {
                0.3 * (2.0 * std::f64::consts::PI * 180.0 * ms / 1_000.0).sin() as f32
            } else {
                0.0
            }
        }
    }

    #[test]
    fn a_word_said_through_the_release_keeps_the_tail_open_until_it_ends() {
        // #1101: the last word ran 350 ms past the release.
        let (tail, waited, clip) = release_tail_for(voice(-800.0, 350.0), Duration::ZERO);
        assert!(
            (Duration::from_millis(500)..=Duration::from_millis(540)).contains(&tail),
            "{tail:?}"
        );
        assert!(waited <= tail + Duration::from_millis(40), "{waited:?}");
        // The whole word is in the clip: 450 ms pre-roll, 1 s, then the tail.
        let word_end = (450 + 1_000 + 340) * 16;
        assert!(clip.samples.len() > word_end);
        assert!(
            clip.samples[word_end - 320..word_end]
                .iter()
                .any(|sample| sample.abs() > 0.1)
        );
    }

    #[test]
    fn a_release_after_the_speech_keeps_the_base_tail_without_waiting_longer() {
        let (tail, waited, clip) = release_tail_for(voice(-800.0, -60.0), Duration::ZERO);
        assert_eq!(tail, POST_RELEASE_TAIL);
        assert_eq!(waited, POST_RELEASE_TAIL);
        assert_eq!(clip.duration_ms(), 1_650);
    }

    #[test]
    fn speech_that_carries_on_stops_the_tail_at_its_cap() {
        let (tail, waited, clip) = release_tail_for(voice(-800.0, 5_000.0), Duration::ZERO);
        assert_eq!(tail, MAX_RELEASE_TAIL);
        assert_eq!(waited, MAX_RELEASE_TAIL);
        assert_eq!(clip.duration_ms(), 2_050);
    }

    #[test]
    fn the_stop_tone_or_a_key_click_never_grows_the_tail() {
        let speech = voice(-800.0, -60.0);
        // The wood stop sound (225 ms), loud and late through the output.
        let stop_tone = Duration::from_millis(225);
        let with_tone = |ms: f64| {
            let tone = if (30.0..255.0).contains(&ms) {
                let decay = 1.0 - (ms - 30.0) / 225.0;
                (0.4 * decay * (2.0 * std::f64::consts::PI * 880.0 * ms / 1_000.0).sin()) as f32
            } else {
                0.0
            };
            speech(ms) + tone
        };
        let (tail, _, _) = release_tail_for(with_tone, stop_tone);
        assert_eq!(tail, POST_RELEASE_TAIL);
        // A key click: 15 ms of noise right at the end of the base tail.
        let with_click = |ms: f64| {
            let click = if (185.0..200.0).contains(&ms) {
                if (ms * 16.0) as i64 % 2 == 0 {
                    0.8
                } else {
                    -0.8
                }
            } else {
                0.0
            };
            speech(ms) + click
        };
        let (tail, waited, _) = release_tail_for(with_click, stop_tone);
        assert_eq!(tail, POST_RELEASE_TAIL);
        assert_eq!(waited, POST_RELEASE_TAIL);
        // Speech still going once the tone is over does grow the tail.
        let (tail, _, _) =
            release_tail_for(|ms| with_tone(ms) + voice(150.0, 450.0)(ms), stop_tone);
        assert!(tail > Duration::from_millis(450), "{tail:?}");
    }

    #[test]
    fn the_stop_tone_in_the_tail_does_not_make_silence_transcribe() {
        let mut capture = DictationCapture::new(16_000);
        let origin = capture_time();
        capture.start_at(origin);
        let released = origin + Duration::from_secs(1);
        capture.ingest(&vec![0.0; 16_000], released);
        capture.ingest(
            &tone(0.1, 3_200).collect::<Vec<_>>(),
            released + POST_RELEASE_TAIL,
        );
        let Finish::Transcribe(clip) = capture.finish_with_tail(released, POST_RELEASE_TAIL) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 1_200);
        assert!(clip.is_near_silent());
    }

    #[test]
    fn short_tap_discards_on_its_physical_release_despite_the_tail() {
        let origin = capture_time();
        let mut capture = DictationCapture::new(16_000);
        capture.start_at(origin);
        capture.ingest(&vec![0.5; 16_000], origin + Duration::from_secs(1));
        // The tail would carry the boundary past the hold threshold.
        assert!(!capture.would_transcribe(origin + Duration::from_millis(299)));
        assert!(matches!(
            capture.finish_with_tail(origin + Duration::from_millis(299), POST_RELEASE_TAIL),
            Finish::Discard
        ));
        assert!(!capture.is_recording());
    }

    #[test]
    fn minimum_hold_uses_source_event_times_after_a_processing_stall() {
        let origin = capture_time();
        let mut short = DictationCapture::new(16_000);
        short.start_at(origin);
        short.ingest(&vec![0.5; 16_000], origin + Duration::from_secs(2));
        assert!(matches!(
            short.finish(origin + Duration::from_millis(299)),
            Finish::Discard
        ));

        let mut accepted = DictationCapture::new(16_000);
        accepted.start_at(origin);
        accepted.ingest(&vec![0.5; 16_000], origin + Duration::from_secs(2));
        assert!(matches!(
            accepted.finish(origin + MINIMUM_HOLD_DURATION),
            Finish::Transcribe(_)
        ));
    }

    #[test]
    fn pending_input_extends_history_for_an_arbitrary_coordinator_stall() {
        let mut capture = DictationCapture::new(16_000);
        let origin = capture_time();
        capture.ingest(&vec![0.25; 16_000], origin + Duration::from_secs(1));
        let pressed_at = origin + Duration::from_secs(1);
        for second in 2..=31 {
            capture.ingest_with_pending(
                &vec![second as f32; 16_000],
                origin + Duration::from_secs(second),
                Some(pressed_at),
            );
        }

        capture.start_at(pressed_at);
        capture.ingest(&vec![32.0; 16_000], origin + Duration::from_secs(32));
        assert_eq!(capture.ring.len(), 160_000);
        assert_eq!(capture.ring.front(), Some(&23.0));
        assert_eq!(capture.ring.back(), Some(&32.0));

        let Finish::Transcribe(clip) = capture.finish(origin + Duration::from_secs(31)) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 30_450);
        assert_eq!(clip.samples.first(), Some(&0.25));
    }

    #[test]
    fn warm_buffer_accepts_chunks_before_reaching_capacity() {
        let mut capture = DictationCapture::new(16_000);
        capture.keep_warm(&[0.25; 480]);
        capture.keep_warm(&[0.5; 480]);
        assert_eq!(capture.ring.len(), 960);
    }

    #[test]
    fn intentional_hold_includes_the_pre_roll() {
        let mut capture = DictationCapture::new(16_000);
        capture.keep_warm(&vec![0.25; 16_000]);
        let start = capture_time();
        capture.start(start);
        let _ = capture.push(&vec![0.5; 8_000], start + Duration::from_millis(500));
        let Finish::Transcribe(clip) = capture.finish(start + Duration::from_millis(500)) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 950);
        let samples = clip.into_transcription_samples();
        assert_eq!(samples[0], 0.25, "the pre-roll leads the clip");
        assert!(samples.contains(&0.5), "the held speech is kept");
    }

    #[test]
    fn capture_continues_past_sixty_seconds_until_explicitly_finished() {
        let mut capture = DictationCapture::new(16_000);
        let start = capture_time();
        capture.start(start);
        let _ = capture.push(&vec![0.5; 61 * 16_000], start + Duration::from_secs(61));

        let Finish::Transcribe(clip) = capture.finish(start + Duration::from_secs(61)) else {
            panic!("expected transcription")
        };
        assert_eq!(clip.duration_ms(), 61_000);
    }

    #[test]
    fn resampling_flushes_the_end_of_the_recording() {
        for (sample_rate, input_len) in [
            (44_100, 1_337),
            (44_100, 4_410),
            (48_000, 1_024),
            (48_000, 4_800),
        ] {
            let mut recording = Recording::new(capture_time(), sample_rate);
            recording.push(&vec![0.5; input_len]);
            let samples = recording.finish(None);
            let expected = input_len * PARAKEET_SAMPLE_RATE as usize / sample_rate as usize;

            assert_eq!(samples.len(), expected);
            assert!(samples.last().is_some_and(|sample| *sample > 0.25));
        }
    }
}
