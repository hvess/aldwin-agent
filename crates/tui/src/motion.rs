//! The one clock everything on screen moves by, and whether it moves.

use std::time::Duration;

/// The tick period: the working line's 10 frames a second (frame `W2`).
/// Every animation and clock on screen counts ticks, through [`ticks`].
pub(crate) const TICK: Duration = Duration::from_millis(100);

/// Whole ticks in `period`, rounded down.
pub(crate) const fn ticks(period: Duration) -> u64 {
    (period.as_millis() / TICK.as_millis()) as u64
}

/// How much the screen moves: `tui.yaml`'s `motion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Motion {
    /// The caret blinks and the working line animates; the default.
    #[default]
    Full,
    /// The caret and the working line hold still (motion.css's
    /// `prefers-reduced-motion`); the working line's timer still counts.
    Reduced,
}

impl Motion {
    /// Parses `tui.yaml`'s `motion`; anything but `reduced`
    /// (case-insensitive, trimmed), or none, is full.
    ///
    /// ```
    /// use aldwin_tui::Motion;
    ///
    /// assert_eq!(Motion::from_config(Some("reduced")), Motion::Reduced);
    /// assert_eq!(Motion::from_config(None), Motion::Full);
    /// ```
    pub fn from_config(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("reduced") => Motion::Reduced,
            _ => Motion::Full,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_from_config_falls_back_to_full() {
        assert_eq!(Motion::from_config(Some(" Reduced ")), Motion::Reduced);
        assert_eq!(Motion::from_config(Some("none")), Motion::Full);
        assert_eq!(Motion::from_config(None), Motion::Full);
    }

    #[test]
    fn a_period_is_counted_in_whole_ticks() {
        assert_eq!(ticks(Duration::from_secs(1)), 10);
        assert_eq!(ticks(Duration::from_millis(1050)), 10);
    }
}
