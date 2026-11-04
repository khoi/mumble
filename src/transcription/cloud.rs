//! The transcriber: a dictation transcribed by Claude or Codex through the
//! sign-in their own apps already hold, streamed while it records (`stream`)
//! or sent whole on release. There is no local model, so a dictation the
//! service can't transcribe fails with the reason.
//!
//! Hex never refreshes or writes either sign-in: Claude Code and Codex own
//! their tokens, so an expired one reads as signed out until its app renews
//! it. The provider and language come from the saved transcription selection,
//! read as each capture's stream opens and as each whole clip's run starts.

mod claude;
mod codex;
mod socket;
mod stream;

use stream::Fallback;
pub(crate) use stream::{CloudAnswer, CloudStream};

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::transcription::CancelToken;

/// How often the engine looks at its job while the service works.
const WAIT_POLL: Duration = Duration::from_millis(50);

/// The service a cloud dictation goes to.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CloudProvider {
    #[default]
    Claude,
    Codex,
}

impl CloudProvider {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }

    /// The app whose sign-in it uses.
    const fn app(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }
}

/// Why a cloud transcription gave no text.
#[derive(Debug, PartialEq, Eq)]
pub enum CloudError {
    /// No sign-in, or one that expired.
    SignedOut,
    /// The service doesn't take the dictation language.
    Language(String),
    RateLimited,
    /// The service refused this account or client (HTTP status).
    Refused(u16),
    NoSpeech,
    Failed(String),
}

impl CloudError {
    fn reason(&self, provider: CloudProvider) -> String {
        let app = provider.app();
        match self {
            Self::SignedOut => {
                format!("not signed in to {app}, or its sign-in expired; open {app} to sign in")
            }
            Self::Language(language) => {
                format!("{} doesn't transcribe {language:?}", provider.name())
            }
            Self::RateLimited => format!("{} is rate limiting this account", provider.name()),
            Self::Refused(status) => {
                format!("{} refused the request (HTTP {status})", provider.name())
            }
            Self::NoSpeech => format!("{} heard no speech", provider.name()),
            Self::Failed(reason) => reason.clone(),
        }
    }
}

/// A finished dictation's text: its stream's answer, else the whole clip
/// sent to the service. The dictation fails with the reason when the service
/// can't transcribe it; there is no local fallback.
pub(crate) fn dictation(
    clip: &[f32],
    cancel: &CancelToken,
    streamed: Option<CloudAnswer>,
) -> Result<String, String> {
    let provider = crate::app_settings::transcription_selection().1.provider;
    dictation_with(
        provider,
        clip,
        cancel,
        streamed,
        |provider, clip, deadline| {
            let language = crate::app_settings::transcription_selection().1.language;
            transcribe(provider, &clip, &language, deadline)
        },
    )
}

fn dictation_with(
    provider: CloudProvider,
    clip: &[f32],
    cancel: &CancelToken,
    streamed: Option<CloudAnswer>,
    attempt: impl FnOnce(CloudProvider, Vec<f32>, Instant) -> Result<String, CloudError>
    + Send
    + 'static,
) -> Result<String, String> {
    match streamed.map(|answer| answer.wait(cancel)) {
        Some(Ok(text)) => return Ok(text),
        Some(Err(Fallback::Failed(reason))) => return Err(reason),
        Some(Err(Fallback::WholeClip)) | None => {}
    }
    let started = Instant::now();
    let deadline = started + budget(clip.len());
    let clip = clip.to_vec();
    let result = within(deadline, cancel, move || attempt(provider, clip, deadline));
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(text) => {
            tracing::info!(
                provider = provider.name(),
                elapsed_ms,
                "cloud transcription"
            );
            Ok(text)
        }
        Err(error) => {
            let reason = error.reason(provider);
            tracing::warn!(
                provider = provider.name(),
                elapsed_ms,
                %reason,
                "cloud transcription failed"
            );
            Err(reason)
        }
    }
}

/// How long a cloud attempt may take, connecting and sending included:
/// half a minute plus the clip's length, well inside the engine's job
/// deadline, so the model keeps its time when the service fails.
fn budget(samples: usize) -> Duration {
    Duration::from_secs(30) + Duration::from_secs_f64(samples as f64 / 16_000.0)
}

/// `attempt`'s answer, waited for off its own thread, so a cancelled job
/// or a stopping engine stops waiting at once; the attempt itself ends by
/// `deadline`.
fn within(
    deadline: Instant,
    cancel: &CancelToken,
    attempt: impl FnOnce() -> Result<String, CloudError> + Send + 'static,
) -> Result<String, CloudError> {
    let (sender, answer) = mpsc::channel();
    std::thread::Builder::new()
        .name("cloud-transcription".into())
        .spawn(move || {
            let _ = sender.send(attempt());
        })
        .map_err(|error| CloudError::Failed(format!("couldn't start: {error}")))?;
    loop {
        match answer.recv_timeout(WAIT_POLL) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(CloudError::Failed("the attempt stopped".into()));
            }
            Err(RecvTimeoutError::Timeout) if cancel.is_cancelled() => {
                return Err(CloudError::Failed("the dictation was cancelled".into()));
            }
            Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                return Err(CloudError::Failed("no answer in time".into()));
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// A 16 kHz mono clip's text from `provider`, in `language` (a code, or
/// `auto`), by `deadline`.
fn transcribe(
    provider: CloudProvider,
    clip: &[f32],
    language: &str,
    deadline: Instant,
) -> Result<String, CloudError> {
    let text = match provider {
        CloudProvider::Claude => claude::transcribe(
            clip,
            language,
            &crate::dictation::vocabulary::terms(),
            deadline,
        )?,
        CloudProvider::Codex => codex::transcribe(clip, language, deadline)?,
    };
    match text.trim() {
        "" => Err(CloudError::NoSpeech),
        text => Ok(text.to_owned()),
    }
}

/// 16-bit little-endian PCM, as both services take it.
fn pcm16(clip: &[f32]) -> impl Iterator<Item = i16> + '_ {
    clip.iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16)
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_millis() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    #[test]
    fn the_chosen_service_hears_the_whole_clip_and_its_text_is_used() {
        let heard = Arc::new(Mutex::new(None));
        let seen = heard.clone();
        let text = dictation_with(
            CloudProvider::Codex,
            &[0.1, 0.2, 0.3],
            &CancelToken::new(),
            None,
            move |provider, clip, _| {
                *seen.lock().unwrap() = Some((provider, clip));
                Ok("hello there".into())
            },
        );
        assert_eq!(text.as_deref(), Ok("hello there"));
        assert_eq!(
            heard.lock().unwrap().take(),
            Some((CloudProvider::Codex, vec![0.1, 0.2, 0.3]))
        );
    }

    #[test]
    fn a_failed_attempt_fails_the_dictation_with_its_reason() {
        for error in [
            CloudError::SignedOut,
            CloudError::Language("vi".into()),
            CloudError::RateLimited,
            CloudError::Refused(403),
            CloudError::NoSpeech,
            CloudError::Failed("Couldn't reach chatgpt.com".into()),
        ] {
            let expected = error.reason(CloudProvider::Claude);
            let text = dictation_with(
                CloudProvider::Claude,
                &[0.1; 4],
                &CancelToken::new(),
                None,
                move |_, _, _| Err(error),
            );
            assert_eq!(text, Err(expected));
        }
    }

    #[test]
    fn a_cancelled_job_stops_waiting_for_the_service() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let started = Instant::now();
        let text = dictation_with(
            CloudProvider::Claude,
            &[0.1; 4],
            &cancel,
            None,
            |_, _, _| {
                std::thread::sleep(Duration::from_secs(10));
                Ok("late".into())
            },
        );
        assert!(text.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_streamed_answer_is_used_without_sending_the_clip() {
        let text = dictation_with(
            CloudProvider::Claude,
            &[0.1; 4],
            &CancelToken::new(),
            Some(stream::answered(
                CloudProvider::Claude,
                Ok("streamed".into()),
            )),
            |_, _, _| panic!("the clip must not be sent again"),
        );
        assert_eq!(text.as_deref(), Ok("streamed"));
    }

    #[test]
    fn a_stream_that_failed_fails_the_dictation() {
        let text = dictation_with(
            CloudProvider::Claude,
            &[0.1; 4],
            &CancelToken::new(),
            Some(stream::answered(
                CloudProvider::Claude,
                Err(CloudError::Failed("dropped".into())),
            )),
            |_, _, _| panic!("a failed stream doesn't retry the service"),
        );
        assert!(text.is_err());
    }

    #[test]
    fn a_refused_stream_sends_the_whole_clip_to_the_service() {
        let sent = Arc::new(Mutex::new(None));
        let seen = sent.clone();
        let text = dictation_with(
            CloudProvider::Codex,
            &[0.1, 0.2],
            &CancelToken::new(),
            Some(stream::answered(
                CloudProvider::Codex,
                Err(CloudError::Refused(403)),
            )),
            move |provider, clip, _| {
                *seen.lock().unwrap() = Some((provider, clip));
                Ok("whole".into())
            },
        );
        assert_eq!(text.as_deref(), Ok("whole"));
        assert_eq!(
            sent.lock().unwrap().take(),
            Some((CloudProvider::Codex, vec![0.1, 0.2]))
        );
    }

    #[test]
    fn the_wait_ends_at_the_deadline() {
        let started = Instant::now();
        let result = within(
            started + Duration::from_millis(100),
            &CancelToken::new(),
            || {
                std::thread::sleep(Duration::from_secs(10));
                Ok("late".into())
            },
        );
        assert!(matches!(result, Err(CloudError::Failed(_))));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn the_budget_leaves_the_model_its_time() {
        assert_eq!(budget(0), Duration::from_secs(30));
        assert_eq!(budget(16_000 * 60), Duration::from_secs(90));
    }

    #[test]
    fn samples_become_clamped_16_bit_pcm() {
        let pcm: Vec<i16> = pcm16(&[0.0, 1.0, -1.0, 2.0, -2.0, 0.5]).collect();
        assert_eq!(pcm, [0, 32767, -32767, 32767, -32767, 16384]);
    }

    #[test]
    fn providers_save_as_lowercase_names() {
        assert_eq!(
            serde_json::to_value(CloudProvider::Codex).unwrap(),
            serde_json::json!("codex")
        );
        assert_eq!(
            serde_json::from_value::<CloudProvider>(serde_json::json!("claude")).unwrap(),
            CloudProvider::Claude
        );
    }
}
