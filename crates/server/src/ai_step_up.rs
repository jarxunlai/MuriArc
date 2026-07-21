use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use uuid::Uuid;

const DEFAULT_MAX_FAILURES: u32 = 5;
const DEFAULT_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(15 * 60);
const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy)]
pub(crate) struct AiStepUpPolicy {
    max_failures: u32,
    failure_window: Duration,
    cooldown: Duration,
    retention: Duration,
}

impl Default for AiStepUpPolicy {
    fn default() -> Self {
        Self {
            max_failures: DEFAULT_MAX_FAILURES,
            failure_window: DEFAULT_FAILURE_WINDOW,
            cooldown: DEFAULT_COOLDOWN,
            retention: DEFAULT_RETENTION,
        }
    }
}

impl AiStepUpPolicy {
    #[cfg(test)]
    pub(crate) fn for_test(
        max_failures: u32,
        failure_window: Duration,
        cooldown: Duration,
    ) -> Self {
        assert!(max_failures > 0);
        assert!(!failure_window.is_zero());
        assert!(!cooldown.is_zero());
        Self {
            max_failures,
            failure_window,
            cooldown,
            retention: failure_window.max(cooldown).saturating_mul(2),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AiStepUpRateLimiter {
    inner: Arc<Mutex<LimiterState>>,
    policy: AiStepUpPolicy,
}

impl Default for AiStepUpRateLimiter {
    fn default() -> Self {
        Self::new(AiStepUpPolicy::default())
    }
}

impl AiStepUpRateLimiter {
    pub(crate) fn new(policy: AiStepUpPolicy) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LimiterState::default())),
            policy,
        }
    }

    pub(crate) fn begin(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        now: Instant,
    ) -> Result<AiStepUpAttempt, AiStepUpLimit> {
        let key = StepUpSubject {
            user_id,
            session_id,
        };
        let mut inner = self.lock();
        inner.entries.retain(|_, state| {
            state.in_flight
                || now
                    .checked_duration_since(state.last_activity)
                    .is_none_or(|elapsed| elapsed < self.policy.retention)
        });

        let state = inner
            .entries
            .entry(key)
            .or_insert_with(|| AttemptState::new(now));
        if let Some(blocked_until) = state.blocked_until {
            if now < blocked_until {
                state.last_activity = now;
                return Err(AiStepUpLimit::Cooldown {
                    retry_after: blocked_until.duration_since(now),
                });
            }
            state.reset(now);
        } else if state
            .window_started_at
            .is_some_and(|started| elapsed(now, started) >= self.policy.failure_window)
        {
            state.reset(now);
        }

        if state.in_flight {
            return Err(AiStepUpLimit::InProgress);
        }
        state.in_flight = true;
        state.last_activity = now;
        drop(inner);

        Ok(AiStepUpAttempt {
            limiter: self.clone(),
            key,
            finished: false,
        })
    }

    fn record_failure(&self, key: StepUpSubject, now: Instant) -> AiStepUpFailure {
        let mut inner = self.lock();
        let state = inner
            .entries
            .entry(key)
            .or_insert_with(|| AttemptState::new(now));
        if state
            .window_started_at
            .is_none_or(|started| elapsed(now, started) >= self.policy.failure_window)
        {
            state.failed_attempts = 0;
            state.window_started_at = Some(now);
            state.blocked_until = None;
        }
        state.in_flight = false;
        state.last_activity = now;
        state.failed_attempts = state.failed_attempts.saturating_add(1);
        let blocked_for = if state.failed_attempts >= self.policy.max_failures {
            let blocked_until = now.checked_add(self.policy.cooldown).unwrap_or(now);
            state.blocked_until = Some(blocked_until);
            Some(self.policy.cooldown)
        } else {
            None
        };
        AiStepUpFailure {
            failed_attempts: state.failed_attempts,
            blocked_for,
        }
    }

    fn record_success(&self, key: StepUpSubject) {
        self.lock().entries.remove(&key);
    }

    fn cancel(&self, key: StepUpSubject, now: Instant) {
        if let Some(state) = self.lock().entries.get_mut(&key) {
            state.in_flight = false;
            state.last_activity = now;
        }
    }

    fn lock(&self) -> MutexGuard<'_, LimiterState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiStepUpLimit {
    InProgress,
    Cooldown { retry_after: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AiStepUpFailure {
    pub(crate) failed_attempts: u32,
    pub(crate) blocked_for: Option<Duration>,
}

#[derive(Debug)]
#[must_use = "a step-up attempt must be completed or explicitly cancelled"]
pub(crate) struct AiStepUpAttempt {
    limiter: AiStepUpRateLimiter,
    key: StepUpSubject,
    finished: bool,
}

impl AiStepUpAttempt {
    pub(crate) fn succeed(mut self) {
        self.finished = true;
        self.limiter.record_success(self.key);
    }

    pub(crate) fn fail(mut self, now: Instant) -> AiStepUpFailure {
        self.finished = true;
        self.limiter.record_failure(self.key, now)
    }

    pub(crate) fn cancel(mut self, now: Instant) {
        self.finished = true;
        self.limiter.cancel(self.key, now);
    }
}

impl Drop for AiStepUpAttempt {
    fn drop(&mut self) {
        if !self.finished {
            // Dropping an HTTP future does not cancel an already-running
            // spawn_blocking Argon2 task. Count abandonment as a failure so a
            // disconnect loop cannot bypass the per-session CPU bound.
            let failure = self.limiter.record_failure(self.key, Instant::now());
            tracing::warn!(
                target: "muriarc_server::security",
                security_event = "ai_step_up_verification_abandoned",
                user_id = %self.key.user_id,
                session_id = %self.key.session_id,
                failed_attempts = failure.failed_attempts,
                rate_limited = failure.blocked_for.is_some(),
                "AI reinforced approval password verification was abandoned"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StepUpSubject {
    user_id: Uuid,
    session_id: Uuid,
}

#[derive(Debug, Default)]
struct LimiterState {
    entries: HashMap<StepUpSubject, AttemptState>,
}

#[derive(Debug, Clone, Copy)]
struct AttemptState {
    failed_attempts: u32,
    window_started_at: Option<Instant>,
    blocked_until: Option<Instant>,
    in_flight: bool,
    last_activity: Instant,
}

impl AttemptState {
    fn new(now: Instant) -> Self {
        Self {
            failed_attempts: 0,
            window_started_at: None,
            blocked_until: None,
            in_flight: false,
            last_activity: now,
        }
    }

    fn reset(&mut self, now: Instant) {
        *self = Self::new(now);
    }
}

fn elapsed(now: Instant, earlier: Instant) -> Duration {
    now.checked_duration_since(earlier).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(max_failures: u32) -> AiStepUpRateLimiter {
        AiStepUpRateLimiter::new(AiStepUpPolicy::for_test(
            max_failures,
            Duration::from_secs(60),
            Duration::from_secs(30),
        ))
    }

    #[test]
    fn cooldown_is_deterministic_and_expires_at_the_boundary() {
        let limiter = limiter(2);
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let start = Instant::now();

        let first = limiter.begin(user_id, session_id, start).unwrap();
        assert_eq!(first.fail(start).failed_attempts, 1);
        let second_at = start + Duration::from_secs(1);
        let second = limiter.begin(user_id, session_id, second_at).unwrap();
        assert_eq!(
            second.fail(second_at),
            AiStepUpFailure {
                failed_attempts: 2,
                blocked_for: Some(Duration::from_secs(30)),
            }
        );
        assert_eq!(
            limiter
                .begin(user_id, session_id, second_at + Duration::from_secs(5))
                .unwrap_err(),
            AiStepUpLimit::Cooldown {
                retry_after: Duration::from_secs(25),
            }
        );

        let after_cooldown = second_at + Duration::from_secs(30);
        let allowed = limiter.begin(user_id, session_id, after_cooldown).unwrap();
        assert_eq!(allowed.fail(after_cooldown).failed_attempts, 1);
    }

    #[test]
    fn abandoned_verification_counts_toward_the_failure_limit() {
        let limiter = limiter(2);
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let start = Instant::now();

        drop(limiter.begin(user_id, session_id, start).unwrap());
        let second = limiter
            .begin(user_id, session_id, start + Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            second.fail(start + Duration::from_secs(1)),
            AiStepUpFailure {
                failed_attempts: 2,
                blocked_for: Some(Duration::from_secs(30)),
            }
        );
    }

    #[test]
    fn success_clears_failures_and_parallel_argon2_is_rejected() {
        let limiter = limiter(2);
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let start = Instant::now();

        let first = limiter.begin(user_id, session_id, start).unwrap();
        assert_eq!(
            limiter.begin(user_id, session_id, start).unwrap_err(),
            AiStepUpLimit::InProgress
        );
        assert_eq!(first.fail(start).failed_attempts, 1);

        limiter
            .begin(user_id, session_id, start + Duration::from_secs(1))
            .unwrap()
            .succeed();
        let fresh = limiter
            .begin(user_id, session_id, start + Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            fresh.fail(start + Duration::from_secs(2)).failed_attempts,
            1
        );
    }
}
