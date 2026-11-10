# User-approved command execution

## Status

Stages 1 and 2a add core approval policy and non-executing host preparation.
LocalHost, RemoteHost and SupervisedHost expose `prepare_command`; protocol version 3 adds
the `commandPrepare` capability. Preparation validates native project containment, resolves
programs and supported interpreters, constructs the restricted environment, and reports a
host-instance identity. It never launches a subprocess.

Command execution is still unavailable in the app. Stage 2b now includes the backend-independent
execution supervisor and fake managed-process tests. It requires an explicitly supplied
process-tree backend and durable execution journal; neither has a production implementation
yet. Native runners, execution/progress/cancellation transport, native tools, chat persistence,
Inspect and action cards remain to be implemented.

The supplied tests have not been run in the read/edit-block workflow. The agent contract
tests exercise an agent on the test machine, not an actual Windows-to-WSL deployment.
No Windows, macOS or WSL execution behavior is verified by these patches.

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
- Remote notifications are currently logged, not delivered as typed command progress.
- Agent cancellation aborts request tasks, and EOF/shutdown waits for in-flight work.
  Commands need cooperative process-tree cleanup and explicit disconnect semantics.
- Both Foundry pumps currently emit text, usage and Done only. EOF can produce Done without
  an observed successful terminal event. Neither pump currently assembles native tools.
- Saved transcripts and Inspect assume alternating user/assistant text messages.
- Chat completion currently requires non-empty reply text. Tool-only responses must be
  handled without fabricating assistant prose.
- Automatic read follow-ups must pause when a turn proposes executable actions.

## Stages

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
- Implement the durable journal and connect it to saved execution history.
- Add actual-process tests. The supervisor tests use fake managed trees and do not verify
  OS process creation, environment inheritance, descendant termination or shutdown cleanup.

No native backend is selected by default. This is an intentional fail-closed integration
boundary, not a claim that stage 2b is complete.

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
