//! What the running turn is doing, as the footer's working line says it
//! (frames B–D, `W1`, `W2`). `App::apply_event` feeds it; `ui::working`
//! draws it. The words come from the call itself, never from the model.

use std::path::Path;
use std::time::Duration;

use crate::log::{Verb, WorkItem};
use crate::motion::ticks;

/// No new output for this long reads as a stall (frame `W2`).
const STALL: u64 = ticks(Duration::from_secs(30));

/// One second, for the timer.
const SECOND: u64 = ticks(Duration::from_secs(1));

/// The running turn's phrase and clocks, all in ticks.
#[derive(Debug, Default)]
pub(crate) struct Activity {
    /// When the turn began; the timer counts from it.
    began: u64,
    /// When the current phrase began; its typing counts from it.
    since: u64,
    /// When the stall clock last restarted.
    heard: u64,
    doing: Doing,
}

/// The working line at one tick.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WorkingLine {
    /// `Reading limit.rs`, or `Still reading limit.rs` once stalled.
    pub words: String,
    /// Ticks since the phrase began.
    pub age: u64,
    /// Nothing new for `STALL`: drawn still, in `label2`.
    pub stalled: bool,
    /// Whole seconds since the turn began.
    pub seconds: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Doing {
    /// Waiting on the model, or its reasoning streaming.
    #[default]
    Thinking,
    /// The reply's prose streaming.
    Replying,
    /// A call running: its verb and what it acts on.
    Call(Verb, String),
}

impl Doing {
    /// A file is named by its file name alone and a command by its first
    /// line: the disclosure carries the whole of either.
    fn call(verb: Verb, target: &str) -> Self {
        let shown = match verb {
            Verb::Read | Verb::Changed => Path::new(target)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(target),
            Verb::Ran | Verb::Searched | Verb::Used => target.lines().next().unwrap_or(""),
        };
        Doing::Call(verb, shown.trim().to_string())
    }

    /// Lower case, as it follows `Still`.
    fn words(&self) -> String {
        match self {
            Doing::Thinking => "thinking".into(),
            Doing::Replying => "writing a reply".into(),
            Doing::Call(verb, target) if target.is_empty() => verb.doing().into(),
            Doing::Call(verb, target) => format!("{} {target}", verb.doing()),
        }
    }
}

impl Activity {
    /// A turn beginning at `tick`, thinking.
    pub(crate) fn new(tick: u64) -> Self {
        Self {
            began: tick,
            since: tick,
            heard: tick,
            ..Self::default()
        }
    }

    /// Restarts the stall clock: something new happened, or the developer
    /// had the screen.
    pub(crate) fn touch(&mut self, tick: u64) {
        self.heard = tick;
    }

    /// The model is reasoning, or has not answered yet; a running call
    /// keeps the line.
    pub(crate) fn thinking(&mut self, tick: u64) {
        if !matches!(self.doing, Doing::Call(..)) {
            self.set(Doing::Thinking, tick);
        }
    }

    /// The reply's prose is streaming; a running call keeps the line.
    pub(crate) fn replying(&mut self, tick: u64) {
        if !matches!(self.doing, Doing::Call(..)) {
            self.set(Doing::Replying, tick);
        }
    }

    /// A call started or finished: the line names `running`, the turn's
    /// latest call still running, or goes back to thinking once none is.
    pub(crate) fn calls_changed(&mut self, running: Option<&WorkItem>, tick: u64) {
        match running {
            Some(item) => self.set(Doing::call(item.verb, &item.target), tick),
            None if matches!(self.doing, Doing::Call(..)) => self.set(Doing::Thinking, tick),
            None => {}
        }
    }

    /// The line as it reads at `tick`.
    pub(crate) fn line(&self, tick: u64) -> WorkingLine {
        let stalled = tick.saturating_sub(self.heard) >= STALL;
        let words = self.doing.words();
        let words = if stalled {
            format!("Still {words}")
        } else {
            capitalised(&words)
        };
        WorkingLine {
            words,
            age: tick.saturating_sub(self.since),
            stalled,
            seconds: tick.saturating_sub(self.began) / SECOND,
        }
    }

    /// A new phrase starts typing; the same one carries on.
    fn set(&mut self, doing: Doing, tick: u64) {
        if self.doing != doing {
            self.doing = doing;
            self.since = tick;
        }
    }
}

fn capitalised(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(verb: Verb, target: &str) -> WorkItem {
        WorkItem {
            call_id: target.into(),
            verb,
            target: target.into(),
            fact: None,
            failed: false,
        }
    }

    #[test]
    fn a_call_is_named_by_what_it_acts_on() {
        let mut a = Activity::new(0);
        let named = |a: &mut Activity, verb, target| {
            a.calls_changed(Some(&running(verb, target)), 1);
            a.line(1).words
        };
        assert_eq!(
            named(&mut a, Verb::Read, "src/gateway/router.rs"),
            "Reading router.rs"
        );
        assert_eq!(
            named(&mut a, Verb::Ran, "cargo test -q\n| tail"),
            "Running cargo test -q"
        );
        assert_eq!(
            named(&mut a, Verb::Searched, "tower::limit"),
            "Looking up tower::limit"
        );
        assert_eq!(named(&mut a, Verb::Changed, ""), "Changing");
    }

    #[test]
    fn a_running_call_keeps_the_line_until_none_is_left() {
        let mut a = Activity::new(0);
        a.calls_changed(Some(&running(Verb::Read, "b.rs")), 1);
        a.replying(2);
        a.thinking(2);
        assert_eq!(a.line(2).words, "Reading b.rs", "prose waits for the call");
        a.calls_changed(None, 3);
        assert_eq!(a.line(3).words, "Thinking");
        a.replying(4);
        a.calls_changed(None, 5);
        assert_eq!(a.line(5).words, "Writing a reply", "no call ended");
    }

    #[test]
    fn only_a_new_phrase_types_in_again() {
        let mut a = Activity::new(0);
        a.replying(10);
        a.replying(20);
        assert_eq!(a.line(25).age, 15);
        a.thinking(30);
        assert_eq!(a.line(30).age, 0);
    }

    #[test]
    fn thirty_quiet_seconds_read_as_a_stall_and_the_timer_keeps_counting() {
        let mut a = Activity::new(0);
        a.calls_changed(Some(&running(Verb::Ran, "cargo test")), 0);
        let quiet = STALL - 1;
        assert!(!a.line(quiet).stalled);
        let line = a.line(STALL);
        assert!(line.stalled);
        assert_eq!(line.words, "Still running cargo test");
        assert_eq!(line.seconds, 30);
        a.touch(STALL + 1);
        assert!(!a.line(STALL + 1).stalled);
    }
}
