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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_rejects_empty() {
        assert_eq!(JobId::new(""), Err(DomainError::EmptyJobId));
    }

    #[test]
    fn job_id_preserves_value() {
        assert_eq!(JobId::new("job-1").unwrap().as_str(), "job-1");
    }

    #[test]
    fn job_id_orders_lexicographically() {
        assert!(JobId::new("a").unwrap() < JobId::new("b").unwrap());
    }

    #[test]
    fn target_ref_rejects_empty() {
        assert_eq!(TargetRef::new(""), Err(DomainError::EmptyTargetRef));
    }

    #[test]
    fn target_ref_preserves_value() {
        assert_eq!(
            TargetRef::new("component:greet").unwrap().as_str(),
            "component:greet"
        );
    }
}
