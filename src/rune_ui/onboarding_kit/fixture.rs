//! Stand-ins for what the running app reports: the microphone's
//! `Meter { average, peak }` and a recognizer's partial transcripts.

pub(crate) struct Take {
    pub(crate) updates: &'static [(f64, &'static str)],
    pub(crate) release: f64,
}

impl Take {
    pub(crate) fn partial(&self, elapsed: f64) -> &'static str {
        self.updates
            .iter()
            .take_while(|(at, _)| *at <= elapsed)
            .last()
            .map_or("", |(_, text)| text)
    }

    pub(crate) fn finished(&self, elapsed: f64) -> bool {
        elapsed >= self.release
    }

    pub(crate) fn text(&self) -> &'static str {
        self.updates.last().map_or("", |(_, text)| text)
    }

    pub(crate) fn meter(&self, elapsed: f64) -> (f32, f32) {
        if elapsed < 0.0 || elapsed > self.release {
            return (0.02, 0.04);
        }
        let t = elapsed as f32;
        let syllables = ((t * 9.2).sin() * (t * 3.7 + 0.7).sin()).abs();
        let breath = 0.65 + 0.35 * (t * 2.1).sin().abs();
        let average = (0.10 + 0.58 * syllables * breath).min(1.0);
        (average, (average * 1.45).min(1.0))
    }
}

/// Confident 1.5 s spoken answer when the investors are in the room.
pub(crate) const ANSWER_JAY: Take = Take {
    updates: &[
        (0.14, "Every"),
        (0.26, "Every test"),
        (0.38, "Every test is"),
        (0.52, "Every test is green"),
        (0.66, "Every test is green and"),
        (0.80, "Every test is green and prod"),
        (0.94, "Every test is green and prod is"),
        (1.08, "Every test is green and prod is warm."),
        (1.22, "Every test is green and prod is warm. Ship it"),
        (1.36, "Every test is green and prod is warm. Ship it right"),
        (
            1.46,
            "Every test is green and prod is warm. Ship it right now.",
        ),
    ],
    release: 1.5,
};

/// Saying anything on the try screen.
pub(crate) const ANSWER_TRY: Take = Take {
    updates: &[
        (0.4, "Okay"),
        (0.7, "Okay this"),
        (0.88, "Okay this is"),
        (1.2, "Okay this is actually"),
        (1.45, "Okay this is actually kind"),
        (1.6, "Okay this is actually kind of"),
        (2.0, "Okay, this is actually kind of magic."),
    ],
    release: 2.4,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_take_reports_its_newest_partial_in_one_and_a_half_seconds() {
        assert_eq!(ANSWER_JAY.partial(0.0), "");
        assert_eq!(ANSWER_JAY.partial(0.3), "Every test");
        assert!(ANSWER_JAY.partial(2.0).ends_with('.'));
        assert!(ANSWER_JAY.finished(1.5));
    }
}
