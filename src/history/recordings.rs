//! Labs › Keep recordings: a dictation saved to History keeps its 16 kHz
//! mono audio as an owner-only 16-bit WAV in `history-audio/<id>.wav`
//! beside the store, so History can transcribe it again with another model.
//!
//! Audio exists only while the toggle is on as a capture is handed to the
//! engine ([`Recording::keep`]). The store owns every file: an entry that
//! leaves History (retention, the entry and text caps, Delete, Clear all)
//! loses its file at the next save, and the recordings have their own byte
//! cap, past which the oldest entries keep their text but lose their audio.

use std::fs;
use std::io::{self, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use crate::transcription::engine::{TranscriptionEngine, dictation_busy, held, transcribe_clip};
use crate::transcription::models::TranscriptionSelection;

/// Every recording together stays under this; about 4.5 hours of audio.
pub const MAX_RECORDING_BYTES: u64 = 512 * 1024 * 1024;
const SAMPLE_RATE: u32 = 16_000;
const SPEC: WavSpec = WavSpec {
    channels: 1,
    sample_rate: SAMPLE_RATE,
    bits_per_sample: 16,
    sample_format: SampleFormat::Int,
};
const DIRECTORY: &str = "history-audio";
/// The longest recording Transcribe again reads: 64 MiB of samples, about
/// 17 minutes.
const MAX_SAMPLES: usize = 64 * 1024 * 1024 / size_of::<f32>();
/// Recordings up to this long run on the dictation engine's model; a longer
/// one would hold a dictation released during it behind it.
const SHARED_MAX_SAMPLES: usize = 30 * SAMPLE_RATE as usize;
/// How long Transcribe again waits out a dictation, such as a locked one,
/// before loading its own copy of the model.
const SHARED_WAIT: Duration = Duration::from_secs(20);
/// Pause before asking again while dictation holds the engine.
const SHARED_RETRY: Duration = Duration::from_millis(50);

/// One dictation's 16 kHz mono audio, kept for History.
#[derive(Clone, Debug)]
pub struct Recording(Vec<f32>);

impl Recording {
    /// A copy of `samples` while Labs › Keep recordings is on; nothing
    /// otherwise, so dictation audio is never kept by default.
    pub fn keep(samples: &[f32]) -> Option<Self> {
        (crate::labs::keep_recordings() && !samples.is_empty()).then(|| Self(samples.to_vec()))
    }

    #[cfg(test)]
    pub fn from_samples(samples: Vec<f32>) -> Self {
        Self(samples)
    }
}

/// The recordings folder beside the history file at `history`.
pub(super) fn directory(history: &Path) -> PathBuf {
    history.with_file_name(DIRECTORY)
}

pub(super) fn path(directory: &Path, id: u64) -> PathBuf {
    directory.join(format!("{id}.wav"))
}

/// Writes entry `id`'s audio atomically and owner-only; returns its size.
pub(super) fn write(directory: &Path, id: u64, recording: &Recording) -> io::Result<u64> {
    fs::create_dir_all(directory)?;
    restrict_directory(directory)?;
    let target = path(directory, id);
    let temporary = target.with_extension("wav.tmp");
    let written = (|| {
        let file = fs::File::create(&temporary)?;
        super::restrict_to_owner(&file)?;
        let mut writer = WavWriter::new(BufWriter::new(&file), SPEC).map_err(io::Error::other)?;
        for sample in &recording.0 {
            let sample = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
            writer.write_sample(sample).map_err(io::Error::other)?;
        }
        writer.finalize().map_err(io::Error::other)?;
        file.sync_all()?;
        fs::rename(&temporary, &target)?;
        Ok(fs::metadata(&target)?.len())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Deletes every recording below `below` (IDs this store has handed out or
/// read) that `keep` rejects. Higher IDs belong to another process's record
/// still on its way to the file, so they are left alone.
pub(super) fn sweep(directory: &Path, below: u64, keep: impl Fn(u64) -> bool) {
    let Ok(files) = fs::read_dir(directory) else {
        return;
    };
    for file in files.flatten() {
        let name = file.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name
            .strip_suffix(".wav")
            .or_else(|| name.strip_suffix(".wav.tmp"))
            .and_then(|id| id.parse::<u64>().ok())
        else {
            continue;
        };
        if id < below
            && (name.ends_with(".tmp") || !keep(id))
            && let Err(error) = fs::remove_file(file.path())
        {
            tracing::warn!(%error, id, "could not delete a dictation recording");
        }
    }
}

/// Reads a recording [`write`] made.
pub(super) fn read(reader: impl Read) -> Result<Vec<f32>, String> {
    let unreadable = || "The recording could not be read.".to_string();
    let reader = WavReader::new(reader).map_err(|_| unreadable())?;
    if reader.spec() != SPEC || reader.duration() == 0 {
        return Err(unreadable());
    }
    if reader.duration() as usize > MAX_SAMPLES {
        return Err("The recording is too long.".into());
    }
    reader
        .into_samples::<i16>()
        .map(|sample| sample.map(|sample| f32::from(sample) / 32_768.0))
        .collect::<Result<_, _>>()
        .map_err(|_| unreadable())
}

/// Transcribes the recording at `path` again with `selection`: on the
/// dictation engine's model when that is the one asked for, yielding to
/// dictation, or on a copy loaded for this run. Blocks; call it off the UI
/// thread.
pub fn retranscribe(path: &Path, selection: TranscriptionSelection) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|_| "The recording is gone.".to_string())?;
    let samples = read(BufReader::new(file))?;
    let samples = if samples.len() <= SHARED_MAX_SAMPLES {
        let samples = Arc::new(samples);
        if let Some(text) = on_engine(&selection, &samples) {
            return trimmed(text);
        }
        Arc::unwrap_or_clone(samples)
    } else {
        samples
    };
    #[cfg(target_os = "macos")]
    let loaded = {
        crate::parakeet::prioritize_inference_thread();
        crate::transcription::Transcriber::load_selection(&selection)
    };
    #[cfg(not(target_os = "macos"))]
    let loaded = crate::transcription::local_transcriber::LocalTranscriber::load(&selection);
    let mut model = loaded.map_err(|error| error.to_string())?;
    trimmed(transcribe_clip(&mut model, samples))
}

/// Runs on the dictation engine's model when it is the GGUF model
/// `selection` asks for, so History never loads a second copy of it. It
/// starts only while no dictation captures or transcribes. `None` when there
/// is no engine, it holds another model, or dictation stays busy for
/// [`SHARED_WAIT`].
fn on_engine(
    selection: &TranscriptionSelection,
    audio: &Arc<Vec<f32>>,
) -> Option<color_eyre::Result<String>> {
    enum Shared {
        Busy,
        Other,
        Ran(color_eyre::Result<String>),
    }
    let engine = TranscriptionEngine::shared()?;
    let started = Instant::now();
    loop {
        if !dictation_busy() {
            let selection = selection.clone();
            let audio = audio.clone();
            let ran = engine.background(move |model| {
                if dictation_busy() {
                    return Shared::Busy;
                }
                match held(model, &selection) {
                    Some(model) => Shared::Ran(transcribe_clip(model, audio.to_vec())),
                    None => Shared::Other,
                }
            });
            match ran {
                Ok(Some(Shared::Ran(text))) => return Some(text),
                Ok(Some(Shared::Other)) | Err(_) => return None,
                Ok(None | Some(Shared::Busy)) => {}
            }
        }
        if started.elapsed() >= SHARED_WAIT {
            return None;
        }
        thread::sleep(SHARED_RETRY);
    }
}

fn trimmed(text: color_eyre::Result<String>) -> Result<String, String> {
    text.map(|text| text.trim().to_string())
        .map_err(|error| error.to_string())
}

#[cfg(unix)]
fn restrict_directory(directory: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_directory(_directory: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcription::models::{CustomModelId, TranscriptionModelId};

    #[test]
    fn transcribe_again_returns_once_the_run_ends() {
        let directory = std::env::temp_dir().join(format!(
            "hex-retranscribe-{}-{}",
            std::process::id(),
            crate::history::now_ms()
        ));
        write(
            &directory,
            1,
            &Recording::from_samples(vec![0.0; SAMPLE_RATE as usize]),
        )
        .unwrap();
        let recording = path(&directory, 1);
        // A model that is not on this computer fails at once, without loading
        // anything; the answer must still come back (#1089).
        let selection = TranscriptionSelection {
            model: TranscriptionModelId::Custom(
                CustomModelId::parse("custom-0000000000000000").unwrap(),
            ),
            language: "en".into(),
        };
        let (sender, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(retranscribe(&recording, selection));
        });
        let answered = answer.recv_timeout(Duration::from_secs(60));
        let _ = fs::remove_dir_all(&directory);
        assert!(
            answered.expect("Transcribe again never answered").is_err(),
            "a missing model cannot transcribe"
        );
    }
}
