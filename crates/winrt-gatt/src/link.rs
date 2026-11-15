//! Connection state machine and reconnect back-off. Pure logic, no platform calls.

use std::time::Duration;

use crate::error::Error;

/// Where a connection attempt is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing in progress.
    Idle,
    /// Looking for the advertiser.
    Scanning,
    /// Opening the link.
    Connecting,
    /// Reading the GATT table.
    Discovering,
    /// Table known; reads, writes and subscriptions are usable.
    Ready,
    /// The link was lost; waiting to try again.
    Backoff,
    /// Shut down by the caller; terminal.
    Closed,
}

/// What happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A scan began.
    ScanStarted,
    /// An advertiser matched (or the address was supplied directly).
    Found,
    /// The link opened.
    Connected,
    /// The table was discovered.
    Discovered,
    /// A step failed or the link dropped.
    Failed,
    /// The back-off elapsed.
    RetryDue,
    /// The caller closed the client.
    Close,
}

impl State {
    /// Apply an event.
    ///
    /// # Errors
    /// [`Error::State`] if the event is not allowed in this state.
    pub fn on(self, event: Event) -> Result<Self, Error> {
        use Event as E;
        use State as S;
        Ok(match (self, event) {
            (S::Closed, _) => return Err(Error::State("closed".into())),
            (_, E::Close) => S::Closed,
            (S::Idle | S::Backoff, E::ScanStarted) => S::Scanning,
            (S::Idle | S::Scanning, E::Found) => S::Connecting,
            (S::Connecting, E::Connected) => S::Discovering,
            (S::Discovering, E::Discovered) => S::Ready,
            (S::Scanning | S::Connecting | S::Discovering | S::Ready, E::Failed) => S::Backoff,
            (S::Backoff, E::RetryDue) => S::Idle,
            (s, e) => return Err(Error::State(format!("{e:?} not allowed in {s:?}"))),
        })
    }
}

/// Exponential back-off with a ceiling and an optional attempt limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    /// Delay before the first retry.
    pub initial: Duration,
    /// Ceiling for any delay.
    pub max: Duration,
    /// Give up after this many failed attempts; `None` retries forever.
    pub max_attempts: Option<u32>,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(500),
            max: Duration::from_secs(30),
            max_attempts: None,
        }
    }
}

impl Backoff {
    /// The delay after `failures` consecutive failures (1 for the first), or `None` to give up.
    #[must_use]
    pub fn delay(&self, failures: u32) -> Option<Duration> {
        if self.max_attempts.is_some_and(|m| failures >= m) {
            return None;
        }
        let shift = failures.saturating_sub(1).min(31);
        let d = self.initial.saturating_mul(1u32 << shift);
        Some(d.min(self.max))
    }
}

/// Run `attempt` until it succeeds or the back-off gives up, calling `sleep` between attempts.
///
/// `attempt` receives the number of failures so far. `sleep` is a parameter so tests do not wait and so
/// the caller decides how to wait; it aborts the loop by returning `false`.
///
/// # Errors
/// The last error from `attempt` when the policy gives up or `sleep` returns `false`.
pub fn retry<T>(
    policy: &Backoff,
    mut attempt: impl FnMut(u32) -> Result<T, Error>,
    mut sleep: impl FnMut(Duration) -> bool,
) -> Result<T, Error> {
    let mut failures = 0u32;
    loop {
        match attempt(failures) {
            Ok(v) => return Ok(v),
            Err(e) => {
                failures += 1;
                match policy.delay(failures) {
                    Some(d) if sleep(d) => {}
                    _ => return Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_reaches_ready() {
        let mut s = State::Idle;
        for e in [
            Event::ScanStarted,
            Event::Found,
            Event::Connected,
            Event::Discovered,
        ] {
            s = s.on(e).unwrap();
        }
        assert_eq!(s, State::Ready);
    }

    #[test]
    fn a_drop_goes_to_backoff_then_rescans() {
        let s = State::Ready.on(Event::Failed).unwrap();
        assert_eq!(s, State::Backoff);
        assert_eq!(s.on(Event::ScanStarted).unwrap(), State::Scanning);
        assert_eq!(State::Backoff.on(Event::RetryDue).unwrap(), State::Idle);
    }

    #[test]
    fn nothing_leaves_closed_and_close_works_anywhere() {
        for s in [
            State::Idle,
            State::Scanning,
            State::Connecting,
            State::Discovering,
            State::Ready,
            State::Backoff,
        ] {
            assert_eq!(s.on(Event::Close).unwrap(), State::Closed);
        }
        assert!(State::Closed.on(Event::ScanStarted).is_err());
        assert!(State::Closed.on(Event::Close).is_err());
    }

    #[test]
    fn skipping_a_step_is_refused() {
        assert!(State::Idle.on(Event::Discovered).is_err());
        assert!(State::Scanning.on(Event::Connected).is_err());
        assert!(State::Ready.on(Event::Found).is_err());
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let b = Backoff {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(5),
            max_attempts: None,
        };
        let d: Vec<u64> = (1..=5).map(|n| b.delay(n).unwrap().as_secs()).collect();
        assert_eq!(d, [1, 2, 4, 5, 5]);
        assert_eq!(b.delay(1000).unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn backoff_gives_up_at_the_limit() {
        let b = Backoff {
            max_attempts: Some(3),
            ..Backoff::default()
        };
        assert!(b.delay(2).is_some());
        assert!(b.delay(3).is_none());
    }

    #[test]
    fn retry_stops_on_success() {
        let b = Backoff {
            initial: Duration::ZERO,
            max: Duration::ZERO,
            max_attempts: Some(3),
        };
        let mut sleeps = 0;
        let r = retry(
            &b,
            |n| {
                if n == 2 {
                    Ok(n)
                } else {
                    Err(Error::ScanTimeout)
                }
            },
            |_| {
                sleeps += 1;
                true
            },
        );
        assert_eq!(r.unwrap(), 2);
        assert_eq!(sleeps, 2);
    }

    #[test]
    fn retry_reports_the_last_error_on_giving_up() {
        let b = Backoff {
            initial: Duration::ZERO,
            max: Duration::ZERO,
            max_attempts: Some(3),
        };
        let mut calls = 0;
        let r: Result<(), _> = retry(
            &b,
            |_| {
                calls += 1;
                Err(Error::Status(calls.to_string()))
            },
            |_| true,
        );
        assert_eq!(r.unwrap_err(), Error::Status("3".into()));
        assert_eq!(calls, 3);
    }

    #[test]
    fn retry_stops_when_sleep_declines() {
        let b = Backoff::default();
        let mut calls = 0;
        let r: Result<(), _> = retry(
            &b,
            |_| {
                calls += 1;
                Err(Error::ScanTimeout)
            },
            |_| false,
        );
        assert!(r.is_err());
        assert_eq!(calls, 1);
    }
}
