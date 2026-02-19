# Invariant Journal vs Obelisk: Architectural Comparison

A comparison of the Invariant execution journal with the [Obelisk](https://github.com/obeli-sk/obelisk) workflow engine, focusing on journal/execution-log design, determinism strategy, replay mechanics, and overall architecture.

Both systems are Rust-based durable workflow engines built on the WASM Component Model. They share the fundamental premise that append-only event logs enable crash recovery and deterministic replay. The differences lie in the granularity of the event model, the formalism applied, and the scope of the runtime.

---

## 1. Core Architecture

| Dimension | Invariant | Obelisk |
|-----------|-----------|---------|
| Language | Rust | Rust |
| WASM runtime | wasmtime (Component Model) | wasmtime (Component Model) |
| Storage | Not yet implemented (journal is in-memory) | SQLite or PostgreSQL |
| Deployment | Library crates (no runtime binary yet) | Single binary (runtime, DB, executor, web UI) |
| Maturity | Early-stage (stub `main.rs`) | Pre-release, actively used (v0.34+) |
| License | — | AGPL-3.0 |

**Obelisk** is a vertically-integrated product: a single binary bundles the orchestrator, DB, worker executor, CLI, gRPC API, and web UI. The execution log is persisted in SQLite/PostgreSQL with full CRUD support.

**Invariant** is a set of library crates (`invariant-types`, `invariant-journal`, `invariant-engine`) with no runtime shell yet. The focus has been on formally specifying and validating the journal data model before building the runtime around it.

---

## 2. Event Model

This is the most significant architectural difference.

### Invariant: 20 event types, 5 categories

Invariant defines a rich, fine-grained event taxonomy organized by the formal property each category guarantees:

| Category | Formal Property | Events | Count |
|----------|-----------------|--------|-------|
| Lifecycle | WF-net soundness | `ExecutionStarted`, `ExecutionCompleted`, `ExecutionFailed`, `CancelRequested`, `ExecutionCancelled` | 5 |
| Side Effects | Replay correctness | `InvokeScheduled`, `InvokeStarted`, `InvokeCompleted`, `InvokeRetrying` | 4 |
| Nondeterminism | Determinism guarantee | `RandomGenerated`, `TimeRecorded` | 2 |
| Control Flow | State reconstruction | `TimerScheduled`, `TimerFired`, `SignalDelivered`, `SignalReceived`, `ExecutionAwaiting`, `ExecutionResumed` | 6 |
| Concurrency | Total ordering | `JoinSetCreated`, `JoinSetSubmitted`, `JoinSetAwaited` | 3 |

Key design choices:
- **3-phase side effects** (Scheduled → Started → Completed) for intent logging, timeout detection, and result caching
- **Explicit suspend/resume** events (`ExecutionAwaiting`/`ExecutionResumed`) per IEEE 1849 (XES)
- **Two-event signal model** (`SignalDelivered` for durable buffer + `SignalReceived` for consumption)
- **Unified invoke** — no activity/child-workflow split; `InvokeKind` (Function, Http, ...) is extensible via enum variants

### Obelisk: Coarser execution log

Obelisk records child execution submissions and their results in an execution log. The event granularity is coarser:

- Records **child execution creation** (submission) and **child execution result** (completion)
- Records **JoinSet creation** and **JoinSet await** responses
- Does not appear to have explicit suspend/resume events, timer events, or signal events as first-class journal entries
- Activities and workflows are distinct execution types with different persistence semantics

Obelisk's log is closer to a "call graph + results" ledger than a full execution trace. This is sufficient for its replay model because WASM determinism eliminates the need to record intermediate state transitions.

### Summary

Invariant's journal captures *why* the execution reached each state (await reasons, signal buffering, retry attempts). Obelisk's log captures *what happened* (child submissions and results). Invariant optimizes for auditability and formal verification; Obelisk optimizes for simplicity and performance.

---

## 3. Determinism Strategy

Both systems use WASM sandboxing to enforce determinism, but at different levels of rigor.

| Aspect | Invariant | Obelisk |
|--------|-----------|---------|
| WASM sandbox | Yes (`wasm32-unknown-unknown`) | Yes (`wasm32-unknown-unknown` for workflows) |
| NaN canonicalization | `cranelift_nan_canonicalization(true)` | Likely similar (not confirmed in docs) |
| Relaxed SIMD determinism | `relaxed_simd_deterministic(true)` | Not documented |
| Random capture | `RandomGenerated` event with promise_id | Handled implicitly (no `random()` in sandbox) |
| Time capture | `TimeRecorded` event with promise_id | Handled implicitly (no `now()` in sandbox) |
| Component pinning | `component_digest` in `ExecutionStarted` | Implicit (WASM binary hash in registry) |

**Invariant** takes a belt-and-suspenders approach: even though WASM eliminates nondeterminism, it explicitly records random values and timestamps as journal events. This means the journal alone is sufficient to verify determinism — you can replay without re-executing WASM.

**Obelisk** relies entirely on compile-time guarantees. Since `wasm32-unknown-unknown` has no access to randomness or wall-clock time, these values simply cannot leak into workflow logic. The WASM sandbox *is* the determinism guarantee. Activities (which can access the outside world) run on `wasm32-wasip2` and are treated as non-deterministic — their results are persisted and replayed.

---

## 4. Replay Protocol

| Aspect | Invariant | Obelisk |
|--------|-----------|---------|
| Cache structure | `HashMap<PromiseId, CachedResult>` | Execution log index (DB-backed) |
| Cache build | O(n) single-pass scan | Loaded from persistent storage |
| Cache key | `PromiseId` (Dewey-encoded path) | Execution ID / child index |
| Cache types | 5 variants: Invoke, Random, Time, Timer, Signal | Child execution results |
| Yield mechanism | WASM trap | WASM trap (unload from memory) |
| Resume strategy | Replay from beginning | Replay from beginning |
| State serialization | None (replay replaces it) | None (replay replaces it) |

Both systems use the same fundamental replay strategy: on resume, re-execute the workflow from the start and return cached results for operations that were already completed. Neither serializes intermediate state.

**Invariant** uses `PromiseId` (Dewey notation: `root.0.1.3`) as the cache key. Each SDK call (invoke, random, time, timer, signal) advances a local sequence counter that deterministically maps to a unique path in the call tree. The `ReplayCache` holds 5 typed variants.

**Obelisk** indexes by child execution position. Since the only recorded events are child submissions and results, the cache is structurally simpler. The work-stealing executor handles unloading idle workflows from memory and replaying them on demand.

---

## 5. Identity Model

| Aspect | Invariant | Obelisk |
|--------|-----------|---------|
| Execution ID | `PromiseId` (SHA-256 root + path) | Execution ID (likely UUID or similar) |
| Operation ID | `PromiseId` (Dewey notation: `root.0.1`) | Child execution index |
| Max depth | 64 | No documented limit |
| JoinSet ID | `JoinSetId(PromiseId)` newtype | JoinSet ID |

Invariant's `PromiseId` is a core differentiator. It encodes the complete call-tree position using Dewey notation, making every operation's identity deterministic by construction. The path-based scheme provides: unique identity without coordination, deterministic replay keys, and natural parent-child relationships.

Obelisk uses simpler execution identifiers. The structured concurrency model (parent-child relationships) is maintained through the execution log rather than through the identity scheme itself.

---

## 6. Formal Specification

| Aspect | Invariant | Obelisk |
|--------|-----------|---------|
| Formal model | Quint (53 KB spec) | None documented |
| Invariant count | 21 (enforced by Rust validator) | Not documented |
| Model-code parity | `DRIFT_PARITY.md` tracks alignment | N/A |
| Verification | `quint run` + `cargo test` | `cargo test` |

This is Invariant's strongest differentiator. The journal design is backed by a 53 KB Quint formal specification that models the entire state machine. 21 invariants are enforced across 4 categories:

- **Structural (S-1..S-5):** Sequence monotonicity, lifecycle bookends, terminal uniqueness
- **Side Effects (SE-1..SE-4):** Phase ordering for the Scheduled → Started → Completed pipeline
- **Control Flow (CF-1..CF-4):** Timer, signal, and await consistency
- **JoinSet (JS-1..JS-7):** Creation, submission, consumption, and ownership rules

The `DRIFT_PARITY.md` file tracks which invariants are `implemented-local` (enforced in Rust), `model-only` (Quint state-space bounds), `system-level` (enforced by PromiseId construction), or `rust-only-guard` (extra Rust checks). This level of formal rigor is uncommon in workflow engine implementations.

Obelisk does not document a formal specification. Correctness relies on the WASM sandbox guarantees, the database's ACID properties, and integration tests.

---

## 7. Structured Concurrency (JoinSets)

Both systems support structured concurrency via JoinSets, but with different journal representations.

**Invariant** records three events per JoinSet lifecycle:
1. `JoinSetCreated` — opens the concurrent region
2. `JoinSetSubmitted` — adds a promise to the set (no submits after first await — JS-2)
3. `JoinSetAwaited` — replay marker recording which result was consumed

Seven invariants (JS-1 through JS-7) enforce correctness: creation before submission, no late submissions, consumed promises must be members, consumed promises must be completed, no double consumption, bounded consumption, and single-owner promises.

**Obelisk** supports JoinSets with similar semantics (blocking until child executions complete, or cancelling unfinished children). The execution log records JoinSet creation and responses. The closure property (no submits after await) and structured cancellation are enforced at runtime.

---

## 8. Error Handling & Retries

| Aspect | Invariant | Obelisk |
|--------|-----------|---------|
| Retry tracking | `InvokeRetrying` event with attempt number | Auto-retry with configurable policy |
| Retry visibility | Full history in journal (Started → Retrying → Started → Completed) | Results persisted; retry attempts not individually logged (based on docs) |
| Cancellation | 2-phase (`CancelRequested` → `ExecutionCancelled`) | Supported (cascade to children) |
| Timeout | Epoch-based interruption (1s ticks) | Configurable timeouts |

Invariant's 3-phase model with explicit `InvokeRetrying` events means the complete retry history is captured in the journal. You can see exactly which attempt failed, why, and when the next attempt was scheduled.

Obelisk handles retries at the executor level with configurable policies. Activities are auto-retried on errors, timeouts, and WASM traps.

---

## 9. What Invariant Can Learn From Obelisk

1. **Ship a runtime.** Obelisk's single-binary deployment model (everything in one process) proves that a WASM workflow engine can be operationally simple. Invariant's `main.rs` is still a stub.

2. **Persistence layer.** Obelisk's use of SQLite as the default storage demonstrates that durable workflows don't need a complex distributed database. A simple embedded DB suffices for many use cases.

3. **Web UI and debugging tools.** Obelisk's time-traveling debugger and web UI for inspecting execution logs are high-value features for developer experience.

4. **Performance benchmarking.** Obelisk's public benchmarks against Temporal and Restate provide concrete performance data. Invariant should consider a similar benchmark suite.

5. **Work-stealing executor.** Obelisk's ability to unload idle workflows from memory and replay on demand addresses a practical concern for long-running workflows.

---

## 10. What Obelisk Could Learn From Invariant

1. **Formal specification.** Invariant's Quint model and 21 enforced invariants catch design errors before they become runtime bugs. The `DRIFT_PARITY.md` approach ensures the spec and code stay in sync.

2. **Fine-grained event model.** The 3-phase side effect tracking, explicit suspend/resume, and two-event signal model provide richer auditability and enable more precise failure diagnosis.

3. **Path-based identity.** The Dewey-encoded `PromiseId` makes every operation's identity deterministic by construction and self-describing, eliminating the need for coordination.

4. **Nondeterminism capture.** Recording `RandomGenerated` and `TimeRecorded` even within a WASM sandbox enables journal-only replay verification without re-executing WASM.

5. **Categorized invariants.** Grouping invariants by formal property (soundness, replay correctness, determinism, state reconstruction, total ordering) makes the correctness argument modular and easier to reason about.

---

## Summary Table

| Dimension | Invariant | Obelisk |
|-----------|-----------|---------|
| Event types | 20 (5 categories) | Fewer (submissions + results) |
| Formal spec | Quint (21 invariants) | None documented |
| Determinism | WASM sandbox + explicit capture | WASM sandbox only |
| Replay key | Dewey-encoded PromiseId | Execution/child index |
| Side effect phases | 3 (Scheduled → Started → Completed) | ~2 (submission → result) |
| Signal model | 2-event (Delivered + Received) | Not documented as first-class |
| Suspend/resume | Explicit events | Implicit |
| Storage | Not implemented | SQLite / PostgreSQL |
| Runtime | Not implemented | Full single-binary runtime |
| Yield mechanism | WASM trap | WASM trap |
| Concurrency | JoinSets (7 invariants) | JoinSets (runtime-enforced) |
| Developer UX | Library crates | CLI + gRPC + Web UI |

Both projects demonstrate that WASM is a strong foundation for deterministic workflow execution. Invariant brings formal rigor and a rich event model; Obelisk brings a complete, deployable runtime with proven performance. They represent complementary points on the "correctness-first vs. ship-first" spectrum.
