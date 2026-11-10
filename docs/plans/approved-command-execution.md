# User-approved command execution

## Current status and next task

**Where we are (2026-11-10): the code is complete. M1–M6 are implemented, WSL projects run
commands through the real agent, and the feature is now reachable in the real app through
an opt-in setting, "Let the assistant propose commands" (Settings → Commands, off by default,
read at startup). With it on: the model proposes, the user reviews a card and runs, edits,
rejects or stops, chooses what to share, and continues; the app cleans up on close and
rescans afterwards. With it off, which is the default, nothing changes.**

**What is not done is verification, not code** (M7): nobody has looked at the UI on screen,
it has never met a real Foundry deployment, macOS has never run, and Windows could not be
re-tested after M6.2. Until that is done the setting should be treated as experimental.

| Step | State |
| --- | --- |
| M1 Validation | Passed on the Linux development host |
| M2.1 Linux backend | Implemented; verified on Linux/WSL2 |
| M2.1 macOS backend | Implemented; type-checked for Darwin; **blocked: no Mac available to run it** |
| M2.2 Windows journal | Implemented; verified on Windows 11 / NTFS |
| M2.3 Windows backend | Implemented; verified on Windows 11 |
| M3 Cross-process ownership and reconciliation | Implemented; verified on Linux/WSL2 and Windows 11 |
| M4 Host path: Linux agent process | Verified |
| M4 Host path: native Windows | Verified |
| M4 Host path: Windows-to-WSL | Relay verified; **Windows-built client blocked by application control after one passing test** |
| M4 Host path: macOS | **Blocked: no Mac** |
| M5 Native Foundry proposals | Implemented; verified against a mock provider; **not yet against a live deployment** |
| M6.1 Execution state, history, restore, Inspect data | Implemented in core; verified on Linux and Windows |
| M6.2 Review cards: Run, Edit, Reject, Stop | Implemented behind the development switch; behaviour verified in the running app; **appearance not yet looked at** |
| M6.3 Output-sharing preview and continuation | Implemented behind the development switch; behaviour verified in the running app; **appearance not yet looked at** |
| M6.4 Lifecycle: cancellation, shutdown, rescan after a command | Implemented; verified in the running app |
| M6.5 Prompt, help and safety copy | Implemented; wording tested; prompt section verified in the running app |
| Agent execution for WSL projects | Implemented in the real agent (`--commands`); verified on Linux and through the real `wsl.exe` relay with the Linux-built client |
| Opt-in setting (`chat.proposeCommands`) | Implemented; off by default; verified in the running app (debug build) |
| M7 Acceptance | **Not started — needs a person and real hosts.** The setting makes the feature available to whoever turns it on; it stays off by default until acceptance |

Two named blockers keep M2 and M4 from being formally closed. Neither stops M5 or M6:
1. **No macOS host is available** (confirmed by the user on 2026-11-10). The macOS backend
   stays implemented but unverified. When a Mac is available, run the commands under "macOS
   verification". Do not rework the macOS code speculatively.
2. **Windows application control blocks locally built test binaries.** On 2026-11-10 the
   new `wsl_execution` test binary ran once and was then refused ("blocked by your
   organization's Device Guard policy"). This is a security control on the development
   machine and must not be worked around. Later the same day the rebuilt Windows core test
   binary was refused as well, so nothing after M6.2 was re-run on Windows. Running the Windows-built `wsl_execution` tests needs the
   machine owner to permit or sign the binary, or a Windows machine without that policy.

Code:
- `crates/core/src/commands/`: supervisor, journal, admission, agent service and the three
  native backends.
- `crates/core/src/llm/tools.rs`: the `propose_command` definition, `ToolCall`/`ToolResult`,
  bounded call assembly and transcript correlation. `llm/conversation.rs` declares the tool,
  assembles calls in both streams and sends calls and results as native items.
- `crates/core/src/commands/history.rs`: `CommandHistory`, the records saved with a chat,
  and `SharedResult`, exactly what the model is told about a call.
  `crates/core/src/commands/desk.rs`: `CommandDesk`, the in-memory review and approval state
  of one open project. `crates/core/src/chat.rs`: calls and results in the transcript,
  `plan_answers`, `record_command_round`, `restore_commands`. `crates/core/src/inspect.rs`:
  calls and results per round.
- App: `crates/app/src/state/hosts.rs` (the development switch), `state/project.rs` (the
  project owns the `CommandDesk`), `state/chat.rs` (proposals, preparation, run, stop, edit,
  reject, reconcile), `views/command_card.rs` (the card), `dev.rs` (debug automation).
  `crates/core/src/commands/card.rs` decides what a card shows and offers.
- `crates/agent/src/main.rs`: the agent's explicit execution mode,
  `contextual-agent --commands --location <local|wsl:NAME> [--journal <dir>]`. Started without
  arguments, as the app does by default, the agent cannot run anything. (The separate test
  harness binary that M4 introduced has been removed; the tests start the real agent.)
- Host-path tests: `crates/agent/tests/execution_host_path.rs` (a real agent process),
  `crates/core/tests/wsl_execution.rs` (the real `wsl.exe` relay; needs two environment
  variables, otherwise it reports that it skipped), and
  `commands::execution::tests::across_processes` (the in-process host).

A host gets a runner only through `LocalHost::with_command_backend`, and an agent only
through its `--commands` mode. The app uses either only when the user's setting is on at
startup (or, in a debug build, when `CONTEXTUAL_DEV_COMMANDS=1` is set for that run).

**Next concrete steps, in order:**
1. **Look at it.** Run a debug build with the switch on and go through a command by hand:
   the card, the editor, live output, Stop, the share preview, Continue, the status bar and
   the welcome note. All of it was written from the design system and compiled, never seen.
   Fix what looks wrong; expect layout issues.
2. **One real deployment.** Use a Foundry deployment that supports tools. The tool payload
   handling and the "tools not supported" detection were only ever run against a mock.
3. **M7 acceptance** on each supported location, by hand: edits, duplicate clicks, sibling
   proposals, rejection, cancellation, restart, output privacy, and that Save/Undo and Source
   Control still behave. Then decide about enablement. Open before that: the two named
   blockers below. The Windows app starting the agent with `--commands` inside a distro has
   not been run: it needs the Windows GUI build, which this development host cannot run.

Windows tests can be run from the Linux development host while application control allows
them: build with
`cargo test -p contextual-core --no-default-features --target x86_64-pc-windows-gnu --lib --no-run`
and start the printed `.exe` directly; WSL interop runs it on Windows.

Command execution is unavailable in the normal app. Foundry tools and the command-specific app
workflow remain unfinished. Ordinary LocalHost and agent startup do not enable execution.

Fake-process tests are not evidence of native execution. A Darwin type-check is not evidence
that the macOS backend works. A Linux-built client talking through `wsl.exe` is not evidence
for the Windows-built client.

## Authoritative delivery sequence

This checklist is the source of truth for remaining work and completion. Current authorized
implementation work is M2.1.
Complete M2's substeps in order, then M3 through M7. The numbered stages below retain
requirements and implementation history; they are not a second scheduling system. Historical
"still pending" and "next work" statements do not override this checklist.

| Milestone | Status | Successor |
| --- | --- | --- |
| M1 — Validate existing implementation | Passed on the Linux development host (ledger); native Windows/macOS runs not available | Required for M2 acceptance |
| M2 — Native execution backends and Windows journal | Implemented. Linux and Windows verified; macOS blocked on hardware, so not formally passed | M3 |
| M3 — Cross-process ownership and reconciliation | Implemented; verified on Linux and Windows; macOS code path unrun | M4 |
| M4 — Complete host-path verification | Implemented and verified on Linux and native Windows; Windows-to-WSL verified through the relay only; macOS blocked. Not formally passed | M5; Phase 2 is closed |
| M5 — Native Foundry proposals | Implemented; mock-provider evidence recorded; live deployment unverified | M6 |
| M6 — App workflow, history and Inspect | Implemented (all five parts) behind the development switch; exit evidence recorded; appearance unseen | M7 |
| M7 — Full acceptance and enablement | Not started — **next**; needs a person, a real deployment and the supported hosts; original Stage 5 | Feature enabled |

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
failing test. M2.1 implementation is the current user-directed work; missing validation must
not be interpreted as a reason to expand the existing foundations.

Policy corrections, committed with passing regression tests:
- Editing a proposal checks same-turn duplicate invocation policy while invalidating old approval.
- Beginning an invocation invalidates unused sibling approvals so they cannot form a queue.

Status: the checks above pass on the Linux development host (ledger). The one defect found was
a formatting difference in `execution.rs`, now corrected.

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

Normal app and agent execution remain disabled. Next after all of M2 passes: M3, not another
supervisor increment.

#### M2.1 implementation checklist and handoff

**Status: in progress. Linux process tree and launch adapter implemented and passing on
Linux/WSL2. macOS process tree and launch adapter implemented and type-checked only; running
them on a Mac is the next task. No normal-startup backend selected.**

| Deliverable | Implementation state | Verification state |
| --- | --- | --- |
| Inspect concrete Unix dependency APIs | Nix 0.29.0/0.31.3, shared_child 1.1.2, Tokio 1.54.1 `net::unix::pipe` | Static source review |
| Linux/WSL non-reaping root observation | Probes using Nix `waitid` with `WEXITED | WNOWAIT | WNOHANG` | Pass on Linux/WSL2 |
| macOS non-reaping root observation | Probes using Nix kqueue `EVFILT_PROC` / `NOTE_EXIT` | Type-checked for `aarch64-apple-darwin`; not run: needs macOS |
| Linux `ManagedProcessTree` | `linux::LinuxProcessTree` | Pass on Linux/WSL2 (glibc and musl) |
| Linux production `ManagedCommandRunner` | `linux::LinuxCommandRunner` | Pass on Linux/WSL2 (glibc and musl) |
| Exact launch: environment, argv, cwd, null stdin, no implicit shell | Done for Linux | Argument boundaries, exact environment, `/dev/null` stdin, cwd, own process group, no shell fallback for interpreter-less scripts |
| Failed start means nothing ran | Done for Linux: every fallible step precedes the launch | Missing program, non-executable file, exec-format error, missing cwd, inexact requests, no runtime |
| Bounded output, continued draining, timeout, cancellation | Done for Linux | Pass through the actual supervisor with the launch adapter |
| Descendant cleanup on Stop, timeout and Drop | Done for Linux | Pass with a descendant that holds both pipes |
| Linux group-cleanup confirmation | Conservative group-disappearance check | Early-root-exit test accepts Failure or honest Unknown; which one this host takes is not recorded |
| End-to-end local path with the native runner | Explicitly configured `LocalHost` in a test | Preparation, approval, revalidation, launch, journal: pass |
| macOS `ManagedProcessTree` | `macos::MacosProcessTree` | Type-checked for `aarch64-apple-darwin` only; **run on a Mac next** |
| macOS production `ManagedCommandRunner` | `macos::MacosCommandRunner` | Type-checked only; the shared `native_launch_tests` and the end-to-end test are built for macOS but have not run |
| Normal app/agent enablement | Disabled intentionally | Remains gated by M7 |

Linux launch adapter (`LinuxCommandRunner`) decisions:
- Starts the resolved absolute program directly with `std::process::Command`: `env_clear` then
  exactly `Cmd::env`, unchanged argv, the given cwd, `Stdio::null()` stdin, `process_group(0)`.
  A relative program, relative or missing cwd, NUL bytes, or an empty, `=`-containing or
  duplicate environment name is refused rather than reinterpreted. There is no `PATH` search.
- Output pipes come from `tokio::net::unix::pipe::pipe()` (Tokio `net`, enabled for the Linux
  target only). Both ends exist and the read ends are registered with the runtime before the
  launch; the write ends are switched to blocking mode for the child. After a successful
  launch only infallible steps remain, so there is no post-launch setup failure to report.
- The `Command` is dropped right after the launch. It holds the parent's copies of the write
  ends; keeping it would prevent end-of-file on both pipes.
- An ignored `SIGCHLD` (kernel auto-reaping) is detected through `SigIgn` in
  `/proc/self/status` and refused before launch. An unreadable `/proc` also refuses.
- `Cmd::timeout` and `Cmd::cancel` are left to the supervisor.

Known limits of the Linux adapter, to carry into M4/M7 acceptance:
- `SA_NOCLDWAIT` and wait-for-any-child reapers cannot be detected without unsafe code; they
  remain integration preconditions. Tokio's own child handling waits on specific children.
- Without `pre_exec` (unsafe) the child cannot be given a new session or have stray descriptors
  closed. It inherits a controlling terminal if the app has one, and any descriptor some other
  library opened without close-on-exec.
- `spawn` must run in a Tokio runtime with the I/O driver. A missing runtime is refused; a
  runtime without I/O would panic before launch.
- Process-group containment is not a sandbox; a descendant that leaves the group escapes it.

Dependency findings and decisions:
- Pin Nix `=0.31.3`. The Linux backend uses it as a target-specific normal
  dependency with `process` and `signal`; macOS retains a dev dependency for kqueue probes.
  This does not select a runner at startup. Its inspected safe wait, signal and kqueue
  interfaces require no unsafe code in this workspace.
- Both inspected Nix versions expose non-reaping `waitid` on Linux, but not on macOS.
  Do not assume the Linux observer compiles for Darwin.
- Nix 0.31.3 supplies safe kqueue wrappers on macOS. The probes install root observation before
  releasing a fixture-only stdin handshake, avoiding a short-lived-root registration race.
  That handshake is not a production launch protocol or an exception to null stdin.
- Do not select `shared_child` for group lifetime ownership: its public wait methods ultimately
  reap the root, and its non-reaping helpers are private.
- Neither `killpg(SIGKILL)` success nor root exit alone confirms descendant termination.
  The probes observe one known descendant independently: `/proc` terminal state on Linux,
  a registered exit event on macOS. They do not establish complete descendant coverage.
- Exclusive reaping ownership is essential. Never reap the root and then signal its old numeric
  PID/PGID. The production design must also address incompatible process-wide reapers or
  automatic child reaping before enabling launch.

Linux implementation in this increment:
- `LinuxProcessTree` adopts a never-reaped, exclusively owned group leader. The launch
  adapter establishes `process_group(0)` before target code runs and transfers ownership
  without calling `wait` or `try_wait`.
- `wait_root` polls `waitid` with WNOWAIT and never reaps. Cancellation of that future does not
  consume exit status or release the root identity.
- Cleanup sends SIGKILL while the root is still unreaped. Once root exit is observed, it removes
  destructive signalling authority before reaping.
- After reaping, only `killpg(group, None)` is used. Signal zero delivers no signal: ESRCH confirms
  group disappearance, while a persistent group or a permissions error refuses confirmation.
  Even if the numeric group is reused, this phase cannot send SIGKILL to it.
- Group disappearance is deliberately conservative. Orphan zombies can prevent confirmation on
  hosts that do not promptly reap them. A timeout therefore remains Unknown rather than treating
  signal delivery, pipe EOF, or root exit as proof of cleanup.
- Drop signals only while it still owns the unreaped root, then transfers root reaping to an OS
  thread that sends no further signals. Drop is cleanup initiation, not a terminal observation.
- Observation errors that invalidate ownership prevent further destructive signals. Integration
  still must exclude automatic child reaping and competing waiters; a pre-signal ownership check
  alone cannot make an incompatible process-wide reaper safe.
- Tests exercise repeated non-reaping observation, idempotent cleanup, actual bounded
  stdout/stderr draining through the supervisor, timeout, cancellation, early root exit and Drop.
  The early-root test accepts honest Unknown when group disappearance cannot be confirmed; that
  branch is not passing evidence of complete native descendant cleanup.

macOS implementation (unverified):
- Launch is the shared `unix_launch` code, identical to Linux. The kqueue is created before
  the launch, so nothing fallible follows it.
- `MacosProcessTree` registers a one-shot `EVFILT_PROC` event with
  `NOTE_EXIT | NOTE_EXITSTATUS` immediately after the launch and polls it without blocking.
  The event leaves the root unreaped; its data is decoded as a wait status.
- A root that exits before registration is recognised by the registration receipt (or the
  call) reporting ESRCH. Its status then needs a reap. In that one case the group is sent
  SIGKILL when the exit is observed, then the root is reaped with `try_wait`; this differs from
  Linux only in that the group kill precedes, rather than follows, `wait_root` returning.
- Cleanup sends SIGKILL while the root is unreaped, reaps with non-blocking `try_wait`, then
  uses only signal-zero checks. ESRCH alone confirms that the group is gone.
- EPERM from `killpg` is not fatal on macOS: it is neither a delivered request nor a
  confirmation. A group that keeps answering EPERM stays unconfirmed and ends as Unknown.
- The program must start with a Mach-O/universal magic number or `#!`, checked before launch,
  so that `posix_spawnp` cannot hand a non-executable-format file to `/bin/sh`.
- No SIGCHLD-disposition check exists on macOS (no `/proc`, and `sigaction` is unsafe).

macOS design assumptions to confirm on a Mac — each has a test that would expose it:
1. Registering `EVFILT_PROC` for an own, unreaped, already exited child reports ESRCH
   (`a_root_that_exited_before_the_watch_still_reports_its_status`).
2. `NOTE_EXITSTATUS` data is the wait status, and the event does not reap
   (`root_exit_is_observed_without_reaping`).
3. `killpg` on a group holding only the zombie root returns success, ESRCH or EPERM, and after
   the reap signal zero returns ESRCH promptly (every test that asserts a confirmed outcome).
4. Apple's `posix_spawnp` falls back to `/bin/sh` on ENOEXEC. If it does not, the header check
   is merely redundant (`failed_start_reports_that_nothing_ran` passes either way).
5. `[ /dev/stdin -ef /dev/null ]` and `ps -o pgid= -p $$` behave in macOS `/bin/sh` as they do
   on Linux (`launch_uses_null_stdin_the_given_cwd_and_its_own_group`; a test-only concern).

macOS verification, to run on a Mac:
- `cargo test -p contextual-core --no-default-features commands::execution -- --nocapture`
  (the `macos::tests`, `native_launch_tests`, `unix_ownership_tests` and end-to-end tests).
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.

Remaining work, in order:
1. **Next:** run the macOS verification above, fix failures, and record results in the ledger.
2. Resolved (see "Agent execution for WSL projects"): a group that holds only zombies now
   counts as gone on Linux, so an early root exit no longer ends as Unknown.
3. Decide whether Linux needs the same executable-header check: on a host where the standard
   library launches through `execvp` (glibc older than 2.29 with a working directory set),
   glibc also falls back to `/bin/sh`. It was left out because it would refuse `binfmt_misc`
   programs, including Windows executables under WSL. The test host does not fall back.
4. Close M2.1 once the macOS run is recorded. M2.2 is done; M2.3 is the current task.

Linux component/supervisor test command:
`cargo test -p contextual-core --no-default-features commands::execution -- --nocapture`

Probe command, to run on each applicable native host:
`cargo test -p contextual-core --no-default-features unix_ownership_tests -- --nocapture`

A Linux run does not execute the macOS tests. A Linux/WSL run does not verify the complete
Windows-to-WSL connection; that host-path verification remains M4.

#### M2.2 Windows journal

**Status: implemented; verified on Windows 11 (NTFS). Power-loss durability is not testable
here and rests on the reasoning below.**

Findings from a probe run on Windows 11 build 26300, NTFS, standard user:
- `std::fs::hard_link` onto an existing name fails with `AlreadyExists` (OS error 183). It is
  the same atomic no-clobber publication as on Unix.
- A directory cannot be opened with a plain `File::open`. With `FILE_FLAG_BACKUP_SEMANTICS` it
  opens; flushing it is refused on a read handle and succeeds on a write handle.
- Flushing a file is likewise refused on a read-only handle.
- `C:\` and `C:\Users` cannot be opened for writing by a standard user.
- Creating symbolic links is refused (OS error 1314) without developer mode or the privilege.
- Reading a file that another handle holds without sharing fails with OS error 32.

Implementation (`journal.rs`), using only safe standard-library calls:
- Publication is unchanged in shape: private temporary file, write, flush, no-clobber hard
  link, flush, remove the temporary name.
- On Windows the published record is reopened for writing and flushed, then the journal
  directory is flushed through a write handle with backup semantics. Either failing is an
  error; an intent that was linked but not confirmed flushed blocks as Unknown on recovery.
- On initialization the journal directory is flushed strictly. Ancestors are flushed until
  the first one the user cannot open for writing.
- Unix modes do not exist; privacy is inherited from the directory's ACL, which cannot be
  inspected without unsafe code. The default location is inside the user's profile.
- A filesystem without hard links or without directory flush fails closed.

What is and is not established:
- Established by tests on Windows: no-clobber publication, immutability, concurrent writers,
  corrupt/oversized/missing/locked records, temporary files ignored, failed publication,
  a removed or non-directory journal path, and the supervisor's journal integration.
- Not established: that a flushed link survives power loss. Windows documents no
  directory-flush contract. The design relies on NTFS logging metadata sequentially, so that
  a later flush on the volume commits the earlier link. Treat this as a stated assumption.
- Not exercised: symbolic-link rejection (the test account cannot create links; the test
  reports that it skipped), junctions, ReFS, FAT/exFAT and network shares.

#### M2.3 Windows backend

**Status: implemented; verified on Windows 11 with actual processes.**

Dependency inspection (sources read in the Cargo registry; the workspace forbids unsafe code,
so the unsafe Windows calls must live in a dependency, as they do in Nix on Unix):
- **`process-wrap` 10.0.1** (`std`, `job-object`, `creation-flags`): creates the process
  suspended, assigns it to a job object, then resumes its threads; `start_kill` is
  `TerminateJobObject`. A failed assignment or resume terminates the still-suspended process.
  Its own "wait for the job" returns on *any* completion-port message, not only on "no active
  processes", and its job handle is private, so it cannot confirm an empty tree. Its job is
  not kill-on-close in the `std` variant. Pinned with `=`.
- **`win32job` 2** (already in the lockfile through another dependency): creates a job with
  kill-on-close, assigns a process by handle and lists a job's member process IDs. It has no
  termination call.
- **`winsafe` 0.0.29** was considered for a safe `ResumeThread`: it cannot open a thread by
  ID, and using its `CreateProcess` would mean reimplementing the standard library's argument
  quoting and handle inheritance. Not used.
- `process-wrap` 10 requires Rust 1.87, so the workspace `rust-version` went from 1.85 to 1.87.
  That also allows `std::io::pipe`. One unrelated line (`logic/format.rs`) was changed to
  satisfy a Clippy lint that the new minimum enables.

Design (`execution.rs`, module `windows`):
- Two nested job objects. `process-wrap` runs a `post_spawn` hook after creating the process
  and before its own job assignment and resume. `ObservedJob` uses that hook to put the
  suspended process into our `win32job` job first. The process is therefore a member of both
  jobs before any of its code runs, and descendants inherit both. There is no
  spawn-then-assign window.
- Termination is `TerminateJobObject` on the inner job. Confirmation is the root's exit
  plus an empty member list on the outer job.
- The outer job is kill-on-close. Dropping the tree, or the app process dying, ends every
  member. Drop also calls the termination explicitly.
- A root terminated by the tree reports no exit code (Windows would report the code passed to
  the termination call). A root that exited by itself keeps its real code through cleanup.
- Launch: resolved absolute `.exe`/`.com` only (the standard library would run a batch file
  through `cmd.exe`), `env_clear` plus exactly the approved variables, the approved cwd, null
  stdin, `CREATE_NO_WINDOW`. Pipes (`std::io::pipe`), the job and the runtime check all
  precede the launch. Environment names that differ only by case are refused as duplicates.

Known limits, to carry into M4/M7 acceptance:
- Arguments reach a Windows program as one command line, quoted for the standard C runtime
  rules. A program with its own parser (`cmd.exe` is one) may read them differently.
- A member leaves the job's list when termination begins; its handles and open files can
  outlive that briefly.
- Job containment is not a sandbox. Work a member delegates to a service, a scheduled task or
  an already running process is outside the job.
- Handles that other code in the app made inheritable are inherited by the child.
- Nested jobs need Windows 8 or later.
- Tests ran with the test binary started through WSL interop, as a standard user. They have
  not been run from a native Windows shell or under the GUI app.

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

**Status: implemented; verified on Linux/WSL2 and Windows 11 (see the evidence ledger).**

Ownership:
- One lock file per project (hash of location and canonical root) in the journal directory.
  Unix takes `flock` through Nix; Windows opens the file without sharing. Both belong to one
  open handle, so a second attempt conflicts even inside one process, and the operating
  system releases the lock when the owning process dies. Platforms or filesystems with
  neither fail closed.
- The supervising worker takes it without waiting, after its in-process reservation and
  before the recovery check, and holds it until after the completion record. A refusal is
  reported as FailedToStart; no intent is written and nothing is launched.
- It excludes every process sharing the journal directory. Two users with separate journals
  are not excluded from each other.
- Rust's standard-library file lock (1.89) was not used: raising the minimum version again
  would have enabled a dozen unrelated lints. The workspace stays at 1.87.

Durable launch records (intent version 2):
- `<id>.launching.json` is saved before any start is attempted and names the backend's
  containment: owner-bound (Windows job), process group (Unix) or unverifiable (fakes).
  If it cannot be saved, nothing is launched, because its absence is later read as proof
  that nothing started.
- `<id>.launched.json` holds the Unix process-group number and, on Linux, the kernel boot
  identity. It is saved alongside supervision so a slow disk cannot delay Stop.
- Version 1 intents predate the marker and can never be reconciled as "never launched".

Reconciliation (`ExecutionJournal::reconcile`, `ExecutionEngine::reconcile`,
`WorkspaceHost::reconcile_commands`, RPC `workspace/commandReconcile`):
- Explicit only. It takes the project lock, so it fails while any process supervises or
  starts a command for the project.
- An unknown invocation is unblocked only on evidence: the launch marker was never written;
  the tree was owner-bound and its owner no longer holds the lock; the recorded process group
  no longer exists; or the host restarted since the launch.
- The evidence is saved as `<id>.reconciled.json`. The outcome stays unknown and no completion
  is fabricated. An unreadable launch or reconciliation record is never treated as absent.
- It never launches, retries, restores approval, or signals a process. A recorded group
  number is only ever observed with signal zero.
- Only when nothing remains unresolved is the in-process blocked slot released, and, on the
  remote client, only a reservation whose worker has ended (never one still dispatching).
- Reconnecting, re-preparing and fresh approvals change nothing by themselves.

Limits, to carry into M4/M7 acceptance:
- **A surviving Unix process group stays blocked.** After its owner died, the group can only
  be observed, not ended, because its number is no longer reserved. The project unblocks when
  those processes end, or after a reboot (Linux). The recovery UI (M6) must say so.
- A reused group number can keep a project blocked until the unrelated group ends. It can
  never unblock one wrongly.
- A crash in the instant between starting a Unix process and saving its group leaves that
  invocation permanently unresolved. There is deliberately no manual override.
- A Unix descendant that left the process group is not seen by the group check.
- macOS has no boot identity here, so a reboot there is recognised only by the group being
  gone.
- Owner-bound evidence relies on the Windows job being kill-on-close with a single handle.

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

**Status: implemented; verified as listed. Not formally passed because of the two named
blockers (macOS hardware; application control on the Windows-built relay test).**

| Requirement | Linux agent process | Native Windows (in-process) | Windows-to-WSL |
| --- | --- | --- | --- |
| Approved command runs once, output returned | Pass | Pass | Pass with both client builds |
| Stop ends the whole tree | Pass | Pass | Pass, Linux-built client |
| Timeout ends the whole tree | Pass | Pass | Pass, Linux-built client |
| Disconnect: agent cleans up, client stays blocked until reconciled | Pass | n/a | Pass, Linux-built client |
| Agent failure: blocked until its command is gone, nothing replays | Pass | Pass (killed supervising process, M3) | Pass, Linux-built client |
| Orderly shutdown stops a running command first | Pass | n/a | Pass for an idle agent, both client builds |
| Executable or directory changed after review | Pass | Pass | Same agent code as the Linux column |
| Intent-write refusal launches nothing | Fake-runner and in-process tests | Pass | Same agent code |
| Completion-write failure is not reported as a process failure | In-process test (M3) | Pass (M3) | Same agent code |
| No dispatch retry on another agent, no local fallback | Pass | n/a | Same client code |

How it was done:
- (As first done; the harness has since been replaced by the real agent's `--commands` mode.)
  The harness binary serves the agent protocol on stdio with this platform's native backend
  and a file journal. Tests start it as a separate process and connect a real `RemoteHost`.
- `wsl_execution.rs` starts the harness inside a real distro through `wsl.exe`. Built for
  Windows it is the app's client. Built for Linux and run inside WSL it reaches the same
  `wsl.exe` through interop, which exercises the real relay with the Linux build of the client.

Findings:
- **Killing `wsl.exe` takes the agent and its command down together.** WSL ended the whole
  Linux process tree of that session; the agent recorded nothing. A new connection was
  refused until reconciliation, which then released the project on "process group absent".
- **Closing the connection is different:** the agent notices, stops the command, records
  Cancelled and exits by itself.
- A killed agent *process* (inside the distro) leaves its command running, exactly as on plain
  Linux: blocked until that process group ends.
- The Windows-built client passed the run-and-report test in 0.64 s before the binary was
  blocked. Nothing about it failed; it simply could not be started again.

Known limits, to carry into M7 acceptance:
- Review detects a removed or re-pointed executable or directory. It does not detect new
  content at the same path: the plan records paths, not a content hash.
- The four relay tests other than run-and-report have not run with the Windows-built client.
- Nothing here has run under the GUI app or from a native Windows shell.

**Passing M4 closes Phase 2.** Foundry, UI, chat persistence and app lifecycle integration are
M5/M6 work and cannot extend Phase 2. Next: M5.

### M5 — Native Foundry proposals; original Stage 3

Implement `propose_command`, native call/result transcript types, bounded assembly in both
Responses and chat-completions streams, successful terminal-completion checks, tool-only
responses, multiple calls, explicit unsupported-tool behavior and budget accounting.
History reconstruction and chain resets must never dispatch commands.

Exit evidence: mock-provider tests cover split, complete, malformed, incomplete and multiple
calls, call/result correlation and history reconstruction. Parsing creates proposals only.

**Status: implemented; exit evidence recorded against a mock provider (ledger).**

What exists:
- **Opt-in.** `ConversationRequest::command_tool` is false by default and the app does not
  set it yet. With it off and no calls in the history, request bodies are byte-identical to
  before (the recorded parity test still passes).
- **The tool.** One function, `propose_command`, whose schema mirrors `CommandRequest`. It is
  declared non-strict: the schema guides the model, `commands::parse_arguments` decides.
- **Transcript types.** `ChatMessage` gained `tool_calls` (assistant) and `tool_call_id`
  with a `Tool` role. `ConversationRequest::tool_results` answers the calls of the last
  assistant message and is sent before the user's text; with results and no text, no user
  message is sent. Nothing is flattened into prose.
- **Events.** `ChatEvent::ToolCall` is emitted once per call after the response has ended,
  before `Usage` and `Done`. `ChatEvent::ToolsUnsupported` announces a deployment that refuses
  the declaration.
- **Assembly.** Responses: by output item identity, from added/delta/done events, checked
  against the completed response's own output list. Chat completions: by index, with
  identity, name and arguments each allowed to arrive in pieces.
- **Completion.** A call is usable only if the response completed: `response.completed`, or
  `finish_reason: "tool_calls"` *and* the stream's `[DONE]` marker. An arguments-done event,
  a cut-off stream or `length` never suffices.
- **Nothing is lost.** Unknown tool, non-JSON or oversized arguments, a colliding identity,
  disagreement between what was streamed and what was summarised, a call made when no tool
  was offered: each is reported with its identity and a `problem`, and can never become a
  proposal. More than 8 calls in one response fails the turn.
- **Bounds.** 8 calls per response, 256-byte identities, 64-byte names, 64 KiB arguments
  (never accumulated beyond that), 256 KiB per result.
- **Correlation.** Before anything is sent, every call in the transcript must be answered
  exactly once, directly after it, and no result may lack its call. A violation is a local
  refusal (`FoundryError::Rejected`), not a provider error.
- **Tool support is tracked on its own.** A refused declaration is recognised by a narrow
  match (the message must name tools and refuse them), checked before the existing
  "Responses API unsupported" match, so it is never read as a missing API. The turn then
  continues on the same API without the tool, after the notice. If the conversation already
  contains proposals, it fails explicitly instead of stripping them.
- **Budget.** The tool definition, calls and results count toward the logical input estimate.
- **No path to execution.** The `llm` module does not reference the execution engine. A
  first attempt, an HTTP retry, a chain reset and the chat fallback all only build JSON.

Limits, to carry into M6/M7:
- **Not verified against a live Foundry deployment.** The streamed tool payloads in the tests
  are written from the documented API shapes, not recorded. The refusal wording that marks
  tool support as missing is a guess at what deployments send. M7 must include a real run.
- A call whose arguments are not a JSON object is replayed in a rebuilt history with `{}` as
  its arguments, because a provider would reject the whole request otherwise. With a stored
  chain (`previous_response_id`) the provider keeps the original.
- A response that ended incomplete still reports its calls, flagged. Whether a stored
  incomplete response can be continued with results is not known; M6 should reset the chain
  after one.
- The app ignores both new events for now, and saved threads do not store calls or results.
- Existing behaviour is unchanged where it was already loose: a text reply whose stream ends
  without a terminal event still ends as `Done` with no finish reason.

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

#### M6.1 — Execution state, history, restore and Inspect

**Status: implemented in core; verified on Linux and Windows (ledger). No UI yet.**

Two kinds of state, kept apart on purpose:
- **Saved with the chat: observations** (`commands::history`). One `CommandRecord` per call:
  the provider's call identity scoped by an app-generated turn identity, the original
  proposal, every attempt with the exact request that ran (and whether the user edited it),
  the execution location, timestamps, the observed status and flags, and what the model was
  told. No approval, preparation, environment or output. At most 256 records per chat and 8
  attempts per proposal; only fully resolved records are evicted.
- **In memory only: review and approval** (`commands::desk`). `CommandDesk` belongs to the
  open project, not to a chat view. It holds the prepared plans, the approval gate and
  unshared output (the latest 8 results). It has no serialization.

Behaviour:
- A reply's calls enter the chat only through `record_command_round`, as records awaiting
  review, or as unusable ones that keep their identity. Recording prepares and starts nothing.
- Run is two explicit steps on the desk: `approve` of exactly the plan shown (synchronous, so
  a second click cannot create a second approval), then `begin` against a plan prepared again
  just now. Any difference returns the proposal to review.
- Opening the editor voids approval at once. One command runs at a time; a sibling's approval
  does not survive another launch; finishing starts nothing.
- A re-run is a new attempt with its own review and approval; earlier attempts are kept.
  A rejection is final for its proposal. The same invocation twice in one model turn is
  reviewed once.
- An unknown outcome blocks the desk until the host's reconciliation succeeds; after that
  the session is empty and everything needs fresh preparation and review.
- When the conversation continues, every unanswered call is answered truthfully
  (`plan_answers`): rejected; not run (unusable, or the user had not decided — which closes
  it); withheld (it ran and the user shared nothing: the model learns only that); or the
  outcome with exactly the streams the user chose. Nothing continues while a command runs.
  A round that fails leaves the chat untouched.
- `SharedResult` is the bounded structured result: call identity, status, exit code,
  cancellation and timeout flags, per-stream text with truncation and omission flags, whether
  the user edited the text, and whether the command was edited before running.
- Restore (`restore_commands`): an attempt without a recorded result becomes unknown and is
  marked interrupted. Nothing is approved or prepared, and the desk starts empty.
- Chats saved before this still load, and save byte-compatibly when they hold no commands.
- Inspect shows, per round, the model's calls with their current status and exactly what the
  model was told about earlier calls, labelled ("Rejected", "Ran; result not shared",
  "Outcome shared, stdout omitted", …).

Defect found and fixed while testing: the desk first gave every review its own identity in
the gate, which defeated the gate's rule that one invocation proposed twice in a model turn
is refused, and let a rejected proposal be put up for review again. Both now have tests.

Not done here (later parts): any UI; the share preview and secret warning; lifecycle
integration; the rescan after a command; safety copy. The app still does not set
`command_tool`, so none of this is reachable from the running app.

#### M6.2 — Review cards: Run, Edit, Reject, Stop

**Status: implemented behind the development switch. Behaviour verified in the running app
through debug automation; appearance not yet seen by anyone.**

Enablement:
- `CONTEXTUAL_DEV_COMMANDS=1`, debug builds only. It gives the in-process host this
  platform's native backend and a file journal (`CONTEXTUAL_DEV_COMMANDS_JOURNAL` to move it).
  Release builds ignore it. Projects served by an agent (WSL) stay without execution: the
  shipped agent has none. With the switch off nothing changes: the tool is not offered.

How it is wired:
- `Project` owns the `CommandDesk`. `ChatModel` offers the tool, records a reply's calls with
  `record_command_round`, and asks the host to prepare each open proposal; the plan goes to
  the desk for review. Preparation approves nothing.
- A reply that proposes commands pauses the automatic read follow-ups, and may have no text.
- **Run** takes the approval synchronously, has the host prepare the request again, and only
  if that matches what was shown consumes the approval and starts the command. The attempt is
  recorded when the host reports it started; output is followed live and kept in memory.
- **Edit** voids approval the moment the editor opens. The edited request is validated,
  prepared and put back up for review; the record keeps the original proposal and marks the
  attempt as edited.
- **Reject** is final for the proposal. **Stop** cancels the running command's scope.
- After an unknown outcome the card offers "Check whether it still runs", which calls the
  host's reconciliation; only a clean answer lets commands be reviewed again.
- Opening a saved chat calls `restore_commands`, notes an interrupted command, and prepares
  what still awaits review. A chat cannot be switched while its command runs.
- When the user sends the next message, every open call is answered: rejected, not run, or
  "ran, result not shared". Sending is refused while a command runs.
- What a card shows and offers is decided in core (`commands::card`, 8 tests); the view only
  renders it. The component is specified in
  `docs/design/contextual/components/CommandCard/README.md`. That spec is local: the design
  system here is a copy of an external artifact, which does not contain this component.

Looking at it:
`CONTEXTUAL_DEV_COMMANDS=1 cargo run -p contextual -- <project>` with a deployment that
supports tools, then ask for something that needs a command. Without a deployment, point
`AZURE_OPENAI_*` at a local mock that answers with a `propose_command` function call.

Not done here: the Share action and preview (M6.3) — a finished command's output is visible
on its card but can only be withheld from the model; rescan and Source Control refresh after
a command (M6.4); status-bar, help and prompt wording (M6.5). The Edit box is the request as
JSON, not a form. A WSL project shows cards that cannot be run.

#### M6.3 — Output sharing and explicit continuation

**Status: implemented behind the development switch. Behaviour verified in the running app
through debug automation; appearance not yet seen by anyone.**

- **Running sends nothing.** Sharing is its own decision, in two steps, and continuing is a
  third. None of them happens by itself.
- **Share result…** on a finished card (only while this session still holds the output)
  opens the preview: the warning "Output may contain secrets or contents of files you did not
  select."; a tick box and an editable text area per stream; and exactly the structured
  result that would be sent, updated as the user ticks and types. An unticked stream is
  recorded as omitted; text that differs from the capture is recorded as edited. What can be
  typed in place of a stream is bounded (128 KiB).
- **Use this result** stages it in memory. Nothing is sent. The card says what will be sent
  and offers **Don't share**.
- **Continue**, under the latest reply's cards, sends one request that answers every open
  call: staged results, rejections, "not run" for proposals the user had not decided on
  (which closes them), and "ran, result not shared" for the rest. The sentence next to the
  button says which (`chat::continuation_summary`). It needs no typed text. Sending a typed
  message does the same with the text added.
- The continuation is an ordinary turn: same cancellation, same budget check, and any
  commands in the reply come back as proposals awaiting review.
- After the model has its answers, staged results and held output are dropped. If the request
  fails, nothing is recorded and the staged result is still there.
- A result that shares no outcome ("rejected", "not run", "not shared") contains the call
  identity, that status and a sentence — no flags about how the command ended. (Found in the
  running app: the first version still sent `cancelled: false` and `timedOut: false`.)
- Shared output becomes part of the saved transcript, as the result of its call; the preview
  and the Continue row say so. Unshared output is never saved.

Not done here: sharing after a restart is impossible by design (the output is gone; the card
then offers only Continue, which reports "ran, result not shared"). There is no way yet to
share part of the conversation's commands and keep others open: one Continue answers all.

#### M6.4 — Lifecycle, and what happens after a command

**Status: implemented; verified in the running app through debug automation.**

Lifecycle:
- **Closing the project** drops the chat model, and with it a guard that cancels the running
  (or still launching) command's scope. The host ends the process tree and records the
  outcome; nothing keeps running unobserved.
- **Quitting the app** cancels the scope the same way and then waits, for at most six
  seconds and only when a command is in flight, until the host reports it has finished with
  the command. The framework itself waits only 200 ms for quit handlers, so the wait is done
  inside the handler.
- **Switching chats** is still refused while a command from the current chat runs, with a
  message. That is a deliberate simplification: the result belongs to that chat.
- In both closing cases the host's journal has the confirmed outcome, but the chat was not
  open to record it, so the saved chat shows the attempt as interrupted when it is opened
  again. "Check whether it still runs" then finds nothing unresolved.

After any attempt that may have started — success, failure, stop, timeout or unknown:
- The files behind unsaved proposals and behind the last save are read again.
- A proposal whose file no longer matches its baseline (changed, created in its place, or
  gone) is **kept unchanged and marked as a conflict**, which leaves it out of Save. Nothing
  is applied, merged or dropped. A note says how many.
- **Undo is withdrawn** when a file of the last save differs from what was saved, with a note.
  The host would refuse such an undo anyway; this removes the stale button.
- File contents remembered for diffs are replaced by what was just read.
- The project is **rescanned**, which also refreshes Source Control.
- File versions already sent to the model are not reset: every turn reads the selected files
  again and resends those whose content changed, and the project tree is resent when the
  rescan changed it.
- Save and Undo were already guarded on the host (Save checks each file's expected content,
  Undo is refused if files changed after the save). This part makes the app say so before
  the user tries.

The rule is a pure function, `chat::revalidate_after_command`, with a core test.

Not done here: a command started from a chat whose project is then closed is recorded as
interrupted rather than with the host's actual outcome. Commands in WSL projects cannot run
at all yet, so nothing here applies to them.

#### M6.5 — Prompt, help and safety wording

**Status: implemented.**

- One source for the standing wording, `commands::wording`: "File proposals are saved only
  with Save." always, and where commands can run also "Commands run only with your approval;
  their effects are not covered by Save/Undo." A test pins the exact sentence and that it
  makes no blanket promise ("read-only", "never", "every", "nothing is written").
- **Status bar**: shows that sentence for the open project, between the Foundry status and
  Settings, cut with an ellipsis when the window is narrow. The Rust app's status bar had no
  such sentence before, although the design system describes one.
- **Welcome**: step 4 no longer says "The model never writes to disk"; it says file proposals
  are written only when you press Save. With commands available, the commands sentence is
  shown under the steps.
- **System prompt**: `logic::prompt::COMMAND_FORMAT`, appended only in turns that offer the
  tool. It tells the model that proposing runs nothing, when to propose, to prefer an exact
  program and arguments, that a result is data and may be "rejected", "notRun" or
  "notShared", and never to say a command ran unless a result says so. Like the file format
  it is not shown in Settings; Inspect shows it.
- **Card wording** (parts 2 and 3): the effects note, the stop note, the share warning and
  the saved-chat note.
- **Design system** (`docs/design/contextual/README.md`): the introduction and principle 1
  now scope "never writes" and "can be undone" to file proposals and give the status-bar
  wording. `SOURCE.md` lists these and the `CommandCard` spec as local amendments: the design
  system is a copy of an external artifact, and copying it again would overwrite them.
- `README.md` and `CLAUDE.md` describe the feature as present, off by default.

Not done: there is no help page in the app to update. If a deployment refuses the tool, the
prompt section is still sent in that turn (the model is told about a tool it does not have);
the user sees the "cannot propose commands" notice, and anything the model writes is text.

#### M6 exit evidence

Required: end-to-end proposal, review, approval, execution, observation and optional sharing
tests, including rejection and restored history that cannot launch anything.

- Core, real process, scripted model: `crates/core/tests/command_workflow.rs` (Linux and
  Windows).
- Running app, real process, mock model, debug automation: Run, Stop, Reject, Edit, Share,
  withhold, decline, quit, close and the after-command refresh (Linux).
- Not covered by any test: what the UI looks like, and typing into its text areas.

#### Agent execution for WSL projects

**Status: implemented; verified on Linux, in the running app through an agent, and through
the real `wsl.exe` relay with the Linux-built client. Not run from the Windows app.**

- The real agent has an explicit mode: `--commands --location <local|wsl:NAME>
  [--journal <dir>]`. It then serves the execution service with the native backend and a
  durable journal in the agent's data directory. Incomplete or unknown arguments are an
  error, never a quiet fallback in either direction. Without arguments nothing changes: the
  agent prepares commands (inspection only) but offers neither execution nor reconciliation;
  a test pins that.
- The location comes from whoever starts the agent. `WslLauncher` passes the distro it
  launches into, and only when asked (`commands`); a distro name that is not plainly safe in
  a shell command leaves execution off. The connection is bound to that location.
- The app asks for it only under `CONTEXTUAL_DEV_COMMANDS=1` (debug builds). Without the
  switch nothing is passed. With it, an agent that predates the mode ignores the arguments
  and serves without execution, so its cards cannot be run.
- From there it is the same path as M4/M6: prepare on the agent, review and approve in the
  app, dispatch over the reviewed connection, observe, reconcile over the connection.

**Finding: zombies kept a project blocked inside WSL.** When the agent is killed while a
command runs and the command's processes are then ended, they often stay as unreaped zombies:
their adoptive parent is a WSL session `init` that lingers and does not wait for them. The
process-group check counted them as "still exists", so reconciliation stayed blocked
indefinitely (seen in about half the runs). A zombie cannot execute. On Linux the check now
reads the group's members from `/proc` and treats a group with no member that can still run as
gone (two readings must agree). A reused group number with live members still reads as
present. This also resolves the open M2.1 item: a root that exits and leaves a descendant now
ends deterministically as confirmed, not "Unknown" on hosts that do not reap promptly. macOS
has no `/proc` and keeps the stricter check.

Leftover from testing: the killed-agent tests leave a few zombie entries under those lingering
WSL `init` processes on the development machine. They hold no resources beyond a process-table
entry and disappear when the distro restarts.

#### The opt-in setting

**Status: implemented; verified in the running app.**

Decision, made on 2026-11-10 at the user's direction to complete the code: the feature is
reachable in normal builds through an explicit setting instead of only a debug-only
environment variable. This departs from the original sequence, which kept every build
disabled until M7 acceptance had passed. What preserves the intent of that rule:

- **Off by default.** `chat.proposeCommands` is false in new and in existing settings.
  A test pins that settings saved before the option existed leave it off.
- **Explicit.** One switch in Settings → Commands, with the effects note, "Running a command
  sends nothing to the assistant", and "Takes effect the next time you start Contextual".
- **Nothing else relaxed.** Every command still needs review and approval of the exact
  command; there is no always-allow; a restored chat still holds no approval.
- It is read once at startup, so a running session cannot be switched on from under the user.
- `CONTEXTUAL_DEV_COMMANDS=1` still turns it on for one run in debug builds only.

Verified in the running app (debug build, no environment switch): with the setting on, the
tool was offered and an approved command ran; with it off or absent, the tool was not offered
and nothing ran. A release build compiles; the setting was not exercised in a release build,
because the automation that drives the app exists only in debug builds.

M7 is what turns "available to whoever opts in" into "accepted": it should end either with
the experimental wording removed or with the default reconsidered. Neither is decided here.

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

Revision for the 2026-11-10 rows: `b6c182e` plus the uncommitted Unix native backends
(`crates/core/src/commands/execution.rs`, `crates/core/Cargo.toml`, `crates/core/src/host.rs`). Platform: x86_64 Linux,
WSL2 kernel 6.18.33.2, rustc 1.99.1.

| Scope | Revision / platform | Work or check | Result / blocker |
| --- | --- | --- | --- |
| M2.1 API inspection | Supplied dependency source snapshots | Reviewed Nix wait/signal/kqueue APIs and shared_child public/private waiting behavior | Static review completed; not native execution evidence |
| M2.1 native ownership probes | User-reported earlier run; revision/platform not supplied | Root exit without reaping, group cleanup of a known descendant, cancellation while root runs | Superseded for Linux by the row below; macOS still unrecorded |
| M1 contributor checks | 2026-11-10, Linux/WSL2 | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` | Pass: 515 tests, 0 failed. At bare `b6c182e` the same checks passed except one formatting difference |
| M1 cross-target checks | 2026-11-10, Linux/WSL2 host | Core `--no-default-features` Clippy for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`; musl release build of the agent | Pass. Type-checks only |
| M2.1 Linux ownership probes | 2026-11-10, Linux/WSL2 | `unix_ownership_tests` (2 Linux tests) | Pass |
| M2.1 Linux tree and launch adapter | 2026-11-10, Linux/WSL2, glibc and musl targets | `linux::tests` (6) and `native_launch_tests` (10): exact argv, exact environment, null stdin, cwd, own group, failed start, refusals, bounded output, timeout, cancellation, descendants, Drop | Pass; 20 consecutive runs of `commands::execution` without a failure after the shared-code restructuring |
| M2.1 Linux end-to-end local path | 2026-11-10, Linux/WSL2 | `approved_command_runs_once_through_the_native_runner` | Pass |
| Linux checks, independent run | 2026-11-10, the user's main checkout on Linux | Workspace tests | User reports passing |
| M2.1 macOS type-check | 2026-11-10, cross-check from Linux | `cargo clippy -p contextual-core --no-default-features --lib --tests --target aarch64-apple-darwin -- -D warnings` | Pass. Library and test code compile for Darwin; nothing was linked or run |
| M2.1 macOS execution | — | `macos::tests`, `native_launch_tests`, ownership probes, end-to-end test | Blocked: no macOS host has run them. Design assumptions are listed under M2.1 |
| M2.2 Windows probe | 2026-11-10, Windows 11 build 26300, NTFS, standard user | Standalone probe of hard-link, flush, sharing and symlink behaviour | Findings listed under M2.2 |
| M2.2 Windows journal | 2026-11-10, same host; `x86_64-pc-windows-gnu` test binary started through WSL interop | `commands::journal::tests` (22 on Windows) and the three supervisor journal tests in `commands::execution::tests` | Pass; all 104 `commands` tests pass; 15 consecutive runs without a failure. The symbolic-link test skipped itself |
| M2.2 cross-checks | 2026-11-10, Linux/WSL2 | Workspace format, Clippy and tests; Clippy for `x86_64-pc-windows-msvc`, `x86_64-pc-windows-gnu` (with tests) and `aarch64-apple-darwin` (with tests); musl agent build | Pass: 519 tests, 0 failed |
| M2.3 dependency inspection | 2026-11-10 | Read `process-wrap` 10.0.1, `win32job` 2.0.3/2.1.0 and `winsafe` 0.0.29 sources | Findings listed under M2.3 |
| M2.3 job-object probe | 2026-11-10, Windows 11 build 26300, standard user | Standalone probe: assignment while suspended, member list, termination, kill-on-close, failed setup | Suspended process listed before running; descendants listed; list empty within 0.1 ms of termination; a process whose setup failed never ran |
| M2.3 Windows backend | 2026-11-10, same host; `x86_64-pc-windows-gnu` test binary through WSL interop | `commands::execution::windows::tests` (11 plus the helper) and `approved_command_runs_once_through_the_native_windows_runner`: exact argv incl. quotes, backslashes and shell metacharacters; exact environment; cwd; null stdin; failed start; batch files and non-images refused; inexact requests; bounded output; Stop and timeout with a descendant; Drop; kill-on-close; root exit with a surviving descendant | Pass; all 116 `commands` tests pass on Windows; 12 consecutive runs without a failure |
| M2.3 cross-checks | 2026-11-10, Linux/WSL2 | Workspace format, Clippy, tests (519, 0 failed); Clippy for `x86_64-pc-windows-msvc`, `x86_64-pc-windows-gnu` and `aarch64-apple-darwin` (the last two with tests); musl agent build; `cargo check -p contextual --target x86_64-pc-windows-gnu` | Pass |
| M3 journal ownership and reconciliation | 2026-11-10, Linux/WSL2 and Windows 11 (interop) | `commands::journal::tests`: exclusive ownership between independently opened journals; never-launched, pre-marker, owner-bound, unverifiable, process-group present/absent, restarted host; scope; unusable records; immutability | Pass on both (the two process-group tests are Unix-only) |
| M3 separate processes | 2026-11-10, Linux/WSL2 and Windows 11 (interop) | `commands::execution::tests::across_processes`: a second OS process holds ownership (admission refused, nothing written, reconciliation refused); a supervising process is killed mid-command (restart refuses fresh approvals, nothing replays, blocked until termination is established, then one new approved attempt runs); in-process release only after evidence; unverifiable backend stays blocked | Pass on both. Linux evidence: process group absent after the orphan was ended by the test; Windows evidence: owner-bound job |
| M3 agent connection | 2026-11-10, Linux/WSL2 | `crates/agent/tests/command_connection.rs` (9 tests, 3 new): still blocked after reconnection; released only by explicit reconciliation; unknown agent outcome stays blocked; reconciliation refused while the agent's worker owns the project | Pass. Fake process trees with a real journal and the real RPC path |
| M3 full checks | 2026-11-10 | Linux: format, Clippy, `cargo test --workspace` (536, 0 failed), 15 repeat runs of command and agent-connection tests. Windows: all 128 `commands` tests, 8 repeat runs. Clippy for windows-msvc, windows-gnu and Darwin (the last two with tests); musl agent build | Pass |
| M4 real agent process | 2026-11-10, Linux/WSL2 | `crates/agent/tests/execution_host_path.rs` (7 tests): run and report; Stop and timeout with a descendant; lost connection; killed agent; orderly shutdown during a command; changed executable/directory; no retry on another agent and no local fallback | Pass; 12 consecutive runs without a failure |
| M4 in-process host with native backend | 2026-11-10, Linux/WSL2 and Windows 11 (interop) | `across_processes`: Stop and timeout; changed executable/directory; unsavable intent | Pass on both; all 131 Windows `commands` tests pass |
| M4 Windows-to-WSL, Windows-built client | 2026-11-10, Windows 11 build 26300 → `wsl.exe` → Ubuntu | `crates/core/tests/wsl_execution.rs`, test `an_approved_command_runs_inside_wsl_and_reports_back_to_windows` | Pass once. **Every later start of the binary was refused by Windows application control (Device Guard policy).** The other four tests did not run with this client |
| M4 Windows-to-WSL relay, Linux-built client | 2026-11-10, Linux test process → `wsl.exe` (interop) → Ubuntu | All 5 tests of `wsl_execution.rs` | Pass; 3 consecutive runs. Killing `wsl.exe` ended the agent and its command together |
| M4 full checks | 2026-11-10 | Linux: format, Clippy, `cargo test --workspace` (551, 0 failed). Clippy for core on windows-msvc, core and agent on windows-gnu (all targets) and Darwin; musl agent build | Pass |
| M5 mock provider | 2026-11-10, Linux/WSL2 | `crates/core/tests/llm_tools.rs` (20 tests): split, complete, malformed, incomplete, ambiguous, oversized, too many and multiple calls in the Responses stream; indexed split calls, finish reason and end marker in chat completions; results with original identities; chain reset rebuilding calls and results as history with exactly two HTTP requests and no call events; inconsistent transcripts refused before any request; refused tool declaration announced and not treated as a missing API; budget | Pass; 15 consecutive runs. Payloads written from documented shapes, not recorded |
| M5 unit tests | 2026-11-10, Linux/WSL2 | `llm::tools::tests` (7): schema matches the request shape, assembly, bounds, correlation | Pass |
| M5 regression and full checks | 2026-11-10 | Existing Foundry tests including the recorded request-parity test; format; Clippy; `cargo test --workspace` (578, 0 failed); Clippy for core on windows-msvc and Darwin; Windows app check; musl agent build | Pass |
| M6.1 history and desk | 2026-11-10, Linux/WSL2 and Windows 11 (interop) | `commands::history::tests` (7) and `commands::desk::tests` (7): record life cycle, nothing sensitive saved, share result contents, one command at a time, restore, unusable calls, eviction; review/approve/begin, editor voids approval, no queued approvals, re-run, rejection final, duplicate in one turn, unknown outcome until reconciled, new host instance | Pass on both (145 `commands` tests on Windows) |
| M6.1 chat and Inspect | 2026-11-10, Linux/WSL2 | `chat::tests::commands` (7) and one Inspect test: proposals recorded natively, every call answered truthfully, running blocks continuation, failed round records nothing, no unshared output saved, restore, old chats load unchanged; calls and results per round | Pass |
| M6.1 end to end | 2026-11-10, Linux/WSL2 and Windows 11 (interop) | `crates/core/tests/command_workflow.rs` (3): scripted model → proposal → host preparation → review → approval → real process → observation → shared result received by the model; rejection; a restored chat that can launch nothing without fresh preparation and approval | Pass on both; 12 consecutive runs on Linux, one run on Windows |
| M6.1 full checks | 2026-11-10 | Format; Clippy; `cargo test --workspace` (603, 0 failed); Clippy for core on windows-msvc and Darwin; Windows app check; musl agent build | Pass |
| M6.2 card rules | 2026-11-10, Linux/WSL2 and Windows 11 (interop) | `commands::card::tests` (8): exact argument boundaries and scripts; nothing runnable before a plan is shown; blocked reasons; running offers only Stop; finished and unknown outcomes; re-run; closed after an answer; rejected, unusable and restored cards | Pass on both (153 `commands` tests on Windows) |
| M6.2 in the running app | 2026-11-10, Linux/WSL2 (WSLg), debug build, local mock model, isolated data directories | Debug automation pressing the card's buttons: Run (command ran, exit 0 recorded, journal complete, output not saved); Run then Stop (recorded cancelled, the command's later step never ran, nothing left running); Reject (nothing reached the journal); Edit then Run (original proposal kept, attempt marked edited, only the edited command ran) | All as expected. The window was not looked at: layout and rendering are unverified |
| M6.2 full checks | 2026-11-10 | Format; Clippy for the workspace and for the Windows app; `cargo test --workspace` (611, 0 failed); core Clippy on windows-msvc and Darwin; musl agent build; no literal colours in the new view | Pass |
| M6.3 share rules | 2026-11-10, Linux/WSL2 | Core tests: share choice records omissions and edits and bounds typed text; a result without an outcome carries only identity, status and note; sharing offered only for a finished attempt whose output is held; continuation summary wording | Pass |
| M6.3 in the running app | 2026-11-10, Linux/WSL2 (WSLg), debug build, local mock model, isolated data directories | Debug automation: Run → share output without error output → Continue (the model received exactly that result for that call, with a stored-chain request and nothing else; the saved chat contains the shared output and not the unshared stream); Run → Continue without sharing (model told only "notShared"); Reject → Continue (model told "rejected", command never ran) | All as expected. The window was not looked at |
| M6.3 full checks | 2026-11-10 | Format; Clippy for the workspace and the Windows app; `cargo test --workspace` (613, 0 failed); core Clippy on windows-msvc and Darwin; musl agent build | Pass |
| M6.4 revalidation rule | 2026-11-10, Linux/WSL2 | `chat::tests::a_command_that_changed_files_turns_stale_proposals_into_conflicts`: changed, created, deleted and untouched files; nothing applied or dropped; stale Undo; existing conflicts kept | Pass |
| M6.4 in the running app | 2026-11-10, Linux/WSL2 (WSLg), debug build, local mock model, isolated data directories | Quit while a command runs (app exited within a second, host recorded "cancelled", no process left); close the project while a command runs (host recorded "cancelled", no process left); a command that rewrites a file with an unsaved proposal and creates another (proposal kept and marked as a conflict with a note; the project tree contained the new file afterwards) | All as expected. The window was not looked at |
| M6.4 full checks | 2026-11-10 | Format; Clippy for the workspace and the Windows app; `cargo test --workspace` (614, 0 failed); core Clippy on windows-msvc and Darwin; musl agent build; Run and Share re-checked in the app | Pass |
| M6.5 wording | 2026-11-10, Linux/WSL2 | `commands::wording` test (exact sentence, no blanket promise); `logic::prompt` test (command section added only on request, states that proposing runs nothing); recorded prompt fixtures unchanged | Pass |
| M6.5 in the running app | 2026-11-10, Linux/WSL2 (WSLg), debug build, local mock model | With the switch on, both requests carried the tool and the command section of the system prompt; with it off, neither, and a tool call the mock sent anyway ran nothing | As expected. Status bar and welcome text were compiled, not seen |
| M6.5 full checks | 2026-11-10 | Format; Clippy for the workspace and the Windows app; `cargo test --workspace` (616, 0 failed); core Clippy on windows-msvc and Darwin; musl agent build | Pass |
| Agent execution mode | 2026-11-10, Linux/WSL2 | Agent argument parsing; `crates/agent/tests/execution_host_path.rs` against the real agent binary (8 tests, one new: an agent started the default way cannot run or reconcile anything); launch-script quoting | Pass; 10 consecutive runs |
| Agent path in the running app | 2026-11-10, Linux/WSL2 (WSLg), debug build, `CONTEXTUAL_DEV_REMOTE=1` with the switch, local mock model | Run and share; Run then Stop | Command ran through the agent (records in the agent's journal, none in the app's); record location is the distro; shared result reached the model; Stop recorded cancelled; no process or agent left |
| Real `wsl.exe` relay with the real agent | 2026-11-10, Linux test process → `wsl.exe` → Ubuntu | All 5 tests of `wsl_execution.rs` | Pass; 6 consecutive runs after the zombie fix (before it, the killed-agent test failed about half the time) |
| Zombie-only groups | 2026-11-10, Linux/WSL2, glibc and musl | `linux::tests`: a group of unreaped zombies counts as gone, a live member does not; early root exit now always confirmed | Pass; 15 consecutive runs of `commands::execution` |
| Full checks after the agent work | 2026-11-10 | Linux: format, Clippy, `cargo test --workspace` (619, 0 failed). Clippy for the Windows app and agent, core on windows-msvc, core and agent on Darwin; musl agent build | Pass. **Windows tests not re-run: application control refused the rebuilt core test binary** |
| Opt-in setting | 2026-11-10, Linux/WSL2 (WSLg), debug build, local mock model | Settings test (off by default, old settings stay off, round trip); running app with `chat.proposeCommands` true, false and absent, without the environment switch | On: tool offered, approved command ran, journal complete. Off or absent: tool not offered, nothing ran. Workspace 620 tests, 0 failed; release build of the app compiles |
| Windows core suite outside this feature | 2026-11-10, same Windows host | Whole `contextual-core` unit-test binary (at M2.3) | 263 passed, 5 failed, all in `process::tests` (the Git runner's tests). Not part of this delivery and not investigated |
| M4 Windows-to-WSL | — | Agent execution over the real connection | Not started; the Linux rows above ran natively inside WSL, not through the Windows app |

Replace pending check entries or append results only after running the checks. Include the saved
revision and any local modifications; an unsaved proposal is not the revision tested by Cargo.

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
