//! A dictation streamed to Labs › Cloud transcription's service while it is
//! recorded: its own thread owns the connection and sends the audio as it
//! arrives, so on release only the last words are left to transcribe.

use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use transcribe_cpp::CancelToken;

use super::{CloudError, CloudProvider, WAIT_POLL};
use crate::transcription::engine::{Partial, Partials};

/// How long a provider whose service refused streaming goes straight to
/// whole-clip transcription: streaming is rolled out per account.
const REFUSAL_HOLD: Duration = Duration::from_secs(60 * 60);

/// When each provider's service last refused a stream, by
/// [`CloudProvider::ALL`] order.
static REFUSED: Mutex<[Option<Instant>; 2]> = Mutex::new([None, None]);

/// What the capture sends the stream's thread.
pub(super) enum Input {
    Audio(Vec<f32>),
    /// The recording ended; finish with what was sent.
    End,
}

/// Everything the capture sent since the last look.
#[derive(Default)]
pub(super) struct Taken {
    pub(super) audio: Vec<f32>,
    pub(super) ended: bool,
}

/// Takes what `input` holds without waiting. A capture that let go of the
/// stream without ending it was discarded, so the stream stops.
pub(super) fn take(input: &Receiver<Input>) -> Result<Taken, CloudError> {
    let mut taken = Taken::default();
    loop {
        match input.try_recv() {
            Ok(Input::Audio(samples)) => taken.audio.extend(samples),
            Ok(Input::End) => {
                taken.ended = true;
                return Ok(taken);
            }
            Err(TryRecvError::Empty) => return Ok(taken),
            Err(TryRecvError::Disconnected) => {
                return Err(CloudError::Failed("the dictation was discarded".into()));
            }
        }
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
    /// The service refused to stream: send the whole clip to it instead,
    /// then the model.
    WholeClip,
    /// The model transcribes it.
    Model,
}

pub(crate) struct CloudStream {
    provider: CloudProvider,
    input: Sender<Input>,
    answer: Receiver<Result<String, CloudError>>,
}

impl CloudStream {
    /// Connects to `provider` on its own thread and starts taking audio, or
    /// `None` while its service refuses streams.
    pub(crate) fn open(provider: CloudProvider, partials: Option<Partials>) -> Option<Self> {
        if refused(provider) {
            return None;
        }
        let language = crate::app_settings::transcription_selection().1.language;
        let keyterms = crate::dictation::vocabulary::terms();
        Self::spawn(provider, move |input| match provider {
            CloudProvider::Claude => {
                super::claude::stream(&input, &language, &keyterms, partials.as_ref(), None)
            }
            CloudProvider::Codex => super::codex::stream(&input, &language, partials.as_ref()),
        })
    }

    fn spawn(
        provider: CloudProvider,
        run: impl FnOnce(Receiver<Input>) -> Result<String, CloudError> + Send + 'static,
    ) -> Option<Self> {
        let (input, audio) = mpsc::channel();
        let (reply, answer) = mpsc::channel();
        std::thread::Builder::new()
            .name("cloud-stream".into())
            .spawn(move || {
                let _ = reply.send(run(audio));
            })
            .map_err(|error| tracing::warn!(%error, "couldn't start a cloud stream"))
            .ok()?;
        Some(Self {
            provider,
            input,
            answer,
        })
    }

    pub(crate) fn feed(&self, samples: &[f32]) {
        if !samples.is_empty() {
            let _ = self.input.send(Input::Audio(samples.to_vec()));
        }
    }

    /// Ends the audio and waits for the text, or until `cancel`; without
    /// text, says how the clip should be transcribed instead.
    pub(crate) fn finish(self, cancel: &CancelToken) -> Result<String, Fallback> {
        let started = Instant::now();
        let _ = self.input.send(Input::End);
        let result = loop {
            match self.answer.recv_timeout(WAIT_POLL) {
                Ok(result) => break result,
                Err(RecvTimeoutError::Timeout) if cancel.is_cancelled() => {
                    break Err(CloudError::Failed("the dictation was cancelled".into()));
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
    let fallback = if matches!(error, CloudError::Refused(_)) {
        refuse(provider);
        Fallback::WholeClip
    } else {
        Fallback::Model
    };
    tracing::warn!(
        provider = provider.name(),
        finish_ms,
        reason = %error.reason(provider),
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

/// A stream whose service hears everything sent before `End` and then
/// answers `reply`; the audio it heard comes back on the receiver.
#[cfg(test)]
pub(crate) fn fake(
    provider: CloudProvider,
    reply: Result<String, CloudError>,
) -> (CloudStream, Receiver<Vec<f32>>) {
    let (heard_sender, heard) = mpsc::channel();
    let stream = CloudStream::spawn(provider, move |input| {
        let mut audio = Vec::new();
        loop {
            match input.recv() {
                Ok(Input::Audio(samples)) => audio.extend(samples),
                Ok(Input::End) => break,
                Err(_) => return Err(CloudError::Failed("discarded".into())),
            }
        }
        let _ = heard_sender.send(audio);
        reply
    })
    .unwrap();
    (stream, heard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taking_gathers_the_audio_sent_so_far() {
        let (input, audio) = mpsc::channel();
        input.send(Input::Audio(vec![0.1, 0.2])).unwrap();
        input.send(Input::Audio(vec![0.3])).unwrap();
        let taken = take(&audio).unwrap();
        assert_eq!(taken.audio, [0.1, 0.2, 0.3]);
        assert!(!taken.ended);
        input.send(Input::Audio(vec![0.4])).unwrap();
        input.send(Input::End).unwrap();
        drop(input);
        let taken = take(&audio).unwrap();
        assert_eq!(taken.audio, [0.4]);
        assert!(taken.ended);
    }

    #[test]
    fn a_discarded_capture_stops_its_stream() {
        let (input, audio) = mpsc::channel::<Input>();
        drop(input);
        assert!(take(&audio).is_err());
    }

    #[test]
    fn the_stream_hears_every_sample_before_it_answers() {
        let (stream, heard) = fake(CloudProvider::Claude, Ok(" hello there ".into()));
        stream.feed(&[0.1; 3]);
        stream.feed(&[]);
        stream.feed(&[0.2; 2]);
        assert_eq!(
            stream.finish(&CancelToken::new()).as_deref(),
            Ok("hello there")
        );
        assert_eq!(heard.recv().unwrap(), [0.1, 0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn a_failed_or_empty_stream_leaves_the_clip_to_the_model() {
        for reply in [
            Ok("  ".into()),
            Err(CloudError::SignedOut),
            Err(CloudError::Failed("dropped".into())),
        ] {
            let (stream, _) = fake(CloudProvider::Codex, reply);
            assert_eq!(stream.finish(&CancelToken::new()), Err(Fallback::Model));
        }
    }

    #[test]
    fn a_refused_stream_sends_the_whole_clip_and_streams_no_more_for_a_while() {
        *REFUSED.lock().unwrap() = [None, None];
        let (stream, _) = fake(CloudProvider::Codex, Err(CloudError::Refused(403)));
        assert_eq!(stream.finish(&CancelToken::new()), Err(Fallback::WholeClip));
        assert!(refused(CloudProvider::Codex));
        assert!(!refused(CloudProvider::Claude));
        assert!(CloudStream::open(CloudProvider::Codex, None).is_none());
        *REFUSED.lock().unwrap() = [None, None];
    }
}
