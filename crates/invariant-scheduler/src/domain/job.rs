use crate::domain::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(String);

impl JobId {
    pub fn new(id: impl Into<String>) -> Result<Self, DomainError> {
        let value = id.into();
        if value.is_empty() {
            return Err(DomainError::EmptyJobId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetRef(String);

impl TargetRef {
    pub fn new(target: impl Into<String>) -> Result<Self, DomainError> {
        let value = target.into();
        if value.is_empty() {
            return Err(DomainError::EmptyTargetRef);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptNumber(u32);

impl AttemptNumber {
    pub const fn zero() -> Self {
        Self(0)
    }

    #[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    mod job_id {
        use super::*;

        #[test]
        fn rejects_empty() {
            assert_eq!(JobId::new(""), Err(DomainError::EmptyJobId));
        }

        #[test]
        fn preserves_value() {
            assert_eq!(JobId::new("job-1").unwrap().as_str(), "job-1");
        }

        #[test]
        fn orders_lexicographically() {
            assert!(JobId::new("a").unwrap() < JobId::new("b").unwrap());
        }
    }

    mod target_ref {
        use super::*;

        #[test]
        fn rejects_empty() {
            assert_eq!(TargetRef::new(""), Err(DomainError::EmptyTargetRef));
        }

        #[test]
        fn preserves_value() {
            assert_eq!(
                TargetRef::new("component:greet").unwrap().as_str(),
                "component:greet"
            );
        }
    }

    mod attempt_number {
        use super::*;

        #[test]
        fn starts_at_zero() {
            assert_eq!(AttemptNumber::zero().value(), 0);
        }

        #[test]
        fn increments() {
            assert_eq!(AttemptNumber::zero().checked_next().unwrap().value(), 1);
        }

        #[test]
        fn rejects_overflow() {
            assert_eq!(
                AttemptNumber(u32::MAX).checked_next(),
                Err(DomainError::AttemptOverflow)
            );
        }
    }
}
