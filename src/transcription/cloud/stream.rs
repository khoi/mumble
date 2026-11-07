//! A dictation streamed to the service while it is recorded: its own thread
//! owns the connection and sends the audio as it arrives, so on release only
//! the last words are left to transcribe.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{CloudError, CloudProvider, WAIT_POLL};
use crate::transcription::CancelToken;
use crate::transcription::engine::{Partial, Partials};

/// How long a provider whose service refused streaming goes straight to
/// whole-clip transcription: streaming is rolled out per account.
const REFUSAL_HOLD: Duration = Duration::from_secs(60 * 60);
/// Audio waiting for a slow connection, at most: 30 s. Past it the stream
/// is given up and the model transcribes the clip.
const MAX_QUEUED: usize = 30 * 16_000;
/// The longest the engine waits for a released stream's text. The stream's
/// own deadlines end it sooner.
const ANSWER_LIMIT: Duration = Duration::from_secs(45);

/// When each provider's service last refused a stream, by
/// [`CloudProvider::ALL`] order.
static REFUSED: Mutex<[Option<Instant>; 2]> = Mutex::new([None, None]);

/// What the capture sends the stream's thread.
enum Input {
    Audio(Vec<f32>),
    /// The recording ended; finish with what was sent.
    End,
}

/// The capture's audio as the stream's thread receives it.
pub(super) struct Audio {
    input: Receiver<Input>,
    /// Samples sent but not yet taken.
    queued: Arc<AtomicUsize>,
}

/// Everything the capture sent since the last look.
#[derive(Default)]
pub(super) struct Taken {
    pub(super) audio: Vec<f32>,
    pub(super) ended: bool,
}

impl Audio {
    /// A whole clip, already ended.
    pub(super) fn whole(clip: &[f32]) -> Self {
        let (sender, input) = mpsc::channel();
        let _ = sender.send(Input::Audio(clip.to_vec()));
        let _ = sender.send(Input::End);
        Self {
            input,
            queued: Arc::default(),
        }
    }

    /// Takes what was sent without waiting. A capture that let go of the
    /// stream without ending it was discarded, so the stream stops.
    pub(super) fn take(&self) -> Result<Taken, CloudError> {
        let mut taken = Taken::default();
        let result = loop {
            match self.input.try_recv() {
                Ok(Input::Audio(samples)) => taken.audio.extend(samples),
                Ok(Input::End) => {
                    taken.ended = true;
                    break Ok(());
                }
                Err(TryRecvError::Empty) => break Ok(()),
                Err(TryRecvError::Disconnected) => {
                    break Err(CloudError::Failed("the dictation was discarded".into()));
                }
            }
        };
        // Only `feed` adds to `queued`, so it never drops below what this
        // take removes; clamp anyway so a race can't wrap it.
        let taken_len = taken.audio.len().min(self.queued.load(Ordering::Acquire));
        self.queued.fetch_sub(taken_len, Ordering::AcqRel);
        result.map(|()| taken)
    }
}

/// The nonempty `segments`, trimmed and joined by spaces.
pub(super) fn joined<'a>(segments: impl IntoIterator<Item = &'a str>) -> String {
    segments
        .into_iter()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Shows `partial` on Live text, when it is on.
pub(super) fn publish(partials: Option<&Partials>, stable: String, tentative: String) {
    if let Some(partials) = partials {
        partials.publish(Partial { stable, tentative });
    }
}

/// What the engine does with a clip whose stream gave no text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Fallback {
    /// Send the whole clip to the service: it refused to stream, or the
    /// stream heard past the clip.
    WholeClip,
    /// The dictation fails with this reason; there is no local fallback.
    Failed(String),
}

pub(crate) struct CloudStream {
    provider: CloudProvider,
    /// `None` once the stream fell too far behind and was given up.
    input: Option<Sender<Input>>,
    queued: Arc<AtomicUsize>,
    answer: Receiver<Result<String, CloudError>>,
}

impl CloudStream {
    /// Connects to `provider` on its own thread and starts taking audio in
    /// `language` (the selection captured when the capture began), or `None`
    /// while its service refuses streams.
    pub(crate) fn open(
        provider: CloudProvider,
        language: String,
        partials: Option<Partials>,
    ) -> Option<Self> {
        if refused(provider) {
            return None;
        }
        let keyterms = crate::dictation::vocabulary::terms();
        Self::spawn(provider, move |audio| match provider {
            CloudProvider::Claude => {
                super::claude::stream(&audio, &language, &keyterms, partials.as_ref())
            }
            CloudProvider::Codex => super::codex::stream(&audio, &language, partials.as_ref()),
        })
    }

    fn spawn(
        provider: CloudProvider,
        run: impl FnOnce(Audio) -> Result<String, CloudError> + Send + 'static,
    ) -> Option<Self> {
        let (input, receiver) = mpsc::channel();
        let (reply, answer) = mpsc::channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let audio = Audio {
            input: receiver,
            queued: queued.clone(),
        };
        std::thread::Builder::new()
            .name("cloud-stream".into())
            .spawn(move || {
                let _ = reply.send(run(audio));
            })
            .map_err(|error| tracing::warn!(%error, "couldn't start a cloud stream"))
            .ok()?;
        Some(Self {
            provider,
            input: Some(input),
            queued,
            answer,
        })
    }

    /// Sends `samples` on, unless the connection has fallen [`MAX_QUEUED`]
    /// behind, which gives the stream up.
    pub(crate) fn feed(&mut self, samples: &[f32]) {
        let Some(input) = &self.input else {
            return;
        };
        if samples.is_empty() {
            return;
        }
        if self.queued.load(Ordering::Acquire) + samples.len() > MAX_QUEUED {
            tracing::warn!(
                provider = self.provider.name(),
                "the cloud stream fell 30 s behind; giving it up"
            );
            self.input = None;
            return;
        }
        self.queued.fetch_add(samples.len(), Ordering::AcqRel);
        let _ = input.send(Input::Audio(samples.to_vec()));
    }

    /// Releases the stream: sends the clip's unsent `tail` and the end at
    /// once, and leaves its answer to be waited for. A stream that heard
    /// past the clip (`overfed`) is dropped for the whole clip instead.
    pub(crate) fn end(mut self, tail: &[f32], overfed: bool) -> CloudAnswer {
        let provider = self.provider;
        let settled = |fallback| CloudAnswer {
            provider,
            state: Answer::Settled(fallback),
        };
        if overfed {
            tracing::warn!(
                provider = provider.name(),
                "the cloud stream heard past the clip; sending the whole clip"
            );
            return settled(Fallback::WholeClip);
        }
        self.feed(tail);
        match self.input.take() {
            Some(input) => {
                let _ = input.send(Input::End);
                CloudAnswer {
                    provider,
                    state: Answer::Waiting(self.answer),
                }
            }
            None => settled(Fallback::Failed(
                "the cloud stream fell behind and was given up".into(),
            )),
        }
    }
}

/// A released stream's text, still on its way.
pub(crate) struct CloudAnswer {
    provider: CloudProvider,
    state: Answer,
}

enum Answer {
    Waiting(Receiver<Result<String, CloudError>>),
    Settled(Fallback),
}

impl CloudAnswer {
    /// The text, waiting at most [`ANSWER_LIMIT`] or until `cancel`; without
    /// it, how the clip should be transcribed instead.
    pub(crate) fn wait(self, cancel: &CancelToken) -> Result<String, Fallback> {
        let answer = match self.state {
            Answer::Settled(fallback) => return Err(fallback),
            Answer::Waiting(answer) => answer,
        };
        let started = Instant::now();
        let result = loop {
            match answer.recv_timeout(WAIT_POLL) {
                Ok(result) => break result,
                Err(RecvTimeoutError::Timeout) if cancel.is_cancelled() => {
                    break Err(CloudError::Failed("the dictation was cancelled".into()));
                }
                Err(RecvTimeoutError::Timeout) if started.elapsed() >= ANSWER_LIMIT => {
                    break Err(CloudError::Failed("no answer in time".into()));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    break Err(CloudError::Failed("the stream stopped".into()));
                }
            }
        };
        let provider = self.provider;
        let finish_ms = started.elapsed().as_millis() as u64;
        match result.map(|text| text.trim().to_owned()) {
            Ok(text) if !text.is_empty() => {
                tracing::info!(
                    provider = provider.name(),
                    finish_ms,
                    "cloud stream transcribed"
                );
                Ok(text)
            }
            Ok(_) => Err(fall_back(provider, &CloudError::NoSpeech, finish_ms)),
            Err(error) => Err(fall_back(provider, &error, finish_ms)),
        }
    }
}

fn fall_back(provider: CloudProvider, error: &CloudError, finish_ms: u64) -> Fallback {
    let reason = error.reason(provider);
    let fallback = if matches!(error, CloudError::Refused(_)) {
        refuse(provider);
        Fallback::WholeClip
    } else {
        Fallback::Failed(reason.clone())
    };
    tracing::warn!(
        provider = provider.name(),
        finish_ms,
        %reason,
        ?fallback,
        "cloud stream gave no text"
    );
    fallback
}

fn refused(provider: CloudProvider) -> bool {
    let refused = REFUSED.lock().unwrap_or_else(|error| error.into_inner());
    refused[provider as usize].is_some_and(|at| at.elapsed() < REFUSAL_HOLD)
}

fn refuse(provider: CloudProvider) {
    REFUSED.lock().unwrap_or_else(|error| error.into_inner())[provider as usize] =
        Some(Instant::now());
}

/// A stream whose service hears everything sent before the end and then
/// answers `reply`; the audio it heard comes back on the receiver.
#[cfg(test)]
pub(crate) fn fake(
    provider: CloudProvider,
    reply: Result<String, CloudError>,
) -> (CloudStream, Receiver<Vec<f32>>) {
    let (heard_sender, heard) = mpsc::channel();
    let stream = CloudStream::spawn(provider, move |audio| {
        let mut heard = Vec::new();
        loop {
            match audio.input.recv() {
                Ok(Input::Audio(samples)) => heard.extend(samples),
                Ok(Input::End) => break,
                Err(_) => return Err(CloudError::Failed("discarded".into())),
            }
        }
        let _ = heard_sender.send(heard);
        reply
    })
    .unwrap();
    (stream, heard)
}

/// A released stream whose service answered `reply`.
#[cfg(test)]
pub(crate) fn answered(provider: CloudProvider, reply: Result<String, CloudError>) -> CloudAnswer {
    let (sender, answer) = mpsc::channel();
    let _ = sender.send(reply);
    CloudAnswer {
        provider,
        state: Answer::Waiting(answer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio() -> (Sender<Input>, Audio) {
        let (input, receiver) = mpsc::channel();
        let audio = Audio {
            input: receiver,
            queued: Arc::default(),
        };
        (input, audio)
    }

    #[test]
    fn taking_gathers_the_audio_sent_so_far() {
        let (input, audio) = audio();
        input.send(Input::Audio(vec![0.1, 0.2])).unwrap();
        input.send(Input::Audio(vec![0.3])).unwrap();
        let taken = audio.take().unwrap();
        assert_eq!(taken.audio, [0.1, 0.2, 0.3]);
        assert!(!taken.ended);
        input.send(Input::Audio(vec![0.4])).unwrap();
        input.send(Input::End).unwrap();
        drop(input);
        let taken = audio.take().unwrap();
        assert_eq!(taken.audio, [0.4]);
        assert!(taken.ended);
        let whole = Audio::whole(&[0.5; 3]).take().unwrap();
        assert_eq!((whole.audio, whole.ended), (vec![0.5; 3], true));
    }

    #[test]
    fn a_discarded_capture_stops_its_stream() {
        let (input, audio) = audio();
        drop(input);
        assert!(audio.take().is_err());
    }

    #[test]
    fn the_stream_hears_every_sample_by_its_release() {
        let (mut stream, heard) = fake(CloudProvider::Claude, Ok(" hello there ".into()));
        stream.feed(&[0.1; 3]);
        stream.feed(&[]);
        let answer = stream.end(&[0.2; 2], false);
        assert_eq!(heard.recv().unwrap(), [0.1, 0.1, 0.1, 0.2, 0.2]);
        assert_eq!(
            answer.wait(&CancelToken::new()).as_deref(),
            Ok("hello there")
        );
    }

    #[test]
    fn a_failed_or_empty_stream_fails_the_dictation() {
        for reply in [
            Ok("  ".into()),
            Err(CloudError::SignedOut),
            Err(CloudError::Failed("dropped".into())),
        ] {
            let answer = answered(CloudProvider::Codex, reply);
            assert!(matches!(
                answer.wait(&CancelToken::new()),
                Err(Fallback::Failed(_))
            ));
        }
    }

    #[test]
    fn a_stream_that_heard_past_the_clip_sends_the_whole_clip_instead() {
        let (stream, heard) = fake(CloudProvider::Claude, Ok("with extra words".into()));
        let answer = stream.end(&[], true);
        assert_eq!(answer.wait(&CancelToken::new()), Err(Fallback::WholeClip));
        assert!(
            heard.recv().is_err(),
            "the stream was dropped, not finished"
        );
    }

    #[test]
    fn a_stream_that_falls_too_far_behind_is_given_up() {
        let (mut stream, _) = fake(CloudProvider::Claude, Ok("late".into()));
        for _ in 0..=MAX_QUEUED / 16_000 {
            stream.feed(&[0.0; 16_000]);
        }
        let answer = stream.end(&[0.0; 10], false);
        assert!(matches!(
            answer.wait(&CancelToken::new()),
            Err(Fallback::Failed(_))
        ));
    }

    #[test]
    fn a_cancelled_dictation_stops_waiting() {
        let (sender, answer) = mpsc::channel::<Result<String, CloudError>>();
        let pending = CloudAnswer {
            provider: CloudProvider::Claude,
            state: Answer::Waiting(answer),
        };
        let cancel = CancelToken::new();
        cancel.cancel();
        let started = Instant::now();
        assert!(matches!(pending.wait(&cancel), Err(Fallback::Failed(_))));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(sender);
    }

    #[test]
    fn a_refused_stream_sends_the_whole_clip_and_streams_no_more_for_a_while() {
        *REFUSED.lock().unwrap() = [None, None];
        let answer = answered(CloudProvider::Codex, Err(CloudError::Refused(403)));
        assert_eq!(answer.wait(&CancelToken::new()), Err(Fallback::WholeClip));
        assert!(refused(CloudProvider::Codex));
        assert!(!refused(CloudProvider::Claude));
        assert!(CloudStream::open(CloudProvider::Codex, "en".into(), None).is_none());
        *REFUSED.lock().unwrap() = [None, None];
    }
}
