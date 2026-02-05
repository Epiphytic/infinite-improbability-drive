//! Exponential backoff utility for polling intervals.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Exponential backoff with configurable min/max.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExponentialBackoff {
    initial: Duration,
    max: Duration,
    current: Duration,
}

impl ExponentialBackoff {
    /// Creates a new backoff starting at `initial`, capping at `max`.
    pub fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            current: initial,
        }
    }

    /// Returns the current backoff duration.
    pub fn current(&self) -> Duration {
        self.current
    }

    /// Advances to the next backoff interval (doubles, capped at max).
    pub fn next(&mut self) {
        self.current = (self.current * 2).min(self.max);
    }

    /// Resets backoff to initial value.
    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn backoff_starts_at_initial() {
        let backoff = ExponentialBackoff::new(Duration::from_secs(5), Duration::from_secs(300));
        assert_eq!(backoff.current(), Duration::from_secs(5));
    }

    #[test]
    fn backoff_doubles_on_next() {
        let mut backoff = ExponentialBackoff::new(Duration::from_secs(5), Duration::from_secs(300));
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_secs(10));
        backoff.next();
        assert_eq!(backoff.current(), Duration::from_secs(20));
    }

    #[test]
    fn backoff_caps_at_max() {
        let mut backoff =
            ExponentialBackoff::new(Duration::from_secs(100), Duration::from_secs(300));
        backoff.next(); // 200
        backoff.next(); // 400 -> capped to 300
        assert_eq!(backoff.current(), Duration::from_secs(300));
    }

    #[test]
    fn backoff_resets_to_initial() {
        let mut backoff = ExponentialBackoff::new(Duration::from_secs(5), Duration::from_secs(300));
        backoff.next();
        backoff.next();
        backoff.reset();
        assert_eq!(backoff.current(), Duration::from_secs(5));
    }
}
