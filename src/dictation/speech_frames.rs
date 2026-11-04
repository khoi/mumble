//! Speech detection over 16 kHz audio, so the release tail grows while
//! speech runs into it.
//!
//! Speech is any 20 ms frame at least 12 dB above an adaptive noise floor and
//! never quieter than -55 dBFS. The floor starts at the first frame, falls
//! immediately, and rises slowly, so speech between word gaps cannot drag it
//! up to the speech level.

const SAMPLE_RATE: usize = 16_000;
pub(crate) const FRAME_SAMPLES: usize = SAMPLE_RATE / 50;
const SPEECH_MARGIN_DB: f32 = 12.0;
const MINIMUM_SPEECH_DB: f32 = -55.0;
const LOWEST_FLOOR_DB: f32 = -90.0;
/// About 3 dB per second at 50 frames per second.
const FLOOR_RISE_DB_PER_FRAME: f32 = 0.06;

/// Classifies consecutive 20 ms frames of 16 kHz audio as speech or not
/// against the adaptive noise floor described above.
#[derive(Default)]
pub(crate) struct SpeechFrames {
    floor_db: Option<f32>,
}

impl SpeechFrames {
    /// Whether `frame`, the next `FRAME_SAMPLES` samples, is speech.
    pub(crate) fn is_speech(&mut self, frame: &[f32]) -> bool {
        let (level_db, floor_db) = self.measure(frame);
        level_db > (floor_db + SPEECH_MARGIN_DB).max(MINIMUM_SPEECH_DB)
    }

    /// The frame's DC-removed level and the noise floor after it, both in
    /// dBFS. Some microphones (a Ryzen built-in digital microphone, #841)
    /// carry a DC offset far louder than the speech on it; counted as level,
    /// it pins every frame to the floor. The dictation silence gate removes
    /// DC the same way.
    fn measure(&mut self, frame: &[f32]) -> (f32, f32) {
        let count = frame.len() as f32;
        let mean = frame.iter().sum::<f32>() / count;
        let energy = frame
            .iter()
            .map(|sample| (sample - mean) * (sample - mean))
            .sum::<f32>()
            / count;
        let level_db = if energy.is_finite() {
            10.0 * energy.max(1e-18).log10()
        } else {
            0.0
        };
        let floor_db = match self.floor_db {
            Some(floor_db) if level_db >= floor_db => floor_db + FLOOR_RISE_DB_PER_FRAME,
            _ => level_db.max(LOWEST_FLOOR_DB),
        };
        self.floor_db = Some(floor_db);
        (level_db, floor_db)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_tone_reads_as_speech_and_silence_does_not() {
        let tone: Vec<f32> = (0..FRAME_SAMPLES)
            .map(|i| 0.3 * (i as f32 * 0.2).sin())
            .collect();
        let silence = [0.0; FRAME_SAMPLES];
        // A quiet frame settles the floor low; a loud tone over it is speech.
        let mut frames = SpeechFrames::default();
        frames.is_speech(&silence);
        assert!(frames.is_speech(&tone));
        // Silence over the same settled floor is never speech.
        let mut frames = SpeechFrames::default();
        frames.is_speech(&silence);
        assert!(!frames.is_speech(&silence));
    }
}
