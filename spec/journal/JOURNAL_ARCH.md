# Execution Journal Design

Authoritative reference for the Invariant execution journal. The journal is an append-only sequence of events that fully describes an execution's history. It is the source of truth — state is derived by replaying it.

**Purpose:** Durability (survive crashes), replay (reconstruct state), exactly-once semantics (cached results prevent re-execution of side effects).

**Related docs:** `JOINSET_DESIGN.md` (JoinSet deep-dive), `execution_journal.qnt` (Quint formal spec).

---

## Journal Structure

```rust
pub struct ExecutionJournal {
    pub execution_id: ExecutionId,
    pub state: JournalState,        // Derived from events, cached
    pub events: Vec<JournalEvent>,  // Append-only
}

pub struct JournalEvent {
    pub sequence: u64,              // 0-indexed, monotonically increasing
    pub timestamp: DateTime<Utc>,   // Wall-clock (debugging only, NOT used in replay)
    pub event: EventType,
}
```

Version = `events.len()`. Flat structure, simple storage, natural time ordering.

---

## Event Types (20 events, 5 categories)

Each category satisfies a distinct formal correctness property.

### Category 1: Lifecycle (Soundness)

Formal basis: WF-net soundness — proper initiation and termination.

| Event | When Recorded | Data |
|-------|---------------|------|
| `ExecutionStarted` | Always first | component_digest, input, parent_id, idempotency_key |
| `ExecutionCompleted` | Function returns Ok | result |
| `ExecutionFailed` | Function returns Err or traps | error |
| `CancelRequested` | External cancel signal arrives | reason |
| `ExecutionCancelled` | Cancellation finalized | reason |

### Category 2: Side Effects (Replay Correctness)

Formal basis: Non-idempotent operations must cache results. All follow **3-phase pattern: Scheduled -> Started -> Completed**.

**Why 3-phase:** Scheduled = intent (for replay matching, enables exactly-once via intent logging). Started = in-flight (timeout detection). Completed = result (cache for replay). Retrying = transient failure, will retry.

| Event | Phase | Data |
|-------|-------|------|
| `InvokeScheduled` | Scheduled | promise_id, kind, function_name, input, retry_policy |
| `InvokeStarted` | Started | promise_id, attempt |
| `InvokeCompleted` | Completed | promise_id, result, attempt |
| `InvokeRetrying` | Retry | promise_id, failed_attempt, error, retry_at |

`InvokeKind` categorizes the invocation type. The `kind` field lives on `InvokeScheduled` only — later phases inherit kind via promise_id lookup.

Modeling note: Quint keeps a fixed retry bound for tractable model checking and does not interpret `retry_policy`; Rust runtime enforces retry policy (including max attempts).

```rust
pub enum InvokeKind {
    Function,   // function/task/workflow invocation
    Http,       // HTTP request to external service
    // Future: Database, Grpc, Message, etc.
}
```

This is extensible: new side effect types (DB queries, gRPC calls) are added as `InvokeKind` variants, not new event types. All share the same 3-phase structure and replay semantics.

### Category 3: Nondeterminism (Determinism Guarantee)

Formal basis: LTS determinism — entropy sources must be captured. **Single-phase** (pure value capture, no execution to track).

| Event | When Recorded | Data |
|-------|---------------|------|
| `RandomGenerated` | `random()` called | promise_id, value |
| `TimeRecorded` | `now()` called | promise_id, time |

### Category 4: Control Flow (State Reconstruction)

Formal basis: CSP trace semantics — branching decisions define execution paths.

| Event | When Recorded | Data |
|-------|---------------|------|
| `TimerScheduled` | `sleep(duration)` called | promise_id, duration, fire_at |
| `TimerFired` | Duration elapsed | promise_id |
| `SignalDelivered` | External signal arrives at execution | signal_name, payload, delivery_id |
| `SignalReceived` | Workflow consumes signal via `await_signal()` | promise_id, signal_name, payload, delivery_id |
| `ExecutionAwaiting` | Workflow blocks | waiting_on: Vec\<PromiseId\>, kind: AwaitKind |
| `ExecutionResumed` | Blocked → Running (wait satisfied) | — |

`AwaitKind`: `Single` | `Any` (first of many) | `All` (all must complete) | `Signal(name)`. Signal await assigns a PromiseId via the local sequence counter, same as invoke/timer/random.

**Signal two-event model:** `SignalDelivered` is the durable buffer — recorded when an external signal arrives, no `promise_id` (external to the call tree). Each delivery assigns a per-signal-name monotonic `delivery_id`. `SignalReceived` is the consumption event — recorded when `await_signal()` matches a delivered signal, carries a `promise_id` for the replay cache, and records the consumed `delivery_id`. Consumption is FIFO per signal name (oldest unconsumed delivery). If the signal is already buffered when `await_signal()` is called, `SignalReceived` is recorded immediately (no blocking). If not, the workflow blocks until the signal arrives.

`ExecutionAwaiting` is the explicit suspend (IEEE 1849). `ExecutionResumed` is the explicit resume — recorded when the blocked wait condition is satisfied.

### Category 5: Concurrency (Total Ordering)

Formal basis: Lamport timestamps — concurrent results need deterministic ordering for replay.

| Event | When Recorded | Data |
|-------|---------------|------|
| `JoinSetCreated` | `join_set()` called | join_set_id *(child-allocating)* |
| `JoinSetSubmitted` | `js.submit(...)` called | join_set_id, promise_id |
| `JoinSetAwaited` | `js.next()` returns | join_set_id, promise_id, result |

**JoinSetId identity:** `JoinSetId` is a newtype wrapper around `PromiseId`. The `join_set()` call allocates a child position via `nextChildSeq++`, same as invoke/random/time/timer/signal. This keeps JoinSets consistent with the identity model — every SDK call occupies a deterministic position in the call tree. The newtype prevents misuse: a `JoinSetId` cannot be awaited, completed, or submitted to another JoinSet.

```rust
pub struct JoinSetId(pub PromiseId);
```

**Why PromiseId, not string or counter:** (1) Unique by construction — same Dewey encoding guarantee as all other IDs. (2) Deterministic on replay — position-based, not dependent on runtime values. (3) Zero new mechanism — reuses `nextChildSeq` and `allocatedChildren`. (4) A PromiseId newtype captures the structural relationship between JoinSets and the identity hierarchy with typed safety.

**Boundary: INV-6**
- Per-journal validator does not check cross-execution uniqueness.
- Uniqueness is guaranteed by:
  1) `PromiseId` construction (`root + path`)
  2) Persistence constraints (below)

`JoinSetAwaited` is a **replay marker**, not a state transition. It records which result was consumed at this point — whether or not the workflow blocked. `ExecutionAwaiting` handles blocking; `JoinSetAwaited` handles replay ordering. See `JOINSET_DESIGN.md` for full design.

### Category Summary

| Category | Formal Property | Guarantee | Events |
|----------|-----------------|-----------|--------|
| Lifecycle | Soundness | Proper start/end | 5 |
| Side Effects | Replay Correctness | External ops not re-executed | 4 |
| Nondeterminism | Determinism | Same random/time on replay | 2 |
| Control Flow | State Reconstruction | Same execution path on replay | 6 |
| Concurrency | Total Ordering | Same result order on replay | 3 |

**Together:** `forall execution E, replay(journal(E)) = E`

---

## State Machine

```
                    ExecutionStarted
                          |
                          v
                      ┌────────┐
              ┌──────>│Running │<──────┐
              │       └───┬────┘       │
              │           │            │
         result arrives   │     ExecutionAwaiting
              │           │            │
              │     ┌─────┴─────┐      │
              └─────│  Blocked  │──────┘
                    └───────────┘
                          │
              ┌───────────┼───────────┐
              v           v           v
         Completed     Failed    Cancelling
                                      │
                                      v
                                  Cancelled
```

`CancelRequested` transitions Running or Blocked to `Cancelling`.
`ExecutionCancelled` transitions Cancelling to `Cancelled` (terminal).

```rust
pub enum JournalState {
    Running,
    Blocked { waiting_on: Vec<PromiseId>, kind: AwaitKind },
    Cancelling,   // cancel requested, cleanup in progress
    Completed,    // terminal
    Failed,       // terminal
    Cancelled,    // terminal
}
```

**State derivation** (from events, not stored independently):

Precondition: journal is non-empty (invariant S-2 guarantees first event is ExecutionStarted).

 Fold over journal, carrying status forward. Only 7 event types change status:

| Event | Status |
|-------|--------|
| ExecutionStarted | Running |
| CancelRequested | Cancelling |
| ExecutionAwaiting | Blocked(waiting_on, kind) |
| ExecutionResumed | Running |
| ExecutionCompleted | Completed |
| ExecutionFailed | Failed |
| ExecutionCancelled | Cancelled |
| Everything else | unchanged |

Wait satisfaction (guard for `ExecutionResumed`) depends on `AwaitKind`:
- `Single` / `All`: all waiting_on promises in `completedPromises`
- `Any`: at least one waiting_on promise in `completedPromises`
- `Signal(name)`: the signal's promise_id in `completedPromises` (a `SignalReceived` event exists for this pid)

---

## Identity: Path-Based PromiseId

```rust
pub struct PromiseId {
    root: [u8; 32],      // hash(component_digest, parent, idempotency_key)
    path: Vec<u32>,       // sequence numbers at each depth
}
pub type ExecutionId = PromiseId;  // path.is_empty() == true
pub struct JoinSetId(pub PromiseId);  // child-allocating, distinct type
```

Encodes position in call tree. Properties: unique (position is unique), deterministic (same code path -> same sequence -> same IDs), recursive-safe (each depth has own counter).

```
workflow_main()        -> "root"        (depth 0)
  invoke!("task_a")    -> "root.0"      (depth 1, seq 0)
  invoke!("task_a")    -> "root.1"      (depth 1, seq 1 — same fn, different position)
task_a(x)              -> "root.0"
  time.now()           -> "root.0.0"    (depth 2, seq 0)
  invoke!("task_c")    -> "root.0.1"    (depth 2, seq 1)
task_c(z)              -> "root.0.1"
  random.u64()         -> "root.0.1.0"  (depth 3, seq 0)
```

**What PromiseId encodes:**
- `promise_id.execution_root()` -> which journal to load
- `promise_id.parent()` -> who to notify on completion
- `promise_id.child(seq)` -> create child for nth operation

---

## Replay Protocol

**Non-determinism eliminated by design.** ComponentDigest pins execution to exact WASM binary. WASM sandbox eliminates ambient nondeterminism. Same code + same journal = same replay, always. No command matching, no trace comparison, no alignment checks.

```
1. Load journal from storage
2. Build HashMap<PromiseId, CachedResult> from completed events
3. Execute function from beginning
4. On each operation:
   - Generate child_id = current_promise_id.child(local_sequence++)
   - Lookup child_id in HashMap
   - HIT  -> return cached result, continue
   - MISS -> record InvokeScheduled, queue child work item, TRAP
5. On completion: record result, queue parent (respecting JoinSet rules)
```

**Yield mechanism:** WASM trap. No state serialization. Execution replays from beginning on resume.

**Work queue:** Unified. Starting workflows, executing tasks, resuming parents after child completion — all the same: pick WorkItem, load journal, execute/replay.

---

## Retry Handling

PromiseId = logical operation (stable across retries). Attempt = physical execution.

```
invoke!("flaky_api", x) -> promise_id = "root.0" for ALL attempts

Journal:
| promise_id | event            | attempt |
|------------|------------------|---------|
| root.0     | InvokeScheduled  | -       |
| root.0     | InvokeStarted    | 1       |
| root.0     | InvokeRetrying   | 1       |  <- failed
| root.0     | InvokeStarted    | 2       |
| root.0     | InvokeCompleted  | 2       |  <- success
```

On replay: generate "root.0", lookup in cache, return cached result immediately. Retry history preserved but not needed for replay.

---

## Design Decisions

| Decision | Choice | Formal Basis |
|----------|--------|--------------|
| Event categories | 5 categories by correctness property | Process calculi, LTS, WF-net |
| Side effect phases | 3-phase (Scheduled/Started/Completed) | Intent logging, XES lifecycle |
| Nondeterminism phases | 1-phase (value capture) | LTS determinism |
| Journal structure | Append-only, flat | Event sourcing, WAL |
| Ordering | Sequence numbers (Lamport clock) | Lamport 1978 |
| Identity | Path-based PromiseId | Deterministic, unique, O(1) lookup |
| ExecutionId | Type alias for root PromiseId | Same concept at depth 0 |
| Non-determinism detection | Eliminated by design | ComponentDigest + WASM sandbox |
| Cancel | 2-phase (CancelRequested → ExecutionCancelled) | XES pi_abort |
| Unified invoke | No activity/child workflow split | Burckhardt formal model |
| JoinSet closure | No submit after first await | Structured concurrency, WCP2/3/9 |
| JoinSet ownership | Promise in at most one set | Linearizability, scope ownership |
| JoinSetAwaited | Replay marker, not state transition | Burckhardt, Schneider |
| JoinSetId | PromiseId newtype, child-allocating | Identity model consistency, Dewey encoding |
| Yield | WASM trap | No state serialization needed |
| Resume | Explicit ExecutionResumed event | Journal is source of truth; status derived from last event |
| Work scheduling | Unified queue | All execution types same mechanism |
| Signals | Two-event (SignalDelivered + SignalReceived) | Durable buffer + promise-keyed consumption |

---

## Invariants

All invariants must hold at every journal state.

### Structural Invariants

| ID | Name | Property |
|----|------|----------|
| S-1 | `monotonic_sequence` | Sequence numbers strictly increasing |
| S-2 | `starts_with_started` | First event is always ExecutionStarted |
| S-3 | `single_terminal` | At most one terminal event |
| S-4 | `terminal_is_last` | Terminal event is the final event |
| S-5 | `cancelled_requires_requested` | ExecutionCancelled requires preceding CancelRequested |

### Side Effect Invariants

| ID | Name | Property |
|----|------|----------|
| SE-1 | `started_requires_scheduled` | InvokeStarted(pid) requires preceding InvokeScheduled(pid) |
| SE-2 | `completed_requires_started` | InvokeCompleted(pid) requires preceding InvokeStarted(pid) |
| SE-3 | `retrying_requires_started` | InvokeRetrying(pid, attempt) requires preceding InvokeStarted(pid, attempt) |
| SE-4 | `no_events_after_completed` | No InvokeStarted/Retrying after InvokeCompleted for same pid |

### Control Flow Invariants

| ID | Name | Property |
|----|------|----------|
| CF-1 | `timer_fired_requires_scheduled` | TimerFired(pid) requires preceding TimerScheduled(pid) |
| CF-2 | `signal_received_requires_delivered` | SignalReceived(name, delivery_id, payload) requires preceding SignalDelivered(name, delivery_id, payload) |
| CF-3 | `signal_consumed_once` | Each delivery_id is consumed by at most one SignalReceived |
| CF-4 | `await_signal_consistent` | AwaitSignal.promise_id must match the single waiting_on promise_id |

### JoinSet Invariants

| ID | Name | Property |
|----|------|----------|
| JS-1 | `submit_requires_created` | JoinSetSubmitted(js) requires preceding JoinSetCreated(js) |
| JS-2 | `no_submit_after_await` | No JoinSetSubmitted(js) after any JoinSetAwaited(js) |
| JS-3 | `awaited_requires_member` | JoinSetAwaited(js, pid) requires preceding JoinSetSubmitted(js, pid) |
| JS-4 | `awaited_requires_completed` | JoinSetAwaited(_, pid) requires preceding InvokeCompleted(pid) |
| JS-5 | `no_double_consume` | No two JoinSetAwaited for same (js_id, pid) pair |
| JS-6 | `consume_bounded` | Per set: count(JoinSetAwaited) <= count(JoinSetSubmitted) |
| JS-7 | `promise_single_owner` | A promise_id appears in at most one join set |

---

## Journal Sequence: Full Example

A workflow that invokes a task, generates randomness, uses a JoinSet, and completes.

```
seq  event                                                 state
───  ─────                                                 ─────
 0   ExecutionStarted(component_digest, input)             Running
 1   RandomGenerated("root.0", 0x1a2b)                    Running
 2   InvokeScheduled("root.1", "fetch_user", {id:42})     Running
 3   ExecutionAwaiting(["root.1"], Single)                 Blocked
     ... child executes ...
 4   InvokeStarted("root.1", attempt=1)                   Blocked
 5   InvokeCompleted("root.1", Ok(User{...}), attempt=1)  Blocked
 6   ExecutionResumed                                      Running
 7   JoinSetCreated("root.2")                              Running
 8   InvokeScheduled("root.3", "send_email", {...})        Running
 9   JoinSetSubmitted("root.2", "root.3")                  Running
10   InvokeScheduled("root.4", "send_sms", {...})          Running
11   JoinSetSubmitted("root.2", "root.4")                  Running
12   ExecutionAwaiting(["root.3","root.4"], Any)            Blocked
     ... root.4 completes first ...
13   InvokeStarted("root.4", attempt=1)                   Blocked
14   InvokeCompleted("root.4", Ok(...), attempt=1)         Blocked
15   ExecutionResumed                                      Running
16   JoinSetAwaited("root.2", "root.4", Ok(...))           Running
     ... workflow calls js.next() again ...
17   ExecutionAwaiting(["root.3"], Any)                     Blocked
18   InvokeStarted("root.3", attempt=1)                   Blocked
19   InvokeRetrying("root.3", attempt=1, err, retry_at)    Blocked
20   InvokeStarted("root.3", attempt=2)                   Blocked
21   InvokeCompleted("root.3", Ok(...), attempt=2)         Blocked
22   ExecutionResumed                                      Running
23   JoinSetAwaited("root.2", "root.3", Ok(...))           Running
24   ExecutionCompleted(Ok(final_result))                   Completed
```

## Journal Sequence: Signal Examples

### Buffered signal (arrives before `await_signal()`)

```
seq  event                                                          state
───  ─────                                                          ─────
 0   ExecutionStarted(component_digest, input)                      Running
 1   InvokeScheduled("root.0", "create_order", {...})               Running
 2   ExecutionAwaiting(["root.0"], Single)                          Blocked
     ... child executes ...
 3   InvokeStarted("root.0", attempt=1)                            Blocked
 4   InvokeCompleted("root.0", Ok(order), attempt=1)                Blocked
 5   ExecutionResumed                                               Running
 6   SignalDelivered("user_approval", {approved: true}, delivery_id=1)              Running  ← arrives while running
      ... workflow calls await_signal("user_approval") ...
 7   SignalReceived("root.1", "user_approval", {approved: true}, delivery_id=1)    Running  ← consumed instantly, no block
 8   ExecutionCompleted(Ok(result))                                  Completed
```

### Blocking signal (arrives after `await_signal()`)

```
seq  event                                                          state
───  ─────                                                          ─────
 0   ExecutionStarted(component_digest, input)                      Running
 1   InvokeScheduled("root.0", "create_order", {...})               Running
 2   ExecutionAwaiting(["root.0"], Single)                          Blocked
     ... child executes ...
 3   InvokeStarted("root.0", attempt=1)                            Blocked
 4   InvokeCompleted("root.0", Ok(order), attempt=1)                Blocked
 5   ExecutionResumed                                               Running
     ... workflow calls await_signal("user_approval") ...
 6   ExecutionAwaiting(["root.1"], Signal("user_approval"))         Blocked  ← no signal yet, trap
     ... hours pass, user approves ...
 7   SignalDelivered("user_approval", {approved: true}, delivery_id=1)              Blocked
 8   SignalReceived("root.1", "user_approval", {approved: true}, delivery_id=1)    Blocked  ← resolved but not resumed yet
 9   ExecutionResumed                                               Running
10   ExecutionCompleted(Ok(result))                                  Completed
```

In both cases, the replayer indexes `SignalReceived` by `promise_id` into `CachedResult::Signal(payload)`. On replay, `await_signal("user_approval")` generates `"root.1"` → cache HIT → returns payload instantly. `SignalDelivered` events are ignored by the replayer (they exist only for durable buffering).

---

## Open Questions (Future Work)

- **Timeout as distinct event** — Currently folded into ExecutionFailed. May add ExecutionErrorKind::Timeout.
- ~~**2-phase cancel**~~ — Resolved: `CancelRequested` then `ExecutionCancelled`. Included in v1.
- **ContinueAsNew** — For unbounded-history workflows. Defer to v2.
- **JoinSet partial failure** — Propagate failure via JoinSetAwaited with Err.
- **JoinSet cancellation** — Cascade cancel to in-flight children.
- **Atomic regions** — Transactional grouping. Defer to v2.
- ~~**Signal semantics**~~ — Resolved: two-event model (SignalDelivered + SignalReceived). Buffering supported. Included in v1.
