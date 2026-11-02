//! The reading position (#153): how far through its body the reader last
//! was, in the whole thousandths `article_state.read_permille` stores.

/// How far through an article's body the reader last was (#153): a fraction
/// of the body's scroll extent, `0` at the top and `1` at the end.
///
/// Held in whole thousandths, so the projection stays `Eq`, a position that
/// did not move is an exact no-op, and a stored value never drifts through
/// float round-trips. A thousandth of even a very long article is a few
/// lines — well inside a paragraph, which is all a resume needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ReadProgress(u16);

impl ReadProgress {
    /// The top of the article — nothing read yet.
    pub const START: Self = Self(0);
    /// The end of the article.
    pub const END: Self = Self(Self::SCALE);

    /// Thousandths per whole article.
    const SCALE: u16 = 1000;

    /// The progress for a fraction of the body, clamped to `[0, 1]` and
    /// rounded to the nearest thousandth. `NaN` (an empty body's `0 / 0`)
    /// reads as [`Self::START`].
    #[must_use]
    pub fn from_fraction(fraction: f64) -> Self {
        if fraction.is_nan() {
            return Self::START;
        }
        let scaled = (fraction.clamp(0.0, 1.0) * f64::from(Self::SCALE)).round();
        // In range by construction: clamped to [0, 1] and scaled by 1000,
        // `scaled` is a whole number in [0, 1000] — no truncation, no sign.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "scaled is a whole number in [0, 1000]"
        )]
        let permille = scaled as u16;
        Self(permille)
    }

    /// The stored form, in thousandths (`0..=1000`), if `permille` is in range.
    #[must_use]
    pub fn from_permille(permille: u16) -> Option<Self> {
        (permille <= Self::SCALE).then_some(Self(permille))
    }

    /// The progress in thousandths (`0..=1000`) — the stored form.
    #[must_use]
    pub fn permille(self) -> u16 {
        self.0
    }

    /// The progress as a fraction in `[0, 1]`.
    #[must_use]
    pub fn fraction(self) -> f64 {
        f64::from(self.0) / f64::from(Self::SCALE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_progress_clamps_and_rounds_to_thousandths() {
        assert_eq!(ReadProgress::from_fraction(0.0), ReadProgress::START);
        assert_eq!(ReadProgress::from_fraction(1.0), ReadProgress::END);
        assert_eq!(ReadProgress::from_fraction(0.4567).permille(), 457);
        assert_eq!(ReadProgress::from_fraction(0.4564).permille(), 456);
        // Out of range clamps; an empty body's 0/0 is the top.
        assert_eq!(ReadProgress::from_fraction(-0.2), ReadProgress::START);
        assert_eq!(ReadProgress::from_fraction(3.0), ReadProgress::END);
        assert_eq!(ReadProgress::from_fraction(f64::NAN), ReadProgress::START);
        assert_eq!(
            ReadProgress::from_fraction(f64::INFINITY),
            ReadProgress::END
        );
        assert!((ReadProgress::from_fraction(0.25).fraction() - 0.25).abs() < f64::EPSILON);
        assert_eq!(ReadProgress::default(), ReadProgress::START);
    }

    #[test]
    fn read_progress_accepts_only_in_range_permille() {
        assert_eq!(ReadProgress::from_permille(1000), Some(ReadProgress::END));
        assert_eq!(ReadProgress::from_permille(0), Some(ReadProgress::START));
        assert_eq!(ReadProgress::from_permille(1001), None);
    }
}
