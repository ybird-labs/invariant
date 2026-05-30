use std::time::Duration;

use crate::domain::{error::DomainError, time::SchedulerTime, worker::WorkerId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeaseToken(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseEpoch(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseDuration(Duration);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseCredential {
    holder: WorkerId,
    token: LeaseToken,
    epoch: LeaseEpoch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobLease {
    holder: WorkerId,
    token: LeaseToken,
    epoch: LeaseEpoch,
    acquired_at: SchedulerTime,
    renewed_at: SchedulerTime,
    expires_at: SchedulerTime,
}

impl LeaseToken {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DomainError::EmptyLeaseToken);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl LeaseEpoch {
    pub const fn zero() -> Self {
        Self(0)
    }

    pub const fn from_value(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn checked_next(self) -> Result<Self, DomainError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(DomainError::LeaseEpochOverflow)
    }
}

impl LeaseDuration {
    pub fn from_duration(value: Duration) -> Result<Self, DomainError> {
        if value.is_zero() {
            return Err(DomainError::ZeroLeaseDuration);
        }
        Ok(Self(value))
    }

    pub fn from_millis(value: u64) -> Result<Self, DomainError> {
        Self::from_duration(Duration::from_millis(value))
    }

    pub fn from_secs(value: u64) -> Result<Self, DomainError> {
        Self::from_duration(Duration::from_secs(value))
    }

    pub fn as_duration(self) -> Duration {
        self.0
    }
}

impl LeaseCredential {
    pub fn new(holder: WorkerId, token: LeaseToken, epoch: LeaseEpoch) -> Self {
        Self {
            holder,
            token,
            epoch,
        }
    }

    pub fn holder(&self) -> &WorkerId {
        &self.holder
    }

    pub fn token(&self) -> &LeaseToken {
        &self.token
    }

    pub fn epoch(&self) -> LeaseEpoch {
        self.epoch
    }
}

impl JobLease {
    pub fn new(
        holder: WorkerId,
        token: LeaseToken,
        epoch: LeaseEpoch,
        now: SchedulerTime,
        duration: LeaseDuration,
    ) -> Result<Self, DomainError> {
        let expires_at = now.checked_add_duration(duration.as_duration())?;
        Ok(Self {
            holder,
            token,
            epoch,
            acquired_at: now,
            renewed_at: now,
            expires_at,
        })
    }

    pub fn holder(&self) -> &WorkerId {
        &self.holder
    }

    pub fn token(&self) -> &LeaseToken {
        &self.token
    }

    pub fn epoch(&self) -> LeaseEpoch {
        self.epoch
    }

    pub fn acquired_at(&self) -> SchedulerTime {
        self.acquired_at
    }

    pub fn renewed_at(&self) -> SchedulerTime {
        self.renewed_at
    }

    pub fn expires_at(&self) -> SchedulerTime {
        self.expires_at
    }

    pub fn credential(&self) -> LeaseCredential {
        LeaseCredential::new(self.holder.clone(), self.token.clone(), self.epoch)
    }

    pub fn is_expired_at(&self, now: SchedulerTime) -> bool {
        now >= self.expires_at
    }

    pub fn require_current(
        &self,
        credential: &LeaseCredential,
        now: SchedulerTime,
    ) -> Result<(), DomainError> {
        if self.holder != *credential.holder() {
            return Err(DomainError::LeaseHeldByDifferentWorker);
        }
        if self.token != *credential.token() || self.epoch != credential.epoch() {
            return Err(DomainError::StaleLease);
        }
        if self.is_expired_at(now) {
            return Err(DomainError::LeaseExpired);
        }
        Ok(())
    }

    pub fn renew(
        &mut self,
        credential: &LeaseCredential,
        now: SchedulerTime,
        duration: LeaseDuration,
    ) -> Result<(), DomainError> {
        self.require_current(credential, now)?;
        self.renewed_at = now;
        self.expires_at = now.checked_add_duration(duration.as_duration())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(value: &str) -> WorkerId {
        WorkerId::new(value).unwrap()
    }

    fn token(value: &str) -> LeaseToken {
        LeaseToken::new(value).unwrap()
    }

    fn duration(value: u64) -> LeaseDuration {
        LeaseDuration::from_millis(value).unwrap()
    }

    #[test]
    fn lease_duration_should_reject_zero() {
        let result = LeaseDuration::from_millis(0);

        assert_eq!(result, Err(DomainError::ZeroLeaseDuration));
    }

    #[test]
    fn lease_epoch_should_increment_from_zero() {
        let result = LeaseEpoch::zero().checked_next();

        assert_eq!(result, Ok(LeaseEpoch(1)));
    }

    #[test]
    fn lease_epoch_should_reject_overflow() {
        let result = LeaseEpoch(u64::MAX).checked_next();

        assert_eq!(result, Err(DomainError::LeaseEpochOverflow));
    }

    #[test]
    fn new_should_compute_expiry_and_initial_renewed_at() {
        let now = SchedulerTime::from_millis_since_epoch(10);

        let lease = JobLease::new(
            worker("worker-1"),
            token("token-1"),
            LeaseEpoch(1),
            now,
            duration(5),
        )
        .unwrap();

        assert_eq!(lease.acquired_at(), now);
        assert_eq!(lease.renewed_at(), now);
        assert_eq!(
            lease.expires_at(),
            SchedulerTime::from_millis_since_epoch(15)
        );
    }

    #[test]
    fn require_current_should_reject_wrong_holder() {
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = JobLease::new(
            worker("worker-1"),
            token("token-1"),
            LeaseEpoch(1),
            now,
            duration(5),
        )
        .unwrap();
        let credential = LeaseCredential::new(worker("worker-2"), token("token-1"), LeaseEpoch(1));

        let result = lease.require_current(&credential, now);

        assert_eq!(result, Err(DomainError::LeaseHeldByDifferentWorker));
    }

    #[test]
    fn require_current_should_reject_wrong_token() {
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = JobLease::new(
            worker("worker-1"),
            token("token-1"),
            LeaseEpoch(1),
            now,
            duration(5),
        )
        .unwrap();
        let credential = LeaseCredential::new(worker("worker-1"), token("token-2"), LeaseEpoch(1));

        let result = lease.require_current(&credential, now);

        assert_eq!(result, Err(DomainError::StaleLease));
    }

    #[test]
    fn require_current_should_reject_expired_lease_at_deadline() {
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = JobLease::new(
            worker("worker-1"),
            token("token-1"),
            LeaseEpoch(1),
            now,
            duration(5),
        )
        .unwrap();
        let credential = lease.credential();

        let result = lease.require_current(&credential, SchedulerTime::from_millis_since_epoch(15));

        assert_eq!(result, Err(DomainError::LeaseExpired));
    }

    #[test]
    fn renew_should_keep_token_and_epoch_and_extend_expiry() {
        let now = SchedulerTime::from_millis_since_epoch(10);
        let mut lease = JobLease::new(
            worker("worker-1"),
            token("token-1"),
            LeaseEpoch(1),
            now,
            duration(5),
        )
        .unwrap();
        let credential = lease.credential();
        let renewed_at = SchedulerTime::from_millis_since_epoch(12);

        let result = lease.renew(&credential, renewed_at, duration(10));

        assert_eq!(result, Ok(()));
        assert_eq!(lease.token().as_str(), "token-1");
        assert_eq!(lease.epoch(), LeaseEpoch(1));
        assert_eq!(lease.renewed_at(), renewed_at);
        assert_eq!(
            lease.expires_at(),
            SchedulerTime::from_millis_since_epoch(22)
        );
    }
}
