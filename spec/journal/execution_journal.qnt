module executionJournal {
    // =========================================================================
    // Identity Types
    // =========================================================================
    // Path-based identifier encoding position in the call tree (Dewey encoding).
    // Real system: { root: [u8; 32], path: Vec<u32> }
    // Spec: List[int] — e.g., [1] is root, [1, 0] is first child, [1, 0, 2] is grandchild.
    type PromiseId = List[int]

    // This should technically be the root of the promise ID however Quint has no way
    // of knowing this
    type ExecutionId = PromiseId

    // Concurrent region
    // Real System: [u8, 32]
    type JoinSetId = str

    // Hash of the WASM binary. Pins execution to exact code version.
    // Real system: [u8; 32]
    type ComponentDigest = str

    // Opaque data crossing the WASM boundary (bytes + codec).
    // We never inspect contents in the spec — only pass them through.
    type Payload = str

    // Wall-clock timestamp. Used for debugging/metadata, NOT for replay logic.
    // Modeled as int (logical tick) because the spec doesn't need real dates.
    type Timestamp = int

    // Duration for timers/retries. Modeled as int (logical units).
    type Duration = int

    // Delivery id for buffered signals (per signal name, monotonic).
    type SignalId = int

    // Retry policy metadata (opaque for now; not interpreted in the spec).
    type RetryPolicy = str

    // What kind of side effect invocation
    type InvokeKind =
        | Function    // function/task/workflow invocation
        | Http        // HTTP request to external service

    // What kind of await
    type AwaitKind =
        | Single              // waiting on one promise
        | Any                 // first of many (JoinSet js.next())
        | All                 // all must complete (JoinSet js.all())
        | AwaitSignal({ name: str, promise_id: PromiseId })  // waiting for named signal

    // =========================================================================
    // Event Types — 20 events, 5 categories
    // =========================================================================

    type EventType =
        // --- Category 1: Lifecycle (Soundness) ---
        // Formal basis: WF-net soundness — proper initiation and termination
        | ExecutionStarted({ component_digest: ComponentDigest,
                             input: Payload,
                             parent_id: PromiseId,
                             idempotency_key: str })
        | ExecutionCompleted({ result: Payload })
        | ExecutionFailed({ error: str })
        | CancelRequested({ reason: str })
        | ExecutionCancelled({ reason: str })

        // --- Category 2: Side Effects (Replay Correctness) ---
        // Formal basis: non-idempotent ops must cache results
        // All follow 3-phase: Scheduled -> Started -> Completed
        | InvokeScheduled({ promise_id: PromiseId,
                            kind: InvokeKind,
                            function_name: str,
                            input: Payload,
                            retry_policy: RetryPolicy })
        | InvokeStarted({ promise_id: PromiseId,
                          attempt: int })
        | InvokeCompleted({ promise_id: PromiseId,
                            result: Payload,
                            attempt: int })
        | InvokeRetrying({ promise_id: PromiseId,
                           failed_attempt: int,
                           error: str,
                           retry_at: Timestamp })

        // --- Category 3: Nondeterminism (Determinism Guarantee) ---
        // Formal basis: LTS determinism — entropy sources must be captured
        // Single-phase: just value capture
        | RandomGenerated({ promise_id: PromiseId,
                            value: str })
        | TimeRecorded({ promise_id: PromiseId,
                         time: Timestamp })

        // --- Category 4: Control Flow (State Reconstruction) ---
        // Formal basis: CSP trace semantics
        | TimerScheduled({ promise_id: PromiseId,
                           duration: Duration,
                           fire_at: Timestamp })
        | TimerFired({ promise_id: PromiseId })
        | SignalDelivered({ signal_name: str,
                            payload: Payload,
                            delivery_id: SignalId })
        // Signal consumed by workflow via await_signal() — has promise_id for replay cache
        | SignalReceived({ promise_id: PromiseId,
                            signal_name: str,
                            payload: Payload,
                            delivery_id: SignalId })
        | ExecutionAwaiting({ waiting_on: Set[PromiseId],
                              kind: AwaitKind })
        | ExecutionResumed

        // --- Category 5: Concurrency (Total Ordering) ---
        // Formal basis: Lamport — concurrent results need deterministic ordering
        | JoinSetCreated({ join_set_id: JoinSetId })
        | JoinSetSubmitted({ join_set_id: JoinSetId,
                             promise_id: PromiseId })
        | JoinSetAwaited({ join_set_id: JoinSetId,
                           promise_id: PromiseId,
                           result: Payload })

    // =========================================================================
    // Journal Entry — one row in the append-only log
    // =========================================================================
    // Maps to Rust's JournalEvent { sequence, timestamp, event }
    type JournalEntry = {
        sequence: int,
        timestamp: Timestamp,
        event: EventType,
    }

    // =========================================================================
    // Execution Status — journal state machine
    // =========================================================================
    // Running → Blocked ↔ Running → Cancelling → Terminal
    // Terminal states: Completed, Failed, Cancelled
    type ExecutionStatus =
        | Running                     // actively executing code
        | Blocked({ waiting_on: Set[PromiseId], kind: AwaitKind })    // waiting for promise(s) to resolve
        | Cancelling                  // cancellation requested, cleanup in progress
        | Completed                   // success (terminal)
        | Failed                      // error (terminal)
        | Cancelled                   // externally cancelled (terminal)

    // =========================================================================
    // Execution State — per-execution bundle
    // =========================================================================
    type ExecutionState = {
        journal: List[JournalEntry],
        status: ExecutionStatus,
        nextChildSeq: int,
        allocatedChildren: Set[PromiseId],
    }

    // All executions, keyed by their PromiseId
    var executions: PromiseId -> ExecutionState

    var clock: Timestamp

    // =========================================================================
    // Pure Helpers
    // =========================================================================

    // Construct a journal entry
    pure def mkEntry(seq: int, ts: Timestamp, evt: EventType): JournalEntry =
        { sequence: seq, timestamp: ts, event: evt }

    // Terminal states — execution is finished, no more events allowed
    pure val TERMINAL_STATES = Set(Completed, Failed, Cancelled)

    // Deterministic child PromiseId derivation (Dewey encoding)
    // parent.append(seq) — e.g., [1].append(0) = [1, 0]
    pure def childId(parent: PromiseId, seq: int): PromiseId =
        parent.append(seq)

    // Check if a promise_id has a resolution event in the journal
    pure def isResolved(journal: List[JournalEntry], pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | InvokeCompleted(e) => e.promise_id == pid
                | TimerFired(e) => e.promise_id == pid
                | SignalReceived(e) => e.promise_id == pid
                | _ => false
            }
        )

    // Check if a blocked execution's wait condition is satisfied
    pure def canResume(exec: ExecutionState): bool =
        match exec.status {
            | Blocked(b) => match b.kind {
                | Single => b.waiting_on.forall(pid => isResolved(exec.journal, pid))
                | All => b.waiting_on.forall(pid => isResolved(exec.journal, pid))
                | Any => b.waiting_on.exists(pid => isResolved(exec.journal, pid))
                | AwaitSignal(s) => isResolved(exec.journal, s.promise_id)
            }
            | _ => false
        }

    // Check if the event is a terminal event
    pure def isTerminalEvent(evt: EventType): bool =
      match evt {
          | ExecutionCompleted(_) => true
          | ExecutionFailed(_) => true
          | ExecutionCancelled(_) => true
          | _ => false
      }

    // Derive execution status by folding over journal
    // Only 7 event types change status; everything else passes through
    pure def deriveStatus(journal: List[JournalEntry]): ExecutionStatus =
        journal.foldl(Running, (status, entry) =>
            match entry.event {
                | ExecutionStarted(_) => Running
                | ExecutionAwaiting(e) => Blocked({ waiting_on: e.waiting_on, kind: e.kind })
                | ExecutionResumed => Running
                | CancelRequested(_) => Cancelling
                | ExecutionCompleted(_) => Completed
                | ExecutionFailed(_) => Failed
                | ExecutionCancelled(_) => Cancelled
                | _ => status
            }
        )

    // ----- Journal scanning helpers (for action guards) -----

    pure def hasCancelRequested(journal: List[JournalEntry]): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | CancelRequested(_) => true
                | _ => false
            }
        )

    pure def hasInvokeScheduled(journal: List[JournalEntry], pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | InvokeScheduled(e) => e.promise_id == pid
                | _ => false
            }
        )

    pure def hasInvokeStarted(journal: List[JournalEntry], pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | InvokeStarted(e) => e.promise_id == pid
                | _ => false
            }
        )

    pure def hasInvokeCompleted(journal: List[JournalEntry], pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | InvokeCompleted(e) => e.promise_id == pid
                | _ => false
            }
        )

    pure def hasTimerScheduled(journal: List[JournalEntry], pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | TimerScheduled(e) => e.promise_id == pid
                | _ => false
            }
        )

    pure def signalDeliveredCount(journal: List[JournalEntry], name: str): int =
        journal.indices().filter(i =>
            match journal.nth(i).event {
                | SignalDelivered(e) => e.signal_name == name
                | _ => false
            }
        ).size()

    pure def signalReceivedCount(journal: List[JournalEntry], name: str): int =
        journal.indices().filter(i =>
            match journal.nth(i).event {
                | SignalReceived(e) => e.signal_name == name
                | _ => false
            }
        ).size()

    pure def hasSignalDelivered(journal: List[JournalEntry], name: str, deliveryId: SignalId, payload: Payload): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | SignalDelivered(e) => e.signal_name == name and e.delivery_id == deliveryId and e.payload == payload
                | _ => false
            }
        )

    pure def hasJoinSetCreated(journal: List[JournalEntry], jsId: JoinSetId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | JoinSetCreated(e) => e.join_set_id == jsId
                | _ => false
            }
        )

    pure def hasJoinSetAwaited(journal: List[JournalEntry], jsId: JoinSetId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | JoinSetAwaited(e) => e.join_set_id == jsId
                | _ => false
            }
        )

    pure def hasJoinSetSubmittedFor(journal: List[JournalEntry], jsId: JoinSetId, pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | JoinSetSubmitted(e) => e.join_set_id == jsId and e.promise_id == pid
                | _ => false
            }
        )

    pure def hasJoinSetAwaitedFor(journal: List[JournalEntry], jsId: JoinSetId, pid: PromiseId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | JoinSetAwaited(e) => e.join_set_id == jsId and e.promise_id == pid
                | _ => false
            }
        )

    pure def hasSubmittedToOtherJoinSet(journal: List[JournalEntry], pid: PromiseId, jsId: JoinSetId): bool =
        journal.indices().exists(i =>
            match journal.nth(i).event {
                | JoinSetSubmitted(e) => e.promise_id == pid and e.join_set_id != jsId
                | _ => false
            }
        )

    pure def retryCount(journal: List[JournalEntry], pid: PromiseId): int =
        journal.indices().filter(i =>
            match journal.nth(i).event {
                | InvokeRetrying(e) => e.promise_id == pid
                | _ => false
            }
        ).size()

    // =========================================================================
    // Initialization
    // =========================================================================
    action init = all {
        executions' = Map(),
        clock' = 0,
    }

    // =========================================================================
    // Lifecycle Actions
    // =========================================================================

    // Create a new execution — first event is always ExecutionStarted
    action startExecution(eid: PromiseId, ts: Timestamp, digest: ComponentDigest,
                          input: Payload, parent: PromiseId, key: str): bool = all {
        not(eid.in(executions.keys())),
        executions' = executions.put(eid, {
            journal: List(mkEntry(0, ts,
                ExecutionStarted({ component_digest: digest, input: input,
                                   parent_id: parent, idempotency_key: key }))),
            status: Running,
            nextChildSeq: 0,
            allocatedChildren: Set(),
        }),
    }

    // Complete execution successfully — terminal
    action completeExecution(eid: PromiseId, ts: Timestamp, result: Payload): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        ExecutionCompleted({ result: result }))),
                status: Completed,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Fail execution — terminal
    action failExecution(eid: PromiseId, ts: Timestamp, error: str): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        ExecutionFailed({ error: error }))),
                status: Failed,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Request cancellation — not terminal yet, allows cleanup
    action requestCancel(eid: PromiseId, ts: Timestamp, reason: str): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        CancelRequested({ reason: reason }))),
                status: Cancelling,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Cancel execution — terminal
    action cancelExecution(eid: PromiseId, ts: Timestamp, reason: str): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            hasCancelRequested(exec.journal),
            exec.status == Cancelling,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        ExecutionCancelled({ reason: reason }))),
                status: Cancelled,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // =========================================================================
    // Side Effect + Nondeterminism Actions
    // =========================================================================

    // Schedule a side effect invocation — allocates child PromiseId
    action scheduleInvoke(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                          kind: InvokeKind, fname: str, input: Payload,
                          retryPolicy: RetryPolicy): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        InvokeScheduled({ promise_id: childPid, kind: kind,
                                          function_name: fname, input: input,
                                          retry_policy: retryPolicy }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq + 1,
                allocatedChildren: exec.allocatedChildren.union(Set(childPid)),
            }),
        }
    }

    // Mark an invocation as started (records attempt number)
    action startInvoke(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                       attempt: int): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            hasInvokeScheduled(exec.journal, childPid),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        InvokeStarted({ promise_id: childPid, attempt: attempt }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Mark an invocation as completed with its result
    action completeInvoke(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                          result: Payload, attempt: int): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            hasInvokeStarted(exec.journal, childPid),
            not(hasInvokeCompleted(exec.journal, childPid)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        InvokeCompleted({ promise_id: childPid, result: result,
                                          attempt: attempt }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Record a retry after a failed attempt
    action retryInvoke(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                       failedAttempt: int, error: str, retryAt: Timestamp): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            hasInvokeStarted(exec.journal, childPid),
            not(hasInvokeCompleted(exec.journal, childPid)),
            retryCount(exec.journal, childPid) < MAX_RETRY_ATTEMPTS,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        InvokeRetrying({ promise_id: childPid, failed_attempt: failedAttempt,
                                         error: error, retry_at: retryAt }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Capture a generated random value — allocates child PromiseId
    action generateRandom(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                          value: str): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        RandomGenerated({ promise_id: childPid, value: value }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq + 1,
                allocatedChildren: exec.allocatedChildren.union(Set(childPid)),
            }),
        }
    }

    // Capture the current wall-clock time — allocates child PromiseId
    action recordTime(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                      time: Timestamp): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        TimeRecorded({ promise_id: childPid, time: time }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq + 1,
                allocatedChildren: exec.allocatedChildren.union(Set(childPid)),
            }),
        }
    }

    // Schedule a timer — allocates child PromiseId
    action scheduleTimer(eid: PromiseId, ts: Timestamp, childPid: PromiseId,
                         duration: Duration, fireAt: Timestamp): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        TimerScheduled({ promise_id: childPid, duration: duration,
                                         fire_at: fireAt }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq + 1,
                allocatedChildren: exec.allocatedChildren.union(Set(childPid)),
            }),
        }
    }

    // Timer fires — runtime event, execution may be Blocked
    action fireTimer(eid: PromiseId, ts: Timestamp, childPid: PromiseId): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            not(exec.status.in(TERMINAL_STATES)),
            hasTimerScheduled(exec.journal, childPid),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        TimerFired({ promise_id: childPid }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // External signal delivered to execution — no child allocation
    action deliverSignal(eid: PromiseId, ts: Timestamp, signalName: str, payload: Payload): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        val deliveryId = signalDeliveredCount(exec.journal, signalName) + 1
        all {
            not(exec.status.in(TERMINAL_STATES)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        SignalDelivered({ signal_name: signalName,
                                          payload: payload,
                                          delivery_id: deliveryId }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Workflow consumes a signal — allocates child PromiseId
    action receiveSignal(eid: PromiseId, ts: Timestamp, childPid: PromiseId, signalName: str, payload: Payload): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        val deliveryId = signalReceivedCount(exec.journal, signalName) + 1
        all {
            exec.status == Running,
            signalDeliveredCount(exec.journal, signalName) >= deliveryId,
            hasSignalDelivered(exec.journal, signalName, deliveryId, payload),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        SignalReceived({ promise_id: childPid, signal_name: signalName,
                                         payload: payload,
                                         delivery_id: deliveryId }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq + 1,
                allocatedChildren: exec.allocatedChildren.union(Set(childPid)),
            }),
        }
    }

    // Workflow awaits promise(s) — transitions Running → Blocked
    action awaitExecution(eid: PromiseId, ts: Timestamp, waitingOn: Set[PromiseId], kind: AwaitKind): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            match kind {
                | AwaitSignal(s) => waitingOn.size() == 1 and waitingOn.contains(s.promise_id)
                | _ => true
            },
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        ExecutionAwaiting({ waiting_on: waitingOn, kind: kind }))),
                status: Blocked({ waiting_on: waitingOn, kind: kind }),
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Resume a blocked execution — writes ExecutionResumed to journal
    action resumeExecution(eid: PromiseId, ts: Timestamp): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            canResume(exec),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts, ExecutionResumed)),
                status: Running,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Create a join set for concurrent work
    action createJoinSet(eid: PromiseId, ts: Timestamp,
                        jsId: JoinSetId): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        JoinSetCreated({ join_set_id: jsId }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Submit a promise to a join set
    action submitToJoinSet(eid: PromiseId, ts: Timestamp,
                           jsId: JoinSetId, childPid: PromiseId): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            hasJoinSetCreated(exec.journal, jsId),
            not(hasJoinSetAwaited(exec.journal, jsId)),
            not(hasSubmittedToOtherJoinSet(exec.journal, childPid, jsId)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        JoinSetSubmitted({ join_set_id: jsId,
                                           promise_id: childPid }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // Consume a result from a join set
    action awaitJoinSet(eid: PromiseId, ts: Timestamp, jsId: JoinSetId,
                        childPid: PromiseId, result: Payload): bool = all {
        eid.in(executions.keys()),
        val exec = executions.get(eid)
        all {
            exec.status == Running,
            hasJoinSetSubmittedFor(exec.journal, jsId, childPid),
            hasInvokeCompleted(exec.journal, childPid),
            not(hasJoinSetAwaitedFor(exec.journal, jsId, childPid)),
            executions' = executions.set(eid, {
                journal: exec.journal.append(
                    mkEntry(exec.journal.length(), ts,
                        JoinSetAwaited({ join_set_id: jsId,
                                         promise_id: childPid,
                                         result: result }))),
                status: exec.status,
                nextChildSeq: exec.nextChildSeq,
                allocatedChildren: exec.allocatedChildren,
            }),
        }
    }

    // =========================================================================
    // Model checking bounds — small finite sets for state exploration
    // =========================================================================
    pure val EXEC_IDS = Set(List(1), List(2), List(3))
    pure val PAYLOADS = Set("p1", "p2")
    pure val DIGESTS = Set("d1")
    pure val FUNC_NAMES = Set("fn1", "fn2")
    pure val SIGNAL_NAMES = Set("sig1", "sig2")
    pure val JOINSET_IDS = Set("js1", "js2")
    pure val DURATIONS = Set(1, 2, 3)
    pure val ERRORS = Set("err1")
    pure val MAX_RETRY_ATTEMPTS = 3
    pure val IDEM_KEYS = Set("k1", "k2")
    pure val RETRY_POLICIES = Set("rp1", "rp2")

    // =========================================================================
    // Step action — nondeterministic dispatcher for model checking
    // =========================================================================
    action step = any {
        // --- Lifecycle ---
        nondet eid = oneOf(EXEC_IDS)
        nondet digest = oneOf(DIGESTS)
        nondet input = oneOf(PAYLOADS)
        nondet parent = oneOf(EXEC_IDS)
        nondet key = oneOf(IDEM_KEYS)
        all { startExecution(eid, clock, digest, input, parent, key), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet result = oneOf(PAYLOADS)
        all { completeExecution(eid, clock, result), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet error = oneOf(ERRORS)
        all { failExecution(eid, clock, error), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet reason = oneOf(ERRORS)
        all { requestCancel(eid, clock, reason), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet reason = oneOf(ERRORS)
        all { cancelExecution(eid, clock, reason), clock' = clock + 1 },

        // --- Side Effects (child-allocating: childPid derived deterministically) ---
        nondet eid = oneOf(executions.keys())
        nondet kind = oneOf(Set(Function, Http))
        nondet fname = oneOf(FUNC_NAMES)
        nondet input = oneOf(PAYLOADS)
        nondet retryPolicy = oneOf(RETRY_POLICIES)
        val childPid_si = childId(eid, executions.get(eid).nextChildSeq)
        all { scheduleInvoke(eid, clock, childPid_si, kind, fname, input, retryPolicy), clock' = clock + 1 },

        // --- Side Effects (referencing: childPid from already-allocated children) ---
        nondet eid = oneOf(executions.keys())
        nondet childPid_sti = oneOf(executions.get(eid).allocatedChildren)
        nondet attempt = oneOf(Set(1, 2, 3))
        all { startInvoke(eid, clock, childPid_sti, attempt), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet childPid_ci = oneOf(executions.get(eid).allocatedChildren)
        nondet result = oneOf(PAYLOADS)
        nondet attempt = oneOf(Set(1, 2, 3))
        all { completeInvoke(eid, clock, childPid_ci, result, attempt), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet childPid_ri = oneOf(executions.get(eid).allocatedChildren)
        nondet failedAttempt = oneOf(Set(1, 2, 3))
        nondet error = oneOf(ERRORS)
        all { retryInvoke(eid, clock, childPid_ri, failedAttempt, error, clock + 1), clock' = clock + 1 },

        // --- Nondeterminism (child-allocating) ---
        nondet eid = oneOf(executions.keys())
        nondet value = oneOf(Set("rand1", "rand2"))
        val childPid_gr = childId(eid, executions.get(eid).nextChildSeq)
        all { generateRandom(eid, clock, childPid_gr, value), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        val childPid_rt = childId(eid, executions.get(eid).nextChildSeq)
        all { recordTime(eid, clock, childPid_rt, clock), clock' = clock + 1 },

        // --- Control Flow (child-allocating) ---
        nondet eid = oneOf(executions.keys())
        nondet duration = oneOf(DURATIONS)
        val childPid_st = childId(eid, executions.get(eid).nextChildSeq)
        all { scheduleTimer(eid, clock, childPid_st, duration, clock + duration), clock' = clock + 1 },

        // --- Control Flow (referencing) ---
        nondet eid = oneOf(executions.keys())
        nondet childPid_ft = oneOf(executions.get(eid).allocatedChildren)
        all { fireTimer(eid, clock, childPid_ft), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet signalName = oneOf(SIGNAL_NAMES)
        nondet payload = oneOf(PAYLOADS)
        all { deliverSignal(eid, clock, signalName, payload), clock' = clock + 1 },

        // --- Control Flow (child-allocating) ---
        nondet eid = oneOf(executions.keys())
        nondet signalName = oneOf(SIGNAL_NAMES)
        nondet payload = oneOf(PAYLOADS)
        val childPid_rs = childId(eid, executions.get(eid).nextChildSeq)
        all { receiveSignal(eid, clock, childPid_rs, signalName, payload), clock' = clock + 1 },

        // --- Control Flow (await — domain-meaningful patterns) ---
        nondet eid = oneOf(executions.keys())
        val children_aw = executions.get(eid).allocatedChildren
        all {
            children_aw.size() > 0,
            any {
                // Single: wait on one child
                nondet child = oneOf(children_aw)
                all { awaitExecution(eid, clock, Set(child), Single), clock' = clock + 1 },
                // Any: wait on all children, first to resolve unblocks
                all { awaitExecution(eid, clock, children_aw, Any), clock' = clock + 1 },
                // All: wait on all children, all must complete
                all { awaitExecution(eid, clock, children_aw, All), clock' = clock + 1 },
                // AwaitSignal: wait on one child for a named signal
                nondet child = oneOf(children_aw)
                nondet sigName = oneOf(SIGNAL_NAMES)
                all { awaitExecution(eid, clock, Set(child), AwaitSignal({ name: sigName, promise_id: child })), clock' = clock + 1 },
            },
        },

        // --- Resume ---
        nondet eid = oneOf(executions.keys())
        all { resumeExecution(eid, clock), clock' = clock + 1 },

        // --- Concurrency ---
        nondet eid = oneOf(executions.keys())
        nondet jsId = oneOf(JOINSET_IDS)
        all { createJoinSet(eid, clock, jsId), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet jsId = oneOf(JOINSET_IDS)
        nondet childPid_stj = oneOf(executions.get(eid).allocatedChildren)
        all { submitToJoinSet(eid, clock, jsId, childPid_stj), clock' = clock + 1 },

        nondet eid = oneOf(executions.keys())
        nondet jsId = oneOf(JOINSET_IDS)
        nondet childPid_aj = oneOf(executions.get(eid).allocatedChildren)
        nondet result = oneOf(PAYLOADS)
        all { awaitJoinSet(eid, clock, jsId, childPid_aj, result), clock' = clock + 1 },
    }


    // =========================================================================
    // INVARIANTS
    // =========================================================================
    
    // INV-1:  The first event is always Execution Started
   val firstEventIsStarted = executions.keys().forall(eid =>
    val exec = executions.get(eid)
    exec.journal.length() == 0 or
      match exec.journal.nth(0).event {
        | ExecutionStarted(_) => true
        | _ => false
      }
  )

   /// INV-2: Journal sequence numbers are monotonically increasing (entry i has sequence == i)
  val journalMonotonicity = executions.keys().forall(eid =>
    val exec = executions.get(eid)
    exec.journal.indices().forall(i =>
      exec.journal.nth(i).sequence == i
    )
  )

    /// INV-3: Terminal events are always the last entry
    val terminalFinality = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            isTerminalEvent(exec.journal.nth(i).event) implies i == exec.journal.length() - 1
        )
    )

    /// INV-4: Stored status matches journal-derived status
    val statusJournalConsistency = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.status == deriveStatus(exec.journal)
    )

    /// S-3: At most one terminal event per journal
    val singleTerminal = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().filter(i =>
            isTerminalEvent(exec.journal.nth(i).event)
        ).size() <= 1
    )

    /// S-5: ExecutionCancelled requires preceding CancelRequested
    val cancelledRequiresRequested = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | ExecutionCancelled(_) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | CancelRequested(_) => true
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// INV-5: InvokeStarted requires prior InvokeScheduled for same promise_id
    val phaseOrdering = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | InvokeStarted(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | InvokeScheduled(s) => s.promise_id == e.promise_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// SE-2: InvokeCompleted requires preceding InvokeStarted for same promise_id
    val completedRequiresStarted = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | InvokeCompleted(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | InvokeStarted(s) => s.promise_id == e.promise_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// SE-4: No InvokeStarted/Retrying after InvokeCompleted for same promise_id
    val noEventsAfterCompleted = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | InvokeCompleted(e) => exec.journal.indices().forall(j =>
                    j <= i or match exec.journal.nth(j).event {
                        | InvokeStarted(s) => s.promise_id != e.promise_id
                        | InvokeRetrying(s) => s.promise_id != e.promise_id
                        | _ => true
                    }
                )
                | _ => true
            }
        )
    )

    /// SE-3: InvokeRetrying requires preceding InvokeStarted for same promise_id
    val retryingRequiresStarted = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | InvokeRetrying(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | InvokeStarted(s) => s.promise_id == e.promise_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// SE-5: Retry count per promise_id never exceeds MAX_RETRY_ATTEMPTS
    val retryBounded = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.allocatedChildren.forall(pid =>
            retryCount(exec.journal, pid) <= MAX_RETRY_ATTEMPTS
        )
    )

    /// CF-2: SignalReceived requires preceding SignalDelivered for same signal_name, delivery_id, payload
    val signalReceivedRequiresDelivered = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | SignalReceived(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | SignalDelivered(s) => s.signal_name == e.signal_name and s.delivery_id == e.delivery_id and s.payload == e.payload
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// CF-3: Each delivery_id consumed by at most one SignalReceived
    val signalConsumedOnce = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            exec.journal.indices().forall(j =>
                (i != j) implies not(
                    match exec.journal.nth(i).event {
                        | SignalReceived(e1) => match exec.journal.nth(j).event {
                            | SignalReceived(e2) => e1.signal_name == e2.signal_name and e1.delivery_id == e2.delivery_id
                            | _ => false
                        }
                        | _ => false
                    }
                )
            )
        )
    )

    /// CF-4: AwaitSignal waiting_on matches promise_id
    val awaitSignalConsistent = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | ExecutionAwaiting(e) => match e.kind {
                    | AwaitSignal(s) => e.waiting_on.size() == 1 and e.waiting_on.contains(s.promise_id)
                    | _ => true
                }
                | _ => true
            }
        )
    )

    /// CF-1: TimerFired requires preceding TimerScheduled for same promise_id
    val timerFiredRequiresScheduled = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | TimerFired(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | TimerScheduled(s) => s.promise_id == e.promise_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// JS-2: No JoinSetSubmitted after JoinSetAwaited for same join_set_id
    val noSubmitAfterAwait = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | JoinSetSubmitted(e) => exec.journal.indices().forall(j =>
                    j >= i or match exec.journal.nth(j).event {
                        | JoinSetAwaited(s) => s.join_set_id != e.join_set_id
                        | _ => true
                    }
                )
                | _ => true
            }
        )
    )

    /// JS-1: JoinSetSubmitted requires preceding JoinSetCreated for same join_set_id
    val submitRequiresCreated = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | JoinSetSubmitted(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | JoinSetCreated(s) => s.join_set_id == e.join_set_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// JS-4: JoinSetAwaited requires preceding InvokeCompleted for same promise_id
    val awaitedRequiresCompleted = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | JoinSetAwaited(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | InvokeCompleted(s) => s.promise_id == e.promise_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// JS-3: JoinSetAwaited requires preceding JoinSetSubmitted for same (join_set_id, promise_id)
    val awaitedRequiresMember = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            match exec.journal.nth(i).event {
                | JoinSetAwaited(e) => exec.journal.indices().exists(j =>
                    j < i and match exec.journal.nth(j).event {
                        | JoinSetSubmitted(s) => s.join_set_id == e.join_set_id and s.promise_id == e.promise_id
                        | _ => false
                    }
                )
                | _ => true
            }
        )
    )

    /// JS-6: Per join set, count of awaited <= count of submitted
    val consumeBounded = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        JOINSET_IDS.forall(jsId =>
            val awaited = exec.journal.indices().filter(i =>
                match exec.journal.nth(i).event {
                    | JoinSetAwaited(e) => e.join_set_id == jsId
                    | _ => false
                }
            ).size()
            val submitted = exec.journal.indices().filter(i =>
                match exec.journal.nth(i).event {
                    | JoinSetSubmitted(e) => e.join_set_id == jsId
                    | _ => false
                }
            ).size()
            awaited <= submitted
        )
    )

    /// JS-5: No two JoinSetAwaited for same (join_set_id, promise_id)
    val noDoubleConsume = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            exec.journal.indices().forall(j =>
                (i != j) implies not(
                    match exec.journal.nth(i).event {
                        | JoinSetAwaited(e1) => match exec.journal.nth(j).event {
                            | JoinSetAwaited(e2) => e1.join_set_id == e2.join_set_id and e1.promise_id == e2.promise_id
                            | _ => false
                        }
                        | _ => false
                    }
                )
            )
        )
    )

    /// JS-7: A promise_id appears in at most one join set
    val promiseSingleOwner = executions.keys().forall(eid =>
        val exec = executions.get(eid)
        exec.journal.indices().forall(i =>
            exec.journal.indices().forall(j =>
                match exec.journal.nth(i).event {
                    | JoinSetSubmitted(e1) => match exec.journal.nth(j).event {
                        | JoinSetSubmitted(e2) => e1.promise_id == e2.promise_id implies e1.join_set_id == e2.join_set_id
                        | _ => true
                    }
                    | _ => true
                }
            )
        )
    )

    /// INV-6: No duplicate promise_ids across executions
    val promiseIdUniqueness = executions.keys().forall(eid1 =>
        executions.keys().forall(eid2 =>
            eid1 == eid2 or
            executions.get(eid1).allocatedChildren.forall(c1 =>
                executions.get(eid2).allocatedChildren.forall(c2 =>
                    c1 != c2
                )
            )
        )
    )
}
