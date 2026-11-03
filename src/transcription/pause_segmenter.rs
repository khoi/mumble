//! Pause detection over 16 kHz audio, so a dictation stream hears only the
//! ends of long pauses and the release tail grows while speech runs on.
//!
//! Speech is any 20 ms frame at least 12 dB above an adaptive noise floor and
//! never quieter than -55 dBFS. The floor starts at the first frame, falls
//! immediately, and rises slowly, so speech between word gaps cannot drag it
//! up to the speech level.

const SAMPLE_RATE: usize = 16_000;
pub(crate) const FRAME_SAMPLES: usize = SAMPLE_RATE / 50;
const SPEECH_MARGIN_DB: f32 = 12.0;
/// How far above the floor a frame may sit and still count as quiet.
const QUIET_MARGIN_DB: f32 = 4.0;
const MINIMUM_SPEECH_DB: f32 = -55.0;
/// Below this a frame is quiet whatever the floor. Lower than
/// `MINIMUM_SPEECH_DB`: a quiet built-in microphone can carry whole
/// dictations under -55 dBFS (#841).
const QUIET_MINIMUM_DB: f32 = -70.0;
const LOWEST_FLOOR_DB: f32 = -90.0;
/// About 3 dB per second at 50 frames per second.
const FLOOR_RISE_DB_PER_FRAME: f32 = 0.06;

/// One frame against the noise floor. A frame can be neither quiet nor
/// speech, but never both.
pub(crate) struct FrameLevel {
    /// The frame sits at the noise floor (within `QUIET_MARGIN_DB`) or below
    /// `QUIET_MINIMUM_DB`. Stricter than "not speech": speech only a few dB
    /// above steady room noise, or from a quiet microphone, is not quiet, so
    /// audio that is dropped as quiet is never quiet speech the speech
    /// margin missed.
    pub quiet: bool,
    pub speech: bool,
}

/// Classifies consecutive 20 ms frames of 16 kHz audio as speech or not
/// against the adaptive noise floor described above.
#[derive(Default)]
pub(crate) struct SpeechFrames {
    floor_db: Option<f32>,
}

impl SpeechFrames {
    /// Whether `frame`, the next `FRAME_SAMPLES` samples, is speech.
    pub(crate) fn is_speech(&mut self, frame: &[f32]) -> bool {
        self.classify(frame).speech
    }

    /// How `frame`, the next `FRAME_SAMPLES` samples, sits against the noise
    /// floor.
    pub(crate) fn classify(&mut self, frame: &[f32]) -> FrameLevel {
        let (level_db, floor_db) = self.measure(frame);
        FrameLevel {
            quiet: level_db <= (floor_db + QUIET_MARGIN_DB).max(QUIET_MINIMUM_DB),
            speech: level_db > (floor_db + SPEECH_MARGIN_DB).max(MINIMUM_SPEECH_DB),
        }
    }

    /// The frame's DC-removed level and the noise floor after it, both in
    /// dBFS. Some microphones (a Ryzen built-in digital microphone, #841)
    /// carry a DC offset far louder than the speech on it; counted as level,
    /// it pins every frame to the floor and the whole dictation reads as a
    /// pause. The dictation silence gate removes DC the same way.
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
pub(crate) mod tests {
    use super::*;

    /// Deterministic noise at a target RMS level in dBFS.
    pub(crate) fn noise(duration_ms: usize, level_db: f32, seed: &mut u32) -> Vec<f32> {
        let amplitude = 10f32.powf(level_db / 20.0) * 3f32.sqrt();
        (0..duration_ms * SAMPLE_RATE / 1_000)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((*seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    /// Alternating speech and pauses: `(speech_ms, pause_ms)`. Speech is
    /// syllable-like bursts separated by 60 ms dips to the room level.
    pub(crate) fn utterance(parts: &[(usize, usize)], floor_db: f32) -> Vec<f32> {
        let mut seed = 7;
        let mut audio = Vec::new();
        let room = |audio: &mut Vec<f32>, duration_ms: usize, seed: &mut u32| {
            if floor_db.is_finite() {
                audio.extend(noise(duration_ms, floor_db, seed));
            } else {
                audio.resize(audio.len() + duration_ms * SAMPLE_RATE / 1_000, 0.0);
            }
        };
        for &(speech, pause) in parts {
            let mut remaining = speech;
            while remaining > 0 {
                let burst = if remaining <= 360 { remaining } else { 240 };
                audio.extend(noise(burst, -20.0, &mut seed));
                remaining -= burst;
                if remaining > 0 {
                    room(&mut audio, 60, &mut seed);
                    remaining -= 60;
                }
            }
            room(&mut audio, pause, &mut seed);
        }
        audio
    }

    fn speech_frames(audio: &[f32]) -> usize {
        let mut speech = SpeechFrames::default();
        audio
            .as_chunks::<FRAME_SAMPLES>()
            .0
            .iter()
            .filter(|frame| speech.is_speech(frame.as_slice()))
            .count()
    }

    #[test]
    fn the_noise_floor_adapts_to_room_noise() {
        let mut seed = 3;
        assert_eq!(speech_frames(&noise(20_000, -40.0, &mut seed)), 0);
        assert!(speech_frames(&utterance(&[(4_000, 1_000)], -40.0)) > 0);
    }

    #[test]
    fn speech_still_registers_after_digital_silence() {
        let mut audio = vec![0.0; SAMPLE_RATE * 5];
        audio.extend(utterance(&[(2_000, 0)], f32::NEG_INFINITY));
        assert!(speech_frames(&audio) > 0);
    }
}
