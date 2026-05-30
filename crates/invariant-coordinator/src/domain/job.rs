use crate::domain::{
    error::DomainError,
    lease::{JobLease, LeaseCredential, LeaseDuration, LeaseEpoch, LeaseToken},
    time::SchedulerTime,
    worker::WorkerId,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JobId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TargetRef(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptNumber(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Leased,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    id: JobId,
    target: TargetRef,
    status: JobStatus,
    current_lease: Option<JobLease>,
    attempt: AttemptNumber,
    last_lease_epoch: LeaseEpoch,
}

impl JobId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DomainError::EmptyJobId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TargetRef {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DomainError::EmptyTargetRef);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AttemptNumber {
    pub const fn zero() -> Self {
        Self(0)
    }

    pub const fn value(self) -> u32 {
        self.0
    }

    pub fn checked_next(self) -> Result<Self, DomainError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(DomainError::AttemptOverflow)
    }
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl Job {
    pub fn new(id: JobId, target: TargetRef) -> Self {
        Self {
            id,
            target,
            status: JobStatus::Queued,
            current_lease: None,
            attempt: AttemptNumber::zero(),
            last_lease_epoch: LeaseEpoch::zero(),
        }
    }

    pub fn id(&self) -> &JobId {
        &self.id
    }

    pub fn target(&self) -> &TargetRef {
        &self.target
    }

    pub fn status(&self) -> JobStatus {
        self.status
    }

    pub fn current_lease(&self) -> Option<&JobLease> {
        self.current_lease.as_ref()
    }

    pub fn attempt(&self) -> AttemptNumber {
        self.attempt
    }

    pub fn last_lease_epoch(&self) -> LeaseEpoch {
        self.last_lease_epoch
    }

    pub fn claim(
        &mut self,
        holder: WorkerId,
        token: LeaseToken,
        now: SchedulerTime,
        duration: LeaseDuration,
    ) -> Result<JobLease, DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::JobTerminal);
        }

        if let Some(current) = &self.current_lease
            && !current.is_expired_at(now)
        {
            return Err(DomainError::JobAlreadyLeased);
        }

        let next_epoch = self.last_lease_epoch.checked_next()?;
        let next_attempt = self.attempt.checked_next()?;
        let lease = JobLease::new(holder, token, next_epoch, now, duration)?;

        self.current_lease = Some(lease.clone());
        self.last_lease_epoch = next_epoch;
        self.attempt = next_attempt;
        self.status = JobStatus::Leased;

        Ok(lease)
    }

    pub fn renew_lease(
        &mut self,
        credential: &LeaseCredential,
        now: SchedulerTime,
        duration: LeaseDuration,
    ) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::JobTerminal);
        }

        let lease = self
            .current_lease
            .as_mut()
            .ok_or(DomainError::JobNotLeased)?;

        lease.renew(credential, now, duration)
    }

    pub fn complete(
        &mut self,
        credential: &LeaseCredential,
        now: SchedulerTime,
    ) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::JobTerminal);
        }

        let lease = self
            .current_lease
            .as_ref()
            .ok_or(DomainError::JobNotLeased)?;

        lease.require_current(credential, now)?;

        self.status = JobStatus::Completed;
        self.current_lease = None;

        Ok(())
    }

    pub fn release(
        &mut self,
        credential: &LeaseCredential,
        now: SchedulerTime,
    ) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(DomainError::JobTerminal);
        }

        let lease = self
            .current_lease
            .as_ref()
            .ok_or(DomainError::JobNotLeased)?;

        lease.require_current(credential, now)?;

        self.current_lease = None;
        self.status = JobStatus::Queued;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_id(value: &str) -> JobId {
        JobId::new(value).unwrap()
    }

    fn target(value: &str) -> TargetRef {
        TargetRef::new(value).unwrap()
    }

    fn worker(value: &str) -> WorkerId {
        WorkerId::new(value).unwrap()
    }

    fn token(value: &str) -> LeaseToken {
        LeaseToken::new(value).unwrap()
    }

    fn duration(value: u64) -> LeaseDuration {
        LeaseDuration::from_millis(value).unwrap()
    }

    fn new_job() -> Job {
        Job::new(job_id("job-1"), target("target-1"))
    }

    #[test]
    fn new_job_starts_queued_without_lease() {
        let job = new_job();

        assert_eq!(job.status(), JobStatus::Queued);
        assert_eq!(job.current_lease(), None);
        assert_eq!(job.attempt(), AttemptNumber::zero());
        assert_eq!(job.last_lease_epoch(), LeaseEpoch::zero());
    }

    #[test]
    fn constructors_should_reject_empty_ids() {
        assert_eq!(JobId::new(""), Err(DomainError::EmptyJobId));
        assert_eq!(TargetRef::new(""), Err(DomainError::EmptyTargetRef));
    }

    #[test]
    fn claim_queued_job_creates_lease_epoch_one_attempt_one() {
        let mut job = new_job();
        let now = SchedulerTime::from_millis_since_epoch(10);

        let lease = job
            .claim(worker("worker-1"), token("token-1"), now, duration(5))
            .unwrap();

        assert_eq!(job.status(), JobStatus::Leased);
        assert_eq!(job.attempt(), AttemptNumber(1));
        assert_eq!(job.last_lease_epoch(), LeaseEpoch::from_value(1));
        assert_eq!(lease.epoch(), LeaseEpoch::from_value(1));
        assert_eq!(lease.holder().as_str(), "worker-1");
        assert_eq!(
            lease.expires_at(),
            SchedulerTime::from_millis_since_epoch(15)
        );
        assert_eq!(job.current_lease(), Some(&lease));
    }

    #[test]
    fn claim_before_expiry_is_rejected() {
        let mut job = new_job();
        let now = SchedulerTime::from_millis_since_epoch(10);
        job.claim(worker("worker-1"), token("token-1"), now, duration(5))
            .unwrap();

        let result = job.claim(
            worker("worker-2"),
            token("token-2"),
            SchedulerTime::from_millis_since_epoch(14),
            duration(5),
        );

        assert_eq!(result, Err(DomainError::JobAlreadyLeased));
    }

    #[test]
    fn renew_valid_lease_keeps_token_epoch_and_attempt() {
        let mut job = new_job();
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = job
            .claim(worker("worker-1"), token("token-1"), now, duration(5))
            .unwrap();
        let credential = lease.credential();

        let result = job.renew_lease(
            &credential,
            SchedulerTime::from_millis_since_epoch(12),
            duration(10),
        );

        assert_eq!(result, Ok(()));
        assert_eq!(job.attempt(), AttemptNumber(1));
        let renewed = job.current_lease().unwrap();
        assert_eq!(renewed.token().as_str(), "token-1");
        assert_eq!(renewed.epoch(), LeaseEpoch::from_value(1));
        assert_eq!(
            renewed.expires_at(),
            SchedulerTime::from_millis_since_epoch(22)
        );
    }

    #[test]
    fn renew_with_wrong_holder_is_rejected() {
        let mut job = new_job();
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = job
            .claim(worker("worker-1"), token("token-1"), now, duration(5))
            .unwrap();
        let credential =
            LeaseCredential::new(worker("worker-2"), lease.token().clone(), lease.epoch());

        let result = job.renew_lease(&credential, now, duration(5));

        assert_eq!(result, Err(DomainError::LeaseHeldByDifferentWorker));
    }

    #[test]
    fn renew_with_wrong_token_is_rejected() {
        let mut job = new_job();
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = job
            .claim(worker("worker-1"), token("token-1"), now, duration(5))
            .unwrap();
        let credential =
            LeaseCredential::new(lease.holder().clone(), token("token-2"), lease.epoch());

        let result = job.renew_lease(&credential, now, duration(5));

        assert_eq!(result, Err(DomainError::StaleLease));
    }

    #[test]
    fn renew_with_wrong_epoch_is_rejected() {
        let mut job = new_job();
        let now = SchedulerTime::from_millis_since_epoch(10);
        let lease = job
            .claim(worker("worker-1"), token("token-1"), now, duration(5))
            .unwrap();
        let credential = LeaseCredential::new(
            lease.holder().clone(),
            lease.token().clone(),
            LeaseEpoch::from_value(2),
        );

        let result = job.renew_lease(&credential, now, duration(5));

        assert_eq!(result, Err(DomainError::StaleLease));
    }

    #[test]
    fn expired_lease_can_be_reclaimed_with_new_epoch() {
        let mut job = new_job();
        let lease = job
            .claim(
                worker("worker-1"),
                token("token-1"),
                SchedulerTime::from_millis_since_epoch(10),
                duration(5),
            )
            .unwrap();

        let new_lease = job
            .claim(
                worker("worker-2"),
                token("token-2"),
                SchedulerTime::from_millis_since_epoch(15),
                duration(5),
            )
            .unwrap();

        assert_eq!(lease.epoch(), LeaseEpoch::from_value(1));
        assert_eq!(new_lease.epoch(), LeaseEpoch::from_value(2));
        assert_eq!(new_lease.holder().as_str(), "worker-2");
        assert_eq!(job.attempt(), AttemptNumber(2));
    }

    #[test]
    fn stale_complete_after_reclaim_is_rejected() {
        let mut job = new_job();
        let old_lease = job
            .claim(
                worker("worker-1"),
                token("token-1"),
                SchedulerTime::from_millis_since_epoch(10),
                duration(5),
            )
            .unwrap();
        job.claim(
            worker("worker-2"),
            token("token-2"),
            SchedulerTime::from_millis_since_epoch(15),
            duration(5),
        )
        .unwrap();

        let result = job.complete(
            &old_lease.credential(),
            SchedulerTime::from_millis_since_epoch(16),
        );

        assert_eq!(result, Err(DomainError::LeaseHeldByDifferentWorker));
        assert_eq!(job.status(), JobStatus::Leased);
        assert_eq!(job.current_lease().unwrap().holder().as_str(), "worker-2");
    }

    #[test]
    fn expired_lease_cannot_complete_even_without_reclaim() {
        let mut job = new_job();
        let lease = job
            .claim(
                worker("worker-1"),
                token("token-1"),
                SchedulerTime::from_millis_since_epoch(10),
                duration(5),
            )
            .unwrap();

        let result = job.complete(
            &lease.credential(),
            SchedulerTime::from_millis_since_epoch(15),
        );

        assert_eq!(result, Err(DomainError::LeaseExpired));
        assert_eq!(job.status(), JobStatus::Leased);
    }

    #[test]
    fn valid_complete_marks_completed_and_clears_lease() {
        let mut job = new_job();
        let lease = job
            .claim(
                worker("worker-1"),
                token("token-1"),
                SchedulerTime::from_millis_since_epoch(10),
                duration(5),
            )
            .unwrap();

        let result = job.complete(
            &lease.credential(),
            SchedulerTime::from_millis_since_epoch(14),
        );

        assert_eq!(result, Ok(()));
        assert_eq!(job.status(), JobStatus::Completed);
        assert_eq!(job.current_lease(), None);
    }

    #[test]
    fn terminal_job_cannot_be_claimed() {
        let mut job = new_job();
        let lease = job
            .claim(
                worker("worker-1"),
                token("token-1"),
                SchedulerTime::from_millis_since_epoch(10),
                duration(5),
            )
            .unwrap();
        job.complete(
            &lease.credential(),
            SchedulerTime::from_millis_since_epoch(14),
        )
        .unwrap();

        let result = job.claim(
            worker("worker-2"),
            token("token-2"),
            SchedulerTime::from_millis_since_epoch(15),
            duration(5),
        );

        assert_eq!(result, Err(DomainError::JobTerminal));
    }

    #[test]
    fn valid_release_returns_to_queued_and_preserves_epoch() {
        let mut job = new_job();
        let lease = job
            .claim(
                worker("worker-1"),
                token("token-1"),
                SchedulerTime::from_millis_since_epoch(10),
                duration(5),
            )
            .unwrap();

        let result = job.release(
            &lease.credential(),
            SchedulerTime::from_millis_since_epoch(14),
        );

        assert_eq!(result, Ok(()));
        assert_eq!(job.status(), JobStatus::Queued);
        assert_eq!(job.current_lease(), None);
        assert_eq!(job.last_lease_epoch(), LeaseEpoch::from_value(1));
    }
}
