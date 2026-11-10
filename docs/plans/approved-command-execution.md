# User-approved command execution

## Current status and next task

**Next task: M1 — validate and freeze the existing implementation.**
After M1 passes, start M2.1 — the Unix native process backend. Do not describe the next task
as "return to Phase 2b" or "continue transport hardening."

Command execution is unavailable in the normal app. Core approval policy, non-executing host
preparation, backend-independent supervision, Unix journaling, in-process admission and opt-in
local/remote execution transport have implementations and tests. They are not yet a verified
native execution feature.

No native process-tree backend is implemented or selected. Windows durable journal publication,
cross-process ownership/reconciliation, Foundry tools and the command-specific app workflow
remain unfinished. Ordinary LocalHost and agent startup do not enable execution.

The latest patches have not been compiled, formatted or tested in the read/edit-block workflow.
Fake-process tests are not evidence of native Windows, macOS or Windows-to-WSL execution.

## Authoritative delivery sequence

This checklist is the source of truth for remaining work and completion. Execute M1 through M7
in order. The numbered stages below retain requirements and implementation history; they are
not a second scheduling system. Historical "still pending" and "next work" statements describe
their increment at the time and do not override this checklist.

| Milestone | Status | Successor |
| --- | --- | --- |
| M1 — Validate existing implementation | Next; validation evidence pending | M2.1 |
| M2 — Native execution backends and Windows journal | Not implemented | M3 |
| M3 — Cross-process ownership and reconciliation | Not implemented | M4 |
| M4 — Complete host-path verification | Blocked on M2 and M3 | M5; Phase 2 is closed |
| M5 — Native Foundry proposals | Not implemented; original Stage 3 | M6 |
| M6 — App workflow, history and Inspect | Not implemented; original Stage 4 | M7 |
| M7 — Full acceptance and enablement | Blocked on preceding milestones; original Stage 5 | Feature enabled |

### M1 — Validate and freeze the existing implementation

Scope:
- Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`.
- Perform the no-default-features core cross-checks and static-agent build described in
  CLAUDE.md.
- Fix concrete build, lint, test and review defects in the existing policy, supervisor,
  journal and transport implementation. Do not add another general foundation layer.

Exit evidence: record the revision, commands, targets and results below. Existing fake-process
tests establish the baseline, not native process correctness. Unavailable checks remain blocked.

After passing, freeze these interfaces except for defects tied to a specific requirement or
failing test. The next task is M2.1.

### M2 — Implement native execution and Windows journal durability

Complete these substeps in order:
1. **M2.1 — Unix backend:** implement safe process-group ownership on Linux/WSL and macOS,
   preserving ownership through root exit and cleanup without signalling recycled process IDs.
2. **M2.2 — Windows journal:** implement durable no-clobber publication and recovery behavior.
3. **M2.3 — Windows backend:** establish job ownership before target code runs.

Inspect dependency APIs before selecting adapters; preserve the workspace's unsafe-code
prohibition. Backends must clear the environment, preserve exact arguments, use null stdin,
refuse unsupported ownership before launch and initiate cleanup on Drop.

Exit evidence: actual-process tests for each backend cover launch, failed start, argument
boundaries, environment filtering, bounded output with continued draining, timeout, cancellation
and descendant cleanup. Windows journal tests cover publication and recovery failures.

Normal app and agent execution remain disabled. Next: M3, not another supervisor increment.

### M3 — Finish cross-process ownership and reconciliation

Scope:
- Establish exclusive canonical-project execution ownership across separate app/agent processes.
- Connect ownership to durable intent and recovery.
- Implement explicit reconciliation that establishes termination before releasing unresolved
  admission. If termination cannot be established, keep the project blocked.
- Never resend commands, restore approval or treat reconnection as reconciliation.

Exit evidence: multi-process tests demonstrate concurrent admission refusal, crash/restart
without replay, unresolved-state blocking across reconnection, and a new separately approved
attempt after confirmed reconciliation. Recovery UI belongs to M6.

Next: M4.

### M4 — Verify the complete host execution path; close Phase 2

Wire native backends into the existing services in a test harness, not normal app startup.
Verify:
- Native Windows local execution.
- Native macOS local execution.
- Actual Windows-to-WSL agent execution.
- Stop, timeout, disconnect, agent failure and orderly service shutdown.
- Executable/cwd changes between review and launch.
- Intent-write refusal, completion-write failure, no dispatch retry and no local fallback.

Exit evidence: record platform-specific results for the complete approved host path. Missing
hardware is a named blocker, not a reason to create more speculative hardening work.

**Passing M4 closes Phase 2.** Foundry, UI, chat persistence and app lifecycle integration are
M5/M6 work and cannot extend Phase 2. Next: M5.

### M5 — Native Foundry proposals; original Stage 3

Implement `propose_command`, native call/result transcript types, bounded assembly in both
Responses and chat-completions streams, successful terminal-completion checks, tool-only
responses, multiple calls, explicit unsupported-tool behavior and budget accounting.
History reconstruction and chain resets must never dispatch commands.

Exit evidence: mock-provider tests cover split, complete, malformed, incomplete and multiple
calls, call/result correlation and history reconstruction. Parsing creates proposals only.

Next: M6.

### M6 — App workflow, persistence and Inspect; original Stage 4

Deliver in this order:
1. Project-owned execution state, bounded history, restore-without-authorization and Inspect.
2. Exact review cards with Run, Edit, Reject and Stop.
3. Separate output-sharing preview, secret warning, omission/redaction and explicit continuation.
4. Chat/project/app cancellation and shutdown integration; post-command rescan, SCM refresh,
   cache invalidation and revalidation of proposal/Save/Undo preconditions.
5. Prompt, help and design-system safety copy.

Follow the detailed Stage 4 requirements below. Test through explicitly configured integration
paths; normal startup remains disabled until M7 passes.

Exit evidence: end-to-end proposal, review, approval, execution, observation and optional sharing
tests, including rejection and restored history that cannot launch anything.

Next: M7.

### M7 — Full acceptance and enablement; original Stage 5

Run the complete user workflow on Windows, macOS and actual Windows-to-WSL connections.
Cover edits, duplicate clicks, sibling proposals, rejection, cancellation, restart, output
privacy and existing Save/Undo and Source Control behavior. Run contributor checks again.

Exit evidence: recorded acceptance results for all supported execution locations and the
verification requirements below. Only then enable normal app/agent execution.

### Completion and scope rules

- Close a milestone only with evidence, not because code or tests were added.
- Record status as not started, in progress, blocked or passed. Name blockers precisely.
- A failed check produces a specific corrective task within its milestone, not a new phase.
- Later defects identify the failed requirement and affected milestone. They may block release,
  but do not restart the entire sequence or waive safety requirements.
- No automatic execution, always-allow mode, parallel command queue, action-block fallback or
  unrelated refactoring is added to this delivery.
- Native dependency/API decisions and unavailable platform testing are explicit blockers.
  Do not substitute more abstractions for those decisions or tests.

### Evidence ledger

Append results here as milestones advance. Include revision, platform/target, command or manual
scenario, result and any blocker. This ledger records verification, not implementation claims.

No milestone has recorded passing verification evidence yet.

## Contract

The assistant proposes. The user approves each exact invocation. Contextual executes on
the project's host and records the observed outcome.

Approval is not sandboxing. A subprocess can read or modify files outside the project and
use the network. Builds and tests execute arbitrary project code. Stop does not reverse
side effects, and command execution is not covered by Save/Undo.

There is no always-allow setting, automatic execution, automatic command retry or restored
authorization. Ordinary Markdown and incomplete model output are never executable.
Existing user-operated Source Control behavior is unchanged.

## Findings in the current implementation

- `Cmd` and `CommandRunner` are useful execution seams, but `SystemRunner` currently inherits
  most environment variables, captures output without a byte limit, and kills only its child
  on cancellation. Do not expose that behavior to assistant-proposed commands.
- `WorkspaceHost` already routes local and WSL operations correctly. Extend this interface,
  rather than launching from the Windows app for a WSL project.
- `SupervisedHost::call` retries read-only requests. Command dispatch must never use its retry
  path. Replacing an agent after preparation also invalidates approval.
- Remote notification method names are logged without payloads; notifications are not yet
  delivered as typed command progress.
- Agent cancellation aborts request tasks, and EOF/shutdown waits for in-flight work.
  Commands need cooperative process-tree cleanup and explicit disconnect semantics.
- Both Foundry pumps currently emit text, usage and Done only. EOF can produce Done without
  an observed successful terminal event. Neither pump currently assembles native tools.
- Saved transcripts and Inspect assume alternating user/assistant text messages.
- Chat completion currently requires non-empty reply text. Tool-only responses must be
  handled without fabricating assistant prose.
- Automatic read follow-ups must pause when a turn proposes executable actions.

## Requirements and implementation history

The original stage numbers below identify requirements, not the next task. Use the authoritative
M1–M7 sequence above to schedule work. Increment notes are chronological and may contain statements
superseded by later increments, especially within 2c.

### 1. Core policy

- Strict versioned arguments: purpose, direct program/arguments or explicit interpreter/script,
  project-relative working directory, finite timeout, permitted environment overrides.
- Parsing is not authorization and is independent of the transport.
- Existing canonical working directories must remain within the canonical project root.
  Host allowlist checks are additional, not replaced by this helper.
- One project-owned approval gate, shared across conversations.
- Exact snapshot comparison, single-use non-serializable approval, duplicate call protection,
  same-turn duplicate invocation rejection, no implicit queue.
- Running or unknown execution retains the project slot.
- Unit coverage of policy and directory validation, without claiming process integration.

The stage-1 `ReviewPlan` is a comparison value, not proof that a host has validated it.
Production plans must be built from the host preparation service. App-side conversion of
prepared snapshots into review/approval state is part of the later execution integration.

### 2a. Non-executing host preparation

Implemented in `commands::prepare`, behind `WorkspaceHost::prepare_command`:
- The app supplies the project root and project-instance identity separately from model JSON.
- Tool arguments remain JSON text inside the RPC envelope to preserve duplicate-field
  validation in core. Malformed and unsupported arguments fail closed.
- Native canonical root checks do not trim, expand or translate Windows/WSL paths.
  Symlinks cannot escape the allowlisted canonical project directories.
- A LocalHost's clones share a preparation epoch. A new host/agent has a different epoch.
- Remote preparation requires the explicit capability. The supervisor does not retry a
  failed preparation on a different agent.
- Direct executable lookup preserves argument boundaries and does not use a shell or PATHEXT.
  Windows direct commands are restricted to `.exe`/`.com`; implicit script launchers fail.
- Supported script preparation names the exact interpreter, flags and complete script.
  `cmd` remains unsupported until exact Windows command-line handling is implemented.
  POSIX shell modes are not silently translated for Windows.
- Environment policy `minimal-v1` allows only PATH, HOME, USER, LOGNAME, USERPROFILE,
  SystemRoot, WINDIR, TEMP, TMP, TMPDIR, LANG and LC_ALL from the host, plus the explicitly
  permitted model overrides. Parent CI and tool-specific options are not inherited.
- PATH excludes empty/relative entries. When no absolute entries remain, conventional
  host-platform directories are used. No login shell is executed to discover PATH.
- Complete filtered environment values are returned for exact snapshot comparison. They
  must remain in memory, not logs or saved chat history. PreparedCommand's Debug deliberately
  omits commands, paths, arguments and environment values.
- Preparation is not authorization, a launch guarantee or a sandbox. It does not inspect
  executable contents, stop filesystem races, or prevent a program from loading credentials
  and configuration from the user's home directory.

Do not route approved commands through the existing inherited-environment Git runner.

### 2b. Execution supervisor

`commands::execution` supplies backend-independent supervision:
- Consume a non-cloneable LaunchPermit. There is no API accepting model JSON as authorization.
- Require durable execution intent before dispatch. Failed or timed-out journal writes refuse
  launch; failed completion writes leave the stored intent unknown on restore.
- Re-prepare through the host and compare the exact reviewed snapshot, including host epoch,
  platform and environment-policy version. Failure refuses launch; there is no retry.
- Refuse remote project bindings in the in-process engine. Agent execution must not be
  simulated with a local runner alongside a RemoteHost.
- Use a separate ManagedCommandRunner interface. The existing Git SystemRunner cannot be
  accidentally supplied because it does not implement that interface.
- Retain 64 KiB of raw stdout and stderr independently while continuing to drain excess data.
  Progress uses bounded cumulative watch snapshots, not unbounded queues or lossy deltas.
- Render output as plain escaped text. Terminal control and bidi formatting characters are
  visible escapes, not terminal actions, Markdown or clickable links.
- Apply an execution deadline, bounded process-tree cleanup and bounded pipe draining.
  Unconfirmed cleanup produces Unknown and keeps the approval gate's project slot occupied.
- Handle explicit cancellation and dropped execution handles through a CancellationToken.
  Orderly application shutdown must cancel and await workers before stopping Tokio.
- Keep unshared output in memory. Persistable completion records contain observations and
  truncation information, not output contents or the complete inherited environment.

Remaining work inside 2b:
- Implement and inspect native process-tree adapters before exposing execution. Windows must
  establish job ownership before target code runs. Unix must preserve process-group ownership
  through root exit and cleanup without signalling a recycled process ID.
- Backends must clear the environment, preserve exact parameters, use null stdin, refuse
  unsupported ownership before spawn, and initiate cleanup on Drop.
- Implement Windows durable journal publication, cross-process ownership and reconciliation.
  Connect recovery observations to saved execution history and Inspect.
- Add actual-process tests. The supervisor tests use fake managed trees and do not verify
  OS process creation, environment inheritance, descendant termination or shutdown cleanup.

No native backend is selected by default. This is an intentional fail-closed integration
boundary, not a claim that stage 2b is complete.

#### 2b journal increment

`commands::journal::FileExecutionJournal` implements the write-ahead seam:
- Each consumed approval receives a fresh app invocation ID, separate from the provider call
  identity. The ID is correlation data, not authorization, and uses validated filename syntax.
- The journal lives outside the project and separately from automatically pruned saved chats.
- Intent records retain the exact reviewed request, resolved invocation, project location,
  platform and environment-policy name. They exclude approval and the complete environment.
  Arguments and scripts can themselves contain private data; Debug omits them.
- Completion records correlate both invocation and call IDs, retain observed status and output
  metadata, and never contain stdout/stderr contents.
- Versioned timestamped intent/completion files are immutable. Publication uses a synced private
  temporary file, an atomic no-clobber hard link, and a directory sync. Opening a new store also
  synchronizes ancestor directories. Unsupported filesystem operations refuse success.
- Directory/file creation uses private Unix permissions. Journal files are not followed through
  symlinks during ordinary recovery checks. This is app-owned storage, not a tamper-proof
  boundary against processes running with the same credentials or filesystem replacement races.
- A timed-out blocking journal write can still finish later. It cannot launch a process; a
  resulting intent without completion restores as unknown. Duplicate publication never retries
  execution or overwrites the first observation.
- Missing, malformed, mismatched and unreadable completions preserve an unknown intent.
  Corrupt/missing intents and orphan completions fail recovery, rather than becoming an empty
  successful history. Records are read with a 256 KiB limit; recovery listings are capped at
  1,024 intents and fail instead of silently truncating.
- No automatic pruning or deletion API removes replay protection. Explicit archival/retention
  policy remains future lifecycle work.
- Unix filesystem tests and fake-runner/real-journal supervisor tests are supplied. They do not
  simulate power loss or verify native process behavior. Windows refuses journal initialization
  before creating storage; its durability adapter remains pending.

#### 2b recovery and in-process admission increment

The execution engine now checks recovery under a synchronous host reservation:
- LocalHost instances and clones share one process-wide admission registry. Canonical project
  roots, not chat identities or project-open epochs, identify slots.
- `ExecutionEngine::start` refuses an occupied slot immediately. It never queues an invocation
  to run after another finishes. Start errors mean no worker was launched; callers must finish
  their approval-gate attempt as failed to start, not reuse its consumed approval.
- RemoteHost and SupervisedHost do not offer in-process admission. Even a mistakenly local
  project label cannot make an agent host execute with a local runner.
- Recovery runs before new intent publication. Missing recovery support, errors and timeouts
  refuse launch. A restored unknown invocation blocks a newly approved invocation as well as
  duplicate attempts.
- New intent records include the canonical project root. Older records without it remain
  readable, but unknown ones conservatively block their location rather than guessing at aliases.
- Confirmed completion releases the in-process reservation only when history is also confirmed.
  Unknown outcomes, unconfirmed history and dropped/panicked workers retain a blocked slot.
  Process status and history failure remain distinct observations.
- Reservation release does not approve or launch pending proposals. There is no generic reset
  or "assume stopped" operation.
- Tests cover independent gates/engines, missing or failed recovery support before intent or
  spawn, canonical-root aliases, local/WSL location separation, old unknown records, and
  dropped-worker reservations.

This is not cross-process ownership: separate app/agent processes still require an inspected
ownership mechanism before native execution is enabled. Remote dispatch, lifecycle reconciliation,
project closure/shutdown integration and saved-history/Inspect presentation also remain pending.
Recovery records never create permits or enqueue work.

#### 2c transport-hardening prerequisite

This increment does not add executable host endpoints or select a native backend:
- The agent discards duplicate active RPC IDs before dispatch, including duplicate IDs on
  shutdown requests. It preserves the original operation and its cancellation handle.
  It deliberately sends no second response with that ID: a duplicate's error must not be
  mistaken for the original operation's result.
- RPC active-ID protection is not durable invocation replay protection. Completed or
  cancelled RPC IDs are no longer active; future command endpoints must independently
  enforce single-use invocation IDs through their execution service and journal.
- Either reader EOF/error or writer write/flush failure invalidates RemoteHost, fails pending
  callers, and stops the opposite transport worker. Request registration and disconnection
  share a lock, preventing insertion into an already-cleared dead connection.
- Failed connections reject subsequent requests immediately. Exhausted numeric request IDs
  refuse new requests instead of wrapping and reusing an identity.
- Notification payloads and malformed wire lines are not copied into transport logs.
- Existing Source Control retry classification and process-runner behavior are unchanged.
  No command retry or execution path is introduced.

Deterministic in-memory tests cover active-ID duplicates, duplicate shutdown, preservation
of the original cancellation handle, reader EOF, independent write/flush failure while the
agent's output remains open, pending/later callers, and request-ID exhaustion. They have not
been run in the read/edit-block workflow and do not verify command execution or WSL behavior.

Still pending: typed bounded progress, one-shot approved dispatch, cooperative command
cancellation, outcome-unknown handling and reconciliation across connection loss. The agent's
existing task-abort cancellation must not be reused as proof of command process-tree cleanup.
Native process ownership and cross-process admission remain prerequisites for enabling Run.

#### 2c transport review: cancellation and RPC-ID reuse

Static review found a completion-ownership race in the active-request registry. Tokio abort
does not interrupt a task's current poll. After cancellation removes an RPC ID, a replacement
request can acquire that numeric ID before the old task reaches its completion bookkeeping.
Removing by RPC ID alone could then retire the replacement and publish the old response.

Completion now checks the registered Tokio task identity before removing the request or
publishing its response. The ownership check, removal and response enqueue share the registry
lock. A stale task cannot remove a newer cancellation handle or answer for its request.
This is connection-local response correlation, not durable invocation replay protection.

Deterministic unit tests exercise the late-completion interleaving directly, including
cancelled requests with and without an ID replacement and single-publication behavior.
The existing in-memory agent tests continue to cover duplicate active requests and cancellation
through the wire. These tests have not been run in the read/edit-block workflow.

No executable endpoint, native backend, protocol capability or app execution control is added
by this correction. Source Control dispatch and retry classification remain unchanged.

#### 2c first integration increment: protocol contract and local host dispatch

Protocol version 4 adds the execution wire contract without enabling executable RPC methods:
- Independently versioned dispatch, cumulative progress, cooperative Stop and final-result
  envelopes, with strict field shapes.
- App invocation ID, project root, project epoch and host epoch correlation.
- Explicit local/WSL location and exact reviewed preparation, including argument boundaries
  and the in-memory restricted environment.
- Distinct observed statuses, cancellation/timeout flags, stream metadata and history errors.
- A reserved `commandExecutionV1` capability, deliberately absent from the agent handshake.
- Stop's eventual response is a final observation after cleanup, not a signal acknowledgement.
- Output-bearing wire types deliberately omit Debug and are not saved-chat or sharing types.
  Defining serde types does not validate their semantic bounds or authorize execution.

`WorkspaceHost::start_command` consumes a non-cloneable LaunchPermit synchronously:
- Default hosts refuse before creating a worker. RemoteHost and SupervisedHost inherit this
  refusal; neither uses a retry path or app-side runner.
- `LocalHost::with_command_backend` constructs a fresh host with explicitly supplied managed
  runner and journal dependencies. It is not used by the app or normal agent startup.
- The configured host creates its execution engine with a clone of that exact host, preserving
  the allowlist and preparation epoch. New host construction invalidates old preparation.
- The existing engine retains synchronous process-wide admission, recovery, durable intent,
  exact re-preparation, bounded capture, cooperative Stop and unknown-outcome blocking.
- Existing Git/Source Control execution and retry classification are unchanged.

Added tests cover wire round trips and required binding fields; approval-consuming dispatch
through a trait-object host; stale-host refusal; cloned-host admission without queuing;
fake-process Stop and completion recording; WSL-to-local refusal; and disabled execution on
ordinary local, remote and supervised hosts. Agent contract tests also check that the reserved
RPC methods remain unavailable and the execution capability is not advertised.
These new tests have not been run in the read/edit-block workflow.

Still required for Phase 2c: validate wire sizes, identities, sequence/counter semantics and
result consistency at the receiving boundary; consume permits into a one-shot remote dispatch;
connect the agent-owned execution service and durable replay protection; implement bounded
progress delivery and cooperative cancellation; retain unknown outcomes on lost delivery or
completion; and test all of those paths with fake managed processes. Do not advertise the
capability merely because these DTOs and local service seams exist.

#### 2c wire-validation increment

`commands::wire` adds command-payload validation and observation state without enabling RPC
execution:
- Raw JSON decoding checks payload byte limits before deserializing and does not echo serde
  diagnostics containing private values. Prepared environment maps reject duplicate keys.
- Dispatch decoding checks invocation syntax, bounded identities, version, exact project/host
  binding and local/WSL location against an independently supplied expected binding.
- Preparation fields, argument totals and the restricted environment have explicit limits.
  Permitted overrides must match the requested values; unrelated environment names fail.
- Encoding consumes a LaunchPermit and preserves its exact parameters. Encoded bytes are
  ordinary transport data, not durable authorization. Receiving-side replay protection is
  still required.
- DecodedDispatch is data, not a LaunchPermit. Its exact-snapshot comparison must use fresh
  preparation from the execution host; parsing performs no filesystem access or launch.
- Stop decoding checks the full invocation/project/host binding but performs no cancellation.
- ObservationReceiver accepts bounded cumulative output and strictly increasing sequences.
  Counters, truncation, stream completion, plain-text escaping and result flags are checked.
  Final results must have a sequence greater than prior progress.
- Cumulative prefixes cannot be rewritten. A trailing provisional UTF-8 replacement character
  may change when a split code point is completed by later bytes.
- Malformed or mismatched observations permanently invalidate the receiver. A later success
  cannot clear that failure. Disconnection reports Unknown with only the last verified output;
  it cannot downgrade an already verified final result or establish termination.
- Known process status and history-write failure remain separate observations. The eventual
  remote admission owner must retain its slot when history is unconfirmed.

Tests cover permit encoding and snapshot preservation, binding mismatches, malformed requests,
environment policy, Stop identity, payload limits, cumulative output, split UTF-8, sequence
replay/wrap, terminal-stream changes, invalid result flags and unknown-on-disconnect behavior.
These tests have not been run in the read/edit-block workflow.

This is not yet a transport integration: command payload limits do not bound the existing
JSON-RPC line reader, and decoding through an intermediate Value would lose duplicate-key
evidence. The executable endpoint increment must connect bounded raw framing, one-shot
dispatch, agent-owned durable replay/admission, sequenced bounded output delivery and
cooperative cancellation. Execution capability advertisement and app execution remain off.

#### 2c connection increment: bounded framing and raw payload preservation

The agent and RemoteHost now use bounded, cancellation-safe newline framing:
- The reader owns partial-frame state across cancelled read futures.
- A 64 MiB general workspace-frame ceiling is enforced before copying excess bytes.
  Command payloads retain their additional, smaller codec limits.
- EOF inside a frame and invalid UTF-8 are transport errors, never complete requests.
- Framing failure is permanent for that reader; no suffix can be treated as a new frame.
- Reader errors also stop the agent's writer task instead of leaving it detached.

Incoming RPC envelopes are decoded separately from raw params/results:
- Duplicate envelope fields, invalid JSON-RPC versions, ambiguous request/response shapes
  and responses with both result and error are rejected.
- Explicit result:null remains distinguishable from an absent result.
- Agent method parameters and RemoteHost typed results are deserialized directly from raw
  JSON. They no longer pass through Value and lose duplicate-field evidence.
- Malformed agent envelopes invalidate RemoteHost and fail pending callers. They do not
  establish a successful or failed command outcome.
- Wire contents and payload-bearing deserializer diagnostics are not copied into logs.
- Existing active-ID ownership, cancellation behavior and Source Control retry classification
  remain in place. The agent still has no executable command endpoint.

Added tests cover exact limits, partial-read cancellation, split UTF-8, incomplete EOF,
raw request/response duplicate preservation, invalid envelopes, pre-dispatch agent rejection
and client connection invalidation. These tests have not been run in the read/edit workflow.

Phase 2c is still not complete. Remaining integration includes bounded outgoing progress,
agent-owned execution and durable replay protection, one-shot RemoteHost dispatch, cooperative
Stop, client-side unknown-outcome admission across reconnection, and fake-process connection
tests. Existing task-abort cancellation must not be applied to executable command workers.
No execution capability is advertised and no native backend or app execution is enabled.

#### 2c executable connection integration

The opt-in agent service and RemoteHost now connect the execution supervisor:
- `AgentExecutionService` owns a concrete native LocalHost, trusted execution location, managed
  backend and agent-owned FileExecutionJournal. No RemoteHost can be paired with a local runner.
- Preparation independently registers project/host epochs and the canonical admission root.
  Raw dispatch validates against that registry. The supervisor re-prepares the full snapshot
  after durable intent and before spawn.
- Accepted workers publish immutable invocation intent even when Stop arrives before spawn.
  Durable intent rejects duplicate execution after journal reopen; the service additionally
  retains bounded in-memory IDs, including synchronous refusals. Admission errors are not
  queued attempts and no client dispatch path retries them.
- Canonical native admission remains process-wide. Unknown process outcomes, unconfirmed
  history and interrupted workers retain their reservations.
- `serve_with_execution` is an explicit library integration seam, not ordinary agent startup.
  Only that connection advertises execution capability. Its process workers belong to the
  service, independently of RPC waiters.
- Outgoing connection buffering is eight bounded frames. Process output remains independently
  capped at 64 KiB per stream and continues draining. Cumulative watch snapshots and checked
  sequences permit skipped progress without losing retained prefixes. A saturated final-result
  queue or stalled writer closes the connection rather than fabricating completion.
- Stop validates the full invocation binding and returns the same final observation as dispatch,
  only after supervisor cleanup and completion recording. `$/cancel` on executable dispatch
  also requests cooperative Stop without removing the active RPC ID or aborting its waiter.
- Reader EOF/error, malformed framing, writer failure and shutdown cancel the execution scope.
  The connection awaits service cleanup before retiring RPC tasks. Dropping a connection future
  requests cancellation; orderly owners must still await shutdown before stopping Tokio.
- RemoteHost consumes one permit synchronously, requires an exact preparation cached on that
  connection, and binds location from trusted integration code. Generic RPC requests cannot
  dispatch commands on an execution-capable agent.
- SupervisedHost dispatch uses only its current already-reviewed connection. It never launches,
  reconnects, retries or falls back while consuming approval.
- Incoming progress is validated before watch coalescing. Invalid observations, RPC errors,
  lost delivery and unverified completion become Unknown with the last verified output.
- Client reservations are process-wide and keyed by location plus host-native canonical root,
  not connection/project epochs. Reconnection and fresh preparation cannot clear Unknown or
  unconfirmed history. There is no reset/assume-stopped API or automatic reconciliation.
- Service bindings/retained operations are capped at 32 projects, invocation memory at 1,024
  attempts, client unresolved reservations at 256, and executable connection requests at 64.
  Limits refuse work instead of evicting replay or unresolved-state protection.
- Unshared output stays in memory. No Foundry tools, UI controls or ordinary app execution
  configuration were added.

New Unix connection tests use fake managed process trees and a real file journal. They cover
approved dispatch, cumulative bounded output, independent-gate admission, exact parameter
boundaries, stale preparation, cooperative Stop, duplicate active RPC/invocation IDs, durable
duplicate protection after journal reopen, old-review refusal on a replacement connection,
and unresolved client admission after disconnection and agent cleanup.

These patches have not been compiled, formatted or tested in the read/edit-block workflow.
They are not evidence of actual WSL, Windows or macOS process-tree behavior. This increment is
the implementation baseline for M1. Native ownership and Windows durability belong to M2;
cross-process admission/reconciliation to M3; host-path verification to M4. App enablement
remains gated by M7. Do not restart the earlier 2c increments as a new work sequence.

### 2. Runner and local/remote host lifecycle

Add a separate approved-command path alongside the existing Git runner. Reuse `Cmd` and the
runner seam where practical, but do not silently change Git's environment or cancellation.

Preparation is host-side and performs no subprocess execution:
- Check the current root allowlist, project identity and host instance.
- Resolve root, cwd and executable on that host. Return exact paths and argument boundaries.
- Reject unsupported interpreters and implicit Windows `.cmd`/`.bat` launching.
  Scripts require explicit shell mode. Do not substitute interpreters or translate scripts.
- Construct the complete child environment using `env_clear` and an explicit host-owned
  allowlist. Do not run a login shell just to discover PATH before approval.
- Exclude application/API secrets, SSH-agent sockets, Git redirection variables, loader
  injection variables and tool-specific option variables.
- Stage 1 permits only CI=true|false|1|0, NO_COLOR=1 and TERM=dumb overrides.
  PATH and other host-owned base environment values are not model-overridable.
- Show the environment policy and all overrides before approval. The exact complete
  environment participates in snapshot comparison but is not persisted or logged.

Execution must:
- Revalidate preparation immediately before dispatch/spawn, including cancellation and
  project/host epochs. No silent re-preparation followed by launch.
- Consume an invocation ID once; retain duplicate protection through terminal results.
- Write an execution-intent history entry before dispatch. Failure to persist intent refuses
  launch; failure to persist a later completion leaves an honest unknown-on-restore record.
- Use null stdin, no elevation, bounded runtime and independently bounded stdout/stderr.
- Keep draining both pipes after capture limits; do not retain unlimited progress queues.
- Distinguish success, nonzero/signal failure, failed start, cancelled, timed out and unknown.
  Preserve exit code where known, bounded output, truncation flags and output-read errors.
- Use Unix process groups and Windows job objects through safe Rust interfaces compatible
  with the workspace's unsafe-code prohibition. Inspect dependency APIs before choosing them.
  Do not ship Windows execution with a race between spawn and containment.
- Explain limitations for deliberately detached descendants; containment is not a sandbox.
- Sanitize terminal controls incrementally across chunk boundaries. Never feed output into
  Markdown actions, hyperlinks or terminal escape interpretation.

Protocol changes require a version bump and an explicit command capability:
- Host preparation with agent-instance identity.
- Single dispatch with invocation ID, exact preparation binding and typed final result.
- Bounded, sequenced stdout/stderr progress correlated to that invocation.
- Cooperative cancellation that waits for cleanup rather than merely aborting the RPC task.
- Reject duplicate active RPC IDs instead of overwriting their task entries.
- Missing capability is non-executable, with no local fallback for a WSL project.
- Command dispatch never retries, even if delivery or completion is uncertain.
- A changed agent instance invalidates preparation; the supervisor may reconnect for new
  preparation, not transparently retry an already authorized command.

On Stop, chat cancellation, project closure or orderly shutdown, cancel active process trees
and invalidate unused approvals. Persist completion only when established.
On connection loss, mark outcome unknown and do not release the single-command slot by guess.
Reconciliation must establish termination before admitting another command; it must never
resend the invocation. Crash recovery restores observations, never approval.

### 3. Native Foundry transport

Use a native `propose_command` function tool with the versioned argument shape. No dedicated
action-block fallback in the first version. Unsupported deployments get an explicit capability
notice; text resembling an execution request remains inert.

Keep provider parsing separate from command validation and authorization:
- Responses: assemble function-call items by output item identity and call ID; validate the
  full arguments and successful response completion, not just an arguments-done event.
  Return `function_call_output` with the original call ID.
- Chat completions: assemble indexed `delta.tool_calls` across chunks, including split name,
  ID and argument fragments. Require the expected terminal finish reason and clean protocol
  completion. Reconstruct assistant `tool_calls` and correlated `tool` messages.
- Accept multiple complete calls, each pending its own user decision. Nothing runs as a batch.
- Bound call count, IDs and accumulated argument bytes before allocating unbounded buffers.
- Partial, malformed, ambiguous or unsupported calls remain non-executable.
- Record duplicate/colliding calls explicitly and correlate their non-execution result; never
  lose a call ID by merging it away. Stage-1 policy rejection is not a transport result.
- Detect tool support separately from Responses endpoint support. Do not use broad
  "unsupported" matching to downgrade a tool request silently.
- HTTP retries can obtain proposals, but can never authorize or invoke a host command.
  Preserve the logical turn identity across provider retries.

Extend transcript types rather than flattening native tool messages into prose.
Chain resets reconstruct calls and previously shared results as history data, never as work
to enqueue. Supplying history to either API must have no path to the execution dispatcher.
Count schemas, tool arguments and results in the existing logical input budget.

### 4. Chat, persistence, Inspect and UI

Read the current frontend skill, tokens, view implementation and relevant component specs
before implementing UI.

Cards must show:
- "Purpose (model-provided)".
- Exact program and argument boundaries, or the complete script and interpreter.
- Requested and resolved cwd, resolved executable, timeout and permitted overrides.
- Local Windows/macOS/Linux development host or the exact WSL distro.
- "Commands may modify files or access the network. Save/Undo does not undo command effects."

Use Run, Edit and Reject. Opening Edit immediately invalidates approval, including when an
edit is cancelled or invalid. Revised parameters require a fresh review.
Disable duplicate launches synchronously; while one command is running, other cards cannot
queue an approved execution. Stop must say that it does not reverse changes.

Use a separate Share output action in version one:
- Run does not send output to Foundry.
- Before sharing, warn: "Output may contain secrets or contents of files you did not select."
- Present exactly the bounded structured result to be shared, with an opportunity to omit
  output. Record omission/redaction explicitly rather than claiming it is complete.
- Rejection can be reported without output or execution; keep model continuation explicit.
- Return call identity, observed outcome/exit code, stdout/stderr, cancellation/timeout flags
  and per-stream truncation information as untrusted tool data.
- Sharing may request at most one model continuation per user action. Reuse cancellation and
  budget checks; commands in that continuation always return to Needs review.
- Handle unresolved sibling calls without fabricating results or violating provider ordering.

Persist observed history separately from runtime policy:
- Original call ID, app invocation ID, original proposal and user-edited invocation.
- Execution location, timestamps, observed status and whether output was shared.
- No approval token, host preparation capability or complete inherited environment.
- Bound output retention. Do not automatically persist unshared output; record when output
  was omitted. Warn that explicitly shared output becomes part of the saved transcript.
- Restored pending cards need new preparation/review; dispatching/running records with no
  confirmed result are unknown. No approval or execution is restored.
- Re-run creates a new execution attempt and fresh approval, preserving earlier history.
- Inspect shows native calls and correlated results, including rejection and omitted output.

After any possibly launched invocation, including unknown/cancelled/timed-out:
- Rescan project and refresh SCM.
- Invalidate file-version hashes, cached file contents and tree context.
- Keep pending proposals but revalidate their disk baselines; never silently apply them.
- Clear stale Save/Undo affordances where their preconditions no longer hold.

Update status-bar, welcome/help, prompts and design-system safety copy before enabling Run.
Use scoped wording: "File proposals are saved only with Save. Commands run only with your
approval; their effects are not covered by Save/Undo." Do not promise that every write is
undoable or that the entire app is read-only before Save.

### 5. Verification

Stage-1 unit tests cover argument validation, unchanged/edited/stale approval, duplicate
clicks and proposals, fresh follow-up approval, session replacement, host/location identity,
single-command admission, unknown-outcome blocking and real directory containment.

Still required before enabling the feature:
- Fake `CommandRunner` tests proving zero launches before approval and after rejection.
- Tests of each actual execution path, not just permit generation.
- Native-tool mock SSE for split arguments, multiple calls, incomplete/error EOF, malformed
  calls, call/result correlation, unsupported deployments and tool-only responses.
- Saved-history/Inspect round trips and chain-reset reconstruction without dispatch.
- Agent contract tests for correct local/WSL routing and no retry on disconnect.
- Real-process tests for stdout/stderr streaming, bounds, draining, timeout, cancellation,
  failed start, process-tree cleanup and terminal-control sanitization.
- Environment inheritance/filtering and executable/cwd changes between review and launch.
- Closing project/chat/app during preparation, dispatch, execution and result sharing.
- Native Windows/macOS and actual WSL agent validation.

Contributor checks:
`cargo fmt --all`
`cargo clippy --workspace --all-targets -- -D warnings`
`cargo test --workspace`

Cross-check the no-default-features core and static agent as described in CLAUDE.md.
A Linux policy test is not evidence of Windows job-object or WSL-disconnection behavior.
