# Drift Parity: Quint Spec vs Rust Journal Validator

This file is the source of truth for parity between:

- Quint model invariants in `spec/journal/execution_journal.qnt`
- Rust local journal validation in `crates/invariant-journal`

Status values:

- `implemented-local`: Enforced by Rust per-journal validator.
- `model-only`: Used in Quint model for state-space or model semantics; not a Rust local invariant.
- `system-level`: Enforced by identity construction/persistence/runtime boundaries, not local validator.
- `rust-only-guard`: Extra Rust guard for schema/representation safety.

## Parity Matrix

| Spec ID | Quint Name | Rust Mapping | Status | Notes |
|---|---|---|---|---|
| INV-1 | `firstEventIsStarted` | `S-2` + empty journal guard in `validate_journal` | implemented-local | Rust rejects empty journal and non-`ExecutionStarted` first event. |
| INV-2 | `journalMonotonicity` | `S-1` (`NonMonotonicSequence`) | implemented-local | Sequence must match append index. |
| INV-3 | `terminalFinality` | `S-3` + `S-4` | implemented-local | At most one terminal, and terminal must be last. |
| INV-4 | `statusJournalConsistency` | none (local) | model-only | Rust journal does not persist a separate status field to compare; status is derived from journal fold. |
| INV-5 | `phaseOrdering` | `SE-1` (`StartedWithoutScheduled`) | implemented-local | `InvokeStarted` requires prior `InvokeScheduled`. |
| SE-2 | `completedRequiresStarted` | `SE-2` (`CompletedWithoutStarted`) | implemented-local | `InvokeCompleted` requires prior `InvokeStarted`. |
| SE-3 | `retryingRequiresStarted` | `SE-3` (`RetryingWithoutStarted`) | implemented-local | Rust checks `(promise_id, failed_attempt)` against started attempts. |
| SE-4 | `noEventsAfterCompleted` | `SE-4` (`EventAfterCompleted`) | implemented-local | Blocks `InvokeStarted`, `InvokeRetrying`, and duplicate `InvokeCompleted` after completion. |
| MB-1 | `modelRetryBounded` | none (local) | model-only | Quint state-space bound. Runtime retry policy enforcement is outside local journal validation. |
| CF-1 | `timerFiredRequiresScheduled` | `CF-1` (`TimerFiredWithoutScheduled`) | implemented-local | Timer fire requires prior schedule. |
| CF-2 | `signalReceivedRequiresDelivered` | `CF-2` (`SignalReceivedWithoutDelivery`) | implemented-local | Requires matching name, delivery id, payload. |
| CF-3 | `signalConsumedOnce` | `CF-3` (`SignalConsumedTwice`) | implemented-local | Delivery may be consumed once. |
| CF-4 | `awaitSignalConsistent` | `CF-4` (`AwaitSignalInconsistent`) | implemented-local | Signal await must wait on exactly one matching promise. |
| JS-1 | `submitRequiresCreated` | `JS-1` (`SubmitWithoutCreate`) | implemented-local | Submit requires create. |
| JS-2 | `noSubmitAfterAwait` | `JS-2` (`SubmitAfterAwait`) | implemented-local | First await freezes submissions. |
| JS-3 | `awaitedRequiresMember` | `JS-3` (`AwaitedNotMember`) | implemented-local | Awaited promise must be submitted member. |
| JS-4 | `awaitedRequiresCompleted` | `JS-4` (`AwaitedNotCompleted`) | implemented-local | Awaited promise must be completed. |
| JS-5 | `noDoubleConsume` | `JS-5` (`DoubleConsume`) | implemented-local | Same `(join_set_id, promise_id)` cannot be consumed twice. |
| JS-6 | `consumeBounded` | `JS-6` (`ConsumeExceedsSubmit`) | implemented-local | Await count cannot exceed submit count. |
| JS-7 | `promiseSingleOwner` | `JS-7` (`PromiseInMultipleJoinSets`) | implemented-local | Promise belongs to at most one join set. |
| INV-6 | `promiseIdUniqueness` | none (local) | system-level | Cross-execution uniqueness is enforced by `PromiseId` construction and persistence constraints, not local per-journal validation. |
| (extra) | `waiting_on` set semantics | `AwaitWaitingOnDuplicate` | rust-only-guard | Rust stores `waiting_on` as `Vec`; validator rejects duplicates to match Quint set semantics. |

## Boundary Decisions

- Rust local validator is intentionally single-journal and does not perform cross-execution scans.
- `INV-6` is a system-level guarantee (identity construction + persistence constraints).
- `INV-4` is not locally enforceable in current Rust shape because no independent persisted status is checked against journal-derived status.

## Verification Commands

Run both to guard against drift:

```bash
cargo test -p invariant-journal
quint run spec/journal/execution_journal.qnt --mbt=true --n-traces=25 --max-steps=60
```
