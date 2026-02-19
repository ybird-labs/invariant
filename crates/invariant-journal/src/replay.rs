use std::collections::HashMap;

use chrono::{DateTime, Utc};
use invariant_types::{EventType, JournalEntry, Payload, PromiseId};

/// Replay-time cached value for a resolved promise.
///
/// Each variant corresponds to one event kind that can be replayed by promise ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CachedResult {
    /// From `InvokeCompleted { result, .. }`.
    Invoke(Payload),
    /// From `RandomGenerated { value, .. }`.
    Random(Vec<u8>),
    /// From `TimeRecorded { time, .. }`.
    Time(DateTime<Utc>),
    /// From `TimerFired { .. }`.
    Timer,
    /// From `SignalReceived { payload, .. }`.
    Signal(Payload),
}

/// Batch-built replay cache keyed by `PromiseId`.
///
/// Construction is a single O(n) scan over journal entries.
/// Only five event kinds contribute cache entries.
#[derive(Clone, Debug, Default)]
pub struct ReplayCache {
    results: HashMap<PromiseId, CachedResult>,
}

impl ReplayCache {
    /// Build cache entries from a full journal history in one pass.
    ///
    /// Cached event kinds:
    /// - `InvokeCompleted` -> `CachedResult::Invoke`
    /// - `RandomGenerated` -> `CachedResult::Random`
    /// - `TimeRecorded` -> `CachedResult::Time`
    /// - `TimerFired` -> `CachedResult::Timer`
    /// - `SignalReceived` -> `CachedResult::Signal`
    ///
    /// Non-cached events:
    /// - `SignalDelivered` (no `promise_id`)
    /// - `JoinSetAwaited` (consumed via sequence scan, not map lookup)
    pub fn build(entries: &[JournalEntry]) -> Self {
        let mut results = HashMap::new();

        for entry in entries {
            match &entry.event {
                EventType::InvokeCompleted {
                    promise_id, result, ..
                } => {
                    results.insert(promise_id.clone(), CachedResult::Invoke(result.clone()));
                }
                EventType::RandomGenerated { promise_id, value } => {
                    results.insert(promise_id.clone(), CachedResult::Random(value.clone()));
                }
                EventType::TimeRecorded { promise_id, time } => {
                    results.insert(promise_id.clone(), CachedResult::Time(time.clone()));
                }
                EventType::TimerFired { promise_id } => {
                    results.insert(promise_id.clone(), CachedResult::Timer);
                }
                EventType::SignalReceived {
                    promise_id,
                    payload,
                    ..
                } => {
                    results.insert(promise_id.clone(), CachedResult::Signal(payload.clone()));
                }
                _ => {}
            }
        }

        Self { results }
    }

    /// Generic lookup by promise ID.
    pub fn lookup(&self, pid: &PromiseId) -> Option<&CachedResult> {
        self.results.get(pid)
    }

    /// Typed accessor for invoke results.
    pub fn get_invoke(&self, pid: &PromiseId) -> Option<&Payload> {
        match self.lookup(pid) {
            Some(CachedResult::Invoke(payload)) => Some(payload),
            _ => None,
        }
    }

    /// Typed accessor for random bytes.
    pub fn get_random(&self, pid: &PromiseId) -> Option<&[u8]> {
        match self.lookup(pid) {
            Some(CachedResult::Random(bytes)) => Some(bytes.as_slice()),
            _ => None,
        }
    }

    /// Typed accessor for recorded wall-clock time.
    pub fn get_time(&self, pid: &PromiseId) -> Option<DateTime<Utc>> {
        match self.lookup(pid) {
            Some(CachedResult::Time(time)) => Some(*time),
            _ => None,
        }
    }

    /// True if timer completion was recorded for this promise.
    pub fn is_timer_complete(&self, pid: &PromiseId) -> bool {
        matches!(self.lookup(pid), Some(CachedResult::Timer))
    }

    /// Typed accessor for received signal payloads.
    pub fn get_signal(&self, pid: &PromiseId) -> Option<&Payload> {
        match self.lookup(pid) {
            Some(CachedResult::Signal(payload)) => Some(payload),
            _ => None,
        }
    }

    /// Number of cached promise results.
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// True when no promise results are cached.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use invariant_types::Codec;

    use super::*;

    fn pid(tag: u8) -> PromiseId {
        PromiseId::new([tag; 32])
    }

    fn payload(bytes: &[u8]) -> Payload {
        Payload::new(bytes.to_vec(), Codec::Json)
    }

    fn entry(sequence: u64, event: EventType) -> JournalEntry {
        JournalEntry {
            sequence,
            timestamp: Utc::now(),
            event,
        }
    }

    #[test]
    fn build_caches_all_supported_event_types() {
        let p_invoke = pid(1);
        let p_random = pid(2);
        let p_time = pid(3);
        let p_timer = pid(4);
        let p_signal = pid(5);

        let entries = vec![
            entry(
                0,
                EventType::InvokeCompleted {
                    promise_id: p_invoke.clone(),
                    result: payload(&[1]),
                    attempt: 1,
                },
            ),
            entry(
                1,
                EventType::RandomGenerated {
                    promise_id: p_random.clone(),
                    value: vec![7, 8, 9],
                },
            ),
            entry(
                2,
                EventType::TimeRecorded {
                    promise_id: p_time.clone(),
                    time: Utc::now(),
                },
            ),
            entry(
                3,
                EventType::TimerFired {
                    promise_id: p_timer.clone(),
                },
            ),
            entry(
                4,
                EventType::SignalReceived {
                    promise_id: p_signal.clone(),
                    signal_name: "sig".into(),
                    payload: payload(&[2]),
                    delivery_id: 1,
                },
            ),
            // Not cached:
            entry(
                5,
                EventType::SignalDelivered {
                    signal_name: "sig".into(),
                    payload: payload(&[3]),
                    delivery_id: 2,
                },
            ),
            entry(
                6,
                EventType::TimerScheduled {
                    promise_id: pid(6),
                    duration: Duration::seconds(1),
                    fire_at: Utc::now(),
                },
            ),
        ];

        let cache = ReplayCache::build(&entries);

        assert_eq!(cache.len(), 5);
        assert!(!cache.is_empty());
        assert_eq!(cache.get_invoke(&p_invoke), Some(&payload(&[1])));
        assert_eq!(cache.get_random(&p_random), Some([7, 8, 9].as_slice()));
        assert!(cache.get_time(&p_time).is_some());
        assert!(cache.is_timer_complete(&p_timer));
        assert_eq!(cache.get_signal(&p_signal), Some(&payload(&[2])));
    }

    #[test]
    fn typed_accessors_fail_closed_on_variant_mismatch() {
        let p_invoke = pid(11);
        let entries = vec![entry(
            0,
            EventType::InvokeCompleted {
                promise_id: p_invoke.clone(),
                result: payload(&[9]),
                attempt: 1,
            },
        )];
        let cache = ReplayCache::build(&entries);

        assert!(cache.get_random(&p_invoke).is_none());
        assert!(cache.get_time(&p_invoke).is_none());
        assert!(!cache.is_timer_complete(&p_invoke));
        assert!(cache.get_signal(&p_invoke).is_none());
    }
}
