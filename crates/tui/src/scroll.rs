/// Rows one wheel notch moves, in the transcript and the review's diff;
/// matches a terminal's alternate-scroll translation.
pub(crate) const WHEEL_ROWS: usize = 3;

/// Log scroll position, in rendered wrapped rows. Every `total_len` must be
/// `App::total_lines()`, never an entry or pre-wrap line count; `offset`
/// indexes `ui::Transcript`, one screen row per element.
///
/// Follows new content while `following`; scrolling up disengages it and
/// reaching the bottom re-engages it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollState {
    pub offset: usize,
    pub following: bool,
    pub viewport_height: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            offset: 0,
            following: true,
            viewport_height: 10,
        }
    }
}

impl ScrollState {
    /// Must be called once per frame with the rendered log height, so a
    /// resize keeps a following viewport at the bottom.
    pub fn set_viewport_height(&mut self, height: usize, total_len: usize) {
        self.viewport_height = height.max(1);
        if self.following {
            self.offset = self.max_offset(total_len);
            return;
        }
        // Clamp a disengaged offset too: `max_offset` shrinks when the
        // viewport grows or the log shrinks, leaving the transcript
        // scrolled into blank space.
        self.offset = self.offset.min(self.max_offset(total_len));
    }

    fn max_offset(&self, total_len: usize) -> usize {
        total_len.saturating_sub(self.viewport_height)
    }

    /// Must be called on every push to the log, with the new row count.
    pub fn on_content_grew(&mut self, total_len: usize) {
        if self.following {
            self.offset = self.max_offset(total_len);
        }
    }

    pub fn jump_to_bottom(&mut self, total_len: usize) {
        self.following = true;
        self.offset = self.max_offset(total_len);
    }

    pub fn line_up(&mut self) {
        self.offset = self.offset.saturating_sub(1);
        self.following = false;
    }

    pub fn line_down(&mut self, total_len: usize) {
        let max = self.max_offset(total_len);
        self.offset = (self.offset + 1).min(max);
        self.following = self.offset >= max;
    }

    pub fn page_up(&mut self) {
        self.offset = self.offset.saturating_sub(self.page());
        self.following = false;
    }

    pub fn page_down(&mut self, total_len: usize) {
        let max = self.max_offset(total_len);
        self.offset = (self.offset + self.page()).min(max);
        self.following = self.offset >= max;
    }

    /// A viewport less two rows of overlap, so the far edge stays on screen
    /// as context; at least one row.
    fn page(&self) -> usize {
        self.viewport_height.saturating_sub(2).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_new_content_by_default() {
        let mut s = ScrollState {
            viewport_height: 5,
            ..Default::default()
        };
        s.on_content_grew(3);
        assert_eq!(s.offset, 0); // fewer entries than the viewport
        s.on_content_grew(10);
        assert_eq!(s.offset, 5);
        assert!(s.following);
    }

    #[test]
    fn scrolling_up_disengages_following() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 5,
            following: true,
        };
        s.line_up();
        assert!(!s.following);
        assert_eq!(s.offset, 4);
    }

    #[test]
    fn scrolling_to_the_bottom_reengages_following() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 0,
            following: false,
        };
        for _ in 0..5 {
            s.line_down(10);
        }
        assert!(s.following);
        assert_eq!(s.offset, 5);
    }

    #[test]
    fn end_key_jumps_to_bottom_and_reengages_following() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 0,
            following: false,
        };
        s.jump_to_bottom(20);
        assert!(s.following);
        assert_eq!(s.offset, 15);
    }

    #[test]
    fn once_disengaged_new_content_does_not_move_the_viewport() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 2,
            following: false,
        };
        s.on_content_grew(10);
        assert_eq!(
            s.offset, 2,
            "must not auto-scroll while the developer has scrolled up"
        );
    }

    #[test]
    fn viewport_height_change_repins_a_following_viewport_to_the_new_bottom() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 5,
            following: true,
        };
        s.set_viewport_height(3, 10);
        assert_eq!(s.offset, 7);
    }

    #[test]
    fn viewport_height_change_leaves_a_disengaged_offset_alone() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 2,
            following: false,
        };
        s.set_viewport_height(3, 10);
        assert_eq!(s.offset, 2);
    }

    /// Regression: a grown viewport left the transcript scrolled into blank
    /// space with no fast way back.
    #[test]
    fn a_disengaged_offset_past_the_new_end_is_pulled_back_to_it() {
        let mut s = ScrollState {
            viewport_height: 3,
            offset: 7,
            following: false,
        };
        s.set_viewport_height(8, 10);
        assert_eq!(
            s.offset, 2,
            "the last row must stay reachable when the viewport grows under a scrolled-up offset"
        );
        assert!(
            !s.following,
            "clamping is not the same as jumping to the bottom — following stays off"
        );
    }

    /// `/clear` and a history rewrite shrink the log under an offset.
    #[test]
    fn a_disengaged_offset_past_a_shrunken_log_is_pulled_back_to_it() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 40,
            following: false,
        };
        s.set_viewport_height(5, 10);
        assert_eq!(s.offset, 5);
    }

    #[test]
    fn page_up_and_down_move_by_a_viewport_less_two_rows_of_overlap() {
        let mut s = ScrollState {
            viewport_height: 5,
            offset: 20,
            following: false,
        };
        s.page_up();
        assert_eq!(s.offset, 17);
        s.page_down(30);
        assert_eq!(s.offset, 20);
    }

    #[test]
    fn paging_a_tiny_viewport_still_advances() {
        let mut s = ScrollState {
            viewport_height: 1,
            offset: 4,
            following: false,
        };
        s.page_up();
        assert_eq!(s.offset, 3);
    }
}
