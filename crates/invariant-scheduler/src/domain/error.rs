use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainError {
    EmptyJobId,
    EmptyTargetRef,
    TimeOverflow,
    AttemptOverflow,
    JobTerminal,
    IllegalTransition,
    QueueFull,
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyJobId => "job id cannot be empty",
            Self::EmptyTargetRef => "target ref cannot be empty",
            Self::TimeOverflow => "scheduler time overflow",
            Self::AttemptOverflow => "attempt number overflow",
            Self::JobTerminal => "job is terminal",
            Self::IllegalTransition => "illegal job state transition",
            Self::QueueFull => "ready queue is full",
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
            (DomainError::TimeOverflow, "scheduler time overflow"),
            (DomainError::AttemptOverflow, "attempt number overflow"),
            (DomainError::JobTerminal, "job is terminal"),
            (
                DomainError::IllegalTransition,
                "illegal job state transition",
            ),
            (DomainError::QueueFull, "ready queue is full"),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}
