//! Labs › Cloud transcription: a dictation transcribed by Claude or Codex
//! through the sign-in their own apps already hold, with Hex's local model
//! as the fallback whenever the service can't answer.
//!
//! Hex never refreshes or writes either sign-in: Claude Code and Codex own
//! their tokens, so an expired one reads as signed out until its app renews
//! it. Nothing here runs unless the feature is on, read as each dictation's
//! run starts.

mod claude;
mod codex;

use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::engine::Offline;

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

/// `local`, after the provider Labs › Cloud transcription names as the run
/// starts; whenever that gives no text, the clip is transcribed here.
pub(crate) fn cloud_first(local: Offline) -> Offline {
    Box::new(
        move |model, clip| match crate::labs::cloud_transcription() {
            Some(provider) => {
                let language = crate::app_settings::transcription_selection().1.language;
                let started = Instant::now();
                let attempt = transcribe(provider, &clip, &language);
                tracing::info!(
                    provider = provider.name(),
                    ok = attempt.is_ok(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "cloud transcription"
                );
                or_local(provider, attempt, || local(model, clip))
            }
            None => local(model, clip),
        },
    )
}

fn or_local(
    provider: CloudProvider,
    attempt: Result<String, CloudError>,
    local: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    attempt.or_else(|error| {
        tracing::warn!(
            provider = provider.name(),
            reason = %error.reason(provider),
            "cloud transcription failed; transcribing on this computer"
        );
        local()
    })
}

/// A 16 kHz mono clip's text from `provider`, in `language` (a code, or
/// `auto`).
pub fn transcribe(
    provider: CloudProvider,
    clip: &[f32],
    language: &str,
) -> Result<String, CloudError> {
    let text = match provider {
        CloudProvider::Claude => {
            claude::transcribe(clip, language, &crate::dictation::vocabulary::terms())?
        }
        CloudProvider::Codex => codex::transcribe(clip, language)?,
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

    #[test]
    fn a_cloud_answer_is_used_without_the_local_model() {
        let mut ran_local = false;
        let text = or_local(CloudProvider::Claude, Ok("hello there".into()), || {
            ran_local = true;
            Ok("local".into())
        });
        assert_eq!(text, Ok("hello there".into()));
        assert!(!ran_local);
    }

    #[test]
    fn any_cloud_failure_transcribes_on_this_computer() {
        for error in [
            CloudError::SignedOut,
            CloudError::Language("vi".into()),
            CloudError::RateLimited,
            CloudError::Refused(403),
            CloudError::NoSpeech,
            CloudError::Failed("Couldn't reach chatgpt.com".into()),
        ] {
            let text = or_local(CloudProvider::Codex, Err(error), || Ok("local".into()));
            assert_eq!(text, Ok("local".into()));
        }
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
