//! Deterministic lifecycle policy shared by the production supervisor and tests.

use std::{
    future::Future,
    time::{Duration, Instant},
};

use crate::mpd::{FailureClass, MpdError};

pub const HEALTHY_RESET_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupervisorState {
    Reconnecting,
    Connected,
    ShuttingDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureDisposition {
    KeepPair,
    TearDownPair,
}

pub fn classify_failure(error: &MpdError) -> FailureDisposition {
    match error.class() {
        FailureClass::CommandAck => FailureDisposition::KeepPair,
        FailureClass::ConnectionFault => FailureDisposition::TearDownPair,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    Connecting,
    Connected,
    ClearRequested { reason: String },
    ClearAcknowledged { reason: String },
    PairFault { reason: String },
    Backoff { attempt: u32, delay: Duration },
    HealthyBackoffReset,
    ShuttingDown,
}

pub trait LifecycleLog: Send + Sync {
    fn record(&self, event: &LifecycleEvent);
}

#[derive(Debug, Default)]
pub struct StderrLifecycleLog;

impl LifecycleLog for StderrLifecycleLog {
    fn record(&self, event: &LifecycleEvent) {
        eprintln!("lifecycle: {event:?}");
    }
}

pub trait LifecycleClock: Clone + Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLifecycleClock;

impl LifecycleClock for SystemLifecycleClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub trait JitterRng: Send + 'static {
    /// Return a deterministic sample in the inclusive range 0..=u64::MAX.
    fn sample(&mut self) -> u64;
}

#[derive(Debug)]
pub struct SystemJitter {
    state: u64,
}

impl Default for SystemJitter {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |duration| duration.as_nanos() as u64);
        Self { state: seed | 1 }
    }
}

impl JitterRng for SystemJitter {
    fn sample(&mut self) -> u64 {
        // xorshift64*: sufficient for retry jitter and dependency-free.
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state = self.state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        self.state
    }
}

pub trait LifecycleSleeper: Clone + Send + Sync + 'static {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TokioSleeper;

impl LifecycleSleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackoffDelay {
    pub attempt: u32,
    pub nominal: Duration,
    pub actual: Duration,
}

#[derive(Debug)]
pub struct BackoffPolicy<C, R> {
    clock: C,
    rng: R,
    step: u32,
    connected_since: Option<Instant>,
}

impl<C, R> BackoffPolicy<C, R>
where
    C: LifecycleClock,
    R: JitterRng,
{
    pub fn new(clock: C, rng: R) -> Self {
        Self {
            clock,
            rng,
            step: 0,
            connected_since: None,
        }
    }

    pub fn entered_connected(&mut self) {
        self.connected_since = Some(self.clock.now());
    }

    /// Returns true only when this refresh reset an escalated schedule.
    pub fn successful_refresh(&mut self) -> bool {
        let healthy = self.connected_since.is_some_and(|since| {
            self.clock
                .now()
                .checked_duration_since(since)
                .unwrap_or_default()
                >= HEALTHY_RESET_AFTER
        });
        if healthy && self.step != 0 {
            self.step = 0;
            return true;
        }
        false
    }

    pub fn disconnected(&mut self) {
        self.connected_since = None;
    }

    pub fn next_delay(&mut self) -> BackoffDelay {
        let attempt = self.step.saturating_add(1);
        let nominal_secs = match self.step {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            _ => 30,
        };
        self.step = self.step.saturating_add(1);

        let nominal = Duration::from_secs(nominal_secs);
        let half_nanos = nominal.as_nanos() / 2;
        let jitter_nanos = (half_nanos * u128::from(self.rng.sample())) / u128::from(u64::MAX);
        let actual_nanos = half_nanos + jitter_nanos;
        let actual = Duration::from_nanos(
            u64::try_from(actual_nanos).expect("30-second backoff fits in u64 nanoseconds"),
        );
        BackoffDelay {
            attempt,
            nominal,
            actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::mpd::CommandFailure;

    #[derive(Clone)]
    struct FakeClock(Arc<Mutex<Instant>>);

    impl FakeClock {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Instant::now())))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().unwrap();
            *now += duration;
        }
    }

    impl LifecycleClock for FakeClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    struct FakeRng(u64);

    impl JitterRng for FakeRng {
        fn sample(&mut self) -> u64 {
            self.0
        }
    }

    #[test]
    fn schedule_is_capped_and_equal_jitter_stays_within_ruled_bounds() {
        let clock = FakeClock::new();
        let mut low = BackoffPolicy::new(clock.clone(), FakeRng(0));
        let mut high = BackoffPolicy::new(clock, FakeRng(u64::MAX));
        let expected = [1, 2, 4, 8, 16, 30, 30];

        for nominal_secs in expected {
            let low_delay = low.next_delay();
            let high_delay = high.next_delay();
            assert_eq!(low_delay.nominal, Duration::from_secs(nominal_secs));
            assert_eq!(high_delay.nominal, Duration::from_secs(nominal_secs));
            assert_eq!(low_delay.actual, Duration::from_millis(nominal_secs * 500));
            assert_eq!(high_delay.actual, Duration::from_secs(nominal_secs));
            assert!(high_delay.actual <= Duration::from_secs(30));
        }
    }

    #[test]
    fn handshake_and_immediate_drop_do_not_reset_but_healthy_refresh_does() {
        let clock = FakeClock::new();
        let mut policy = BackoffPolicy::new(clock.clone(), FakeRng(0));
        assert_eq!(policy.next_delay().nominal, Duration::from_secs(1));
        policy.entered_connected();
        assert!(!policy.successful_refresh());
        policy.disconnected();
        assert_eq!(policy.next_delay().nominal, Duration::from_secs(2));

        policy.entered_connected();
        clock.advance(Duration::from_secs(29));
        assert!(!policy.successful_refresh());
        clock.advance(Duration::from_secs(1));
        assert!(policy.successful_refresh());
        policy.disconnected();
        assert_eq!(policy.next_delay().nominal, Duration::from_secs(1));
    }

    #[test]
    fn fake_pair_ack_stays_connected_while_mapping_fault_tears_down() {
        let ack = MpdError::Command(CommandFailure {
            code: 5,
            command_index: 0,
            command: Some("next".into()),
            message: "No next song".into(),
        });
        let malformed = MpdError::MissingField("state");

        assert_eq!(classify_failure(&ack), FailureDisposition::KeepPair);
        assert_eq!(
            classify_failure(&malformed),
            FailureDisposition::TearDownPair
        );

        for fault in [
            MpdError::UnexpectedFrameCount {
                expected: 2,
                actual: 1,
            },
            MpdError::UnexpectedFields { actual: 1 },
            MpdError::InvalidField {
                field: "state",
                value: "broken".into(),
            },
            MpdError::SnapshotCoherenceExhausted,
        ] {
            assert_eq!(classify_failure(&fault), FailureDisposition::TearDownPair);
        }
    }
}
