use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainError {
    EmptyJobId,
    EmptyTargetRef,
    EmptyWorkerId,
    EmptyLeaseToken,
    ZeroLeaseDuration,
    TimeOverflow,
    LeaseEpochOverflow,
    AttemptOverflow,
    JobAlreadyLeased,
    JobNotLeased,
    JobTerminal,
    LeaseHeldByDifferentWorker,
    StaleLease,
    LeaseExpired,
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyJobId => "job id cannot be empty",
            Self::EmptyTargetRef => "target ref cannot be empty",
            Self::EmptyWorkerId => "worker id cannot be empty",
            Self::EmptyLeaseToken => "lease token cannot be empty",
            Self::ZeroLeaseDuration => "lease duration must be greater than zero",
            Self::TimeOverflow => "scheduler time overflow",
            Self::LeaseEpochOverflow => "lease epoch overflow",
            Self::AttemptOverflow => "attempt number overflow",
            Self::JobAlreadyLeased => "job already has an active lease",
            Self::JobNotLeased => "job is not leased",
            Self::JobTerminal => "job is terminal",
            Self::LeaseHeldByDifferentWorker => "lease is held by a different worker",
            Self::StaleLease => "lease credential is stale",
            Self::LeaseExpired => "lease has expired",
        };
        f.write_str(message)
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_should_render_stable_messages_for_all_domain_errors() {
        let cases = [
            (DomainError::EmptyJobId, "job id cannot be empty"),
            (DomainError::EmptyTargetRef, "target ref cannot be empty"),
            (DomainError::EmptyWorkerId, "worker id cannot be empty"),
            (DomainError::EmptyLeaseToken, "lease token cannot be empty"),
            (
                DomainError::ZeroLeaseDuration,
                "lease duration must be greater than zero",
            ),
            (DomainError::TimeOverflow, "scheduler time overflow"),
            (DomainError::LeaseEpochOverflow, "lease epoch overflow"),
            (DomainError::AttemptOverflow, "attempt number overflow"),
            (
                DomainError::JobAlreadyLeased,
                "job already has an active lease",
            ),
            (DomainError::JobNotLeased, "job is not leased"),
            (DomainError::JobTerminal, "job is terminal"),
            (
                DomainError::LeaseHeldByDifferentWorker,
                "lease is held by a different worker",
            ),
            (DomainError::StaleLease, "lease credential is stale"),
            (DomainError::LeaseExpired, "lease has expired"),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}
