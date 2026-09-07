/// Log scroll position, in rendered *wrapped* terminal rows — `total_len`
/// in every method below must be `App::total_lines()` (see its doc
/// comment), not `App::log`'s entry count and not a logical (pre-wrap)
/// line count either. `viewport_height` is real rendered rows too (only
/// known at render time); comparing that against an *entry* count in
/// `max_offset` is what made scrolling a near-total no-op before
/// `total_lines` existed — a handful of entries routinely render to far
/// more rows than the viewport, so `max_offset` stayed 0 long after there
/// was real content below the fold. `offset` indexes straight into
/// `ui::transcript_rows`, which is one screen row per element by
/// construction — see that function's doc comment for how that invariant is
/// established, and for the three-passes-per-frame wrapping it replaced.
///
/// Auto-follows new content while `following` is true; scrolling up
/// disengages it, and jumping to the bottom (End / `G`) re-engages it — see
/// mjolnir-tui.md's Conversation Log scroll behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollState {
    pub offset:          usize,
    pub following:        bool,
    pub viewport_height: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self { offset: 0, following: true, viewport_height: 10 }
    }
}

impl ScrollState {
    /// Call once per frame with the actual rendered log-area height (only
    /// known at render time) and the current entry count — keeps a
    /// following viewport pinned to the bottom across a terminal resize
    /// instead of showing a stale offset from before it.
    pub fn set_viewport_height(&mut self, height: usize, total_len: usize) {
        self.viewport_height = height.max(1);
        if self.following {
            self.offset = self.max_offset(total_len);
            return;
        }
        // A *disengaged* offset still has to stay reachable. It only ever
        // moves down under `line_down`/`page_down`, which clamp against the
        // `max_offset` of the moment — but that maximum shrinks whenever the
        // viewport grows (a wider terminal rewraps the transcript into fewer
        // rows; a resolved permission panel hands its band back to the log),
        // and nothing was pulling the offset back with it. The transcript
        // then scrolled off the top of its own viewport into blank space,
        // and getting back to the conversation meant holding Up for as many
        // presses as the terminal had grown by — the "scrolling up and down
        // is very difficult" half of the report.
        self.offset = self.offset.min(self.max_offset(total_len));
    }

    fn max_offset(&self, total_len: usize) -> usize {
        total_len.saturating_sub(self.viewport_height)
    }

    /// Called whenever a new entry is pushed to the log.
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
        self.offset = self.offset.saturating_sub(self.viewport_height);
        self.following = false;
    }

    pub fn page_down(&mut self, total_len: usize) {
        let max = self.max_offset(total_len);
        self.offset = (self.offset + self.viewport_height).min(max);
        self.following = self.offset >= max;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_new_content_by_default() {
        let mut s = ScrollState { viewport_height: 5, ..Default::default() };
        s.on_content_grew(3);
        assert_eq!(s.offset, 0); // fewer entries than the viewport
        s.on_content_grew(10);
        assert_eq!(s.offset, 5);
        assert!(s.following);
    }

    #[test]
    fn scrolling_up_disengages_following() {
        let mut s = ScrollState { viewport_height: 5, offset: 5, following: true };
        s.line_up();
        assert!(!s.following);
        assert_eq!(s.offset, 4);
    }

    #[test]
    fn scrolling_to_the_bottom_reengages_following() {
        let mut s = ScrollState { viewport_height: 5, offset: 0, following: false };
        for _ in 0..5 {
            s.line_down(10);
        }
        assert!(s.following);
        assert_eq!(s.offset, 5);
    }

    #[test]
    fn end_key_jumps_to_bottom_and_reengages_following() {
        let mut s = ScrollState { viewport_height: 5, offset: 0, following: false };
        s.jump_to_bottom(20);
        assert!(s.following);
        assert_eq!(s.offset, 15);
    }

    #[test]
    fn once_disengaged_new_content_does_not_move_the_viewport() {
        let mut s = ScrollState { viewport_height: 5, offset: 2, following: false };
        s.on_content_grew(10);
        assert_eq!(s.offset, 2, "must not auto-scroll while the developer has scrolled up");
    }

    #[test]
    fn viewport_height_change_repins_a_following_viewport_to_the_new_bottom() {
        let mut s = ScrollState { viewport_height: 5, offset: 5, following: true };
        s.set_viewport_height(3, 10);
        assert_eq!(s.offset, 7);
    }

    #[test]
    fn viewport_height_change_leaves_a_disengaged_offset_alone() {
        let mut s = ScrollState { viewport_height: 5, offset: 2, following: false };
        s.set_viewport_height(3, 10);
        assert_eq!(s.offset, 2);
    }

    /// The other half of the pair above: an offset that was legal for a
    /// small viewport is *past the end* once the viewport grows, and left
    /// alone it parks the transcript off the top of its own log area with
    /// nothing on screen and no fast way back.
    #[test]
    fn a_disengaged_offset_past_the_new_end_is_pulled_back_to_it() {
        let mut s = ScrollState { viewport_height: 3, offset: 7, following: false };
        s.set_viewport_height(8, 10);
        assert_eq!(s.offset, 2, "the last row must stay reachable when the viewport grows under a scrolled-up offset");
        assert!(!s.following, "clamping is not the same as jumping to the bottom — following stays off");
    }

    /// Same clamp, driven by content rather than by size: `/clear` and a
    /// history rewrite both shrink the log out from under an offset.
    #[test]
    fn a_disengaged_offset_past_a_shrunken_log_is_pulled_back_to_it() {
        let mut s = ScrollState { viewport_height: 5, offset: 40, following: false };
        s.set_viewport_height(5, 10);
        assert_eq!(s.offset, 5);
    }

    #[test]
    fn page_up_and_down_move_by_a_full_viewport() {
        let mut s = ScrollState { viewport_height: 5, offset: 20, following: false };
        s.page_up();
        assert_eq!(s.offset, 15);
        s.page_down(30);
        assert_eq!(s.offset, 20);
    }
}
