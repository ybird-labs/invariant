use crate::domain::error::DomainError;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty() {
            return Err(DomainError::EmptyWorkerId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_should_reject_empty_worker_id() {
        let result = WorkerId::new("");

        assert_eq!(result, Err(DomainError::EmptyWorkerId));
    }

    #[test]
    fn as_str_should_return_worker_id() {
        let worker_id = WorkerId::new("worker-1").unwrap();

        assert_eq!(worker_id.as_str(), "worker-1");
    }
}
