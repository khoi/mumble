pub(crate) mod cloud;
pub(crate) mod engine;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

pub use cloud::CloudProvider;

/// The language value that lets the service detect the language itself.
pub const AUTO_LANGUAGE: &str = "auto";

/// Cooperative cancellation shared between a dictation job and the cloud call
/// it runs, so dropping the engine or abandoning a job stops the work at once.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The transcription service and language a dictation uses. Persisted as
/// `transcription.provider` and `transcription.language`; a settings file
/// from an older build (which named a local model) loads with the model
/// field ignored and these defaulted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TranscriptionSelection {
    pub provider: CloudProvider,
    pub language: String,
}

impl Default for TranscriptionSelection {
    fn default() -> Self {
        Self {
            provider: CloudProvider::default(),
            language: "en".into(),
        }
    }
}
