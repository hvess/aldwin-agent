/// Log scroll position, in whole entries (not wrapped terminal lines — an
/// entry-granularity approximation, not line-accurate; getting line-accurate
/// scrolling right needs wrap-width-dependent height per entry, computed at
/// render time, which is more machinery than a V0 conversation log needs).
///
/// Auto-follows new content while `following` is true; scrolling up
/// disengages it, and jumping to the bottom (End / `G`) re-engages it — see
/// amundsen-tui.md's Conversation Log scroll behaviour.
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
        }
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

    #[test]
    fn page_up_and_down_move_by_a_full_viewport() {
        let mut s = ScrollState { viewport_height: 5, offset: 20, following: false };
        s.page_up();
        assert_eq!(s.offset, 15);
        s.page_down(30);
        assert_eq!(s.offset, 20);
    }
}
