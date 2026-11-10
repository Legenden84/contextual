//! The in-memory side of command review for one open project: prepared plans, approvals and
//! output that has not been shared.
//!
//! Everything here is runtime policy and is deliberately not serializable. It is owned by the
//! project, not by a chat view, so that switching chats cannot lose track of a running
//! command or create a second way to start one. After a restart the desk is empty: every
//! proposal that still awaits review must be prepared and reviewed again, and nothing that
//! was approved before can be started.
//!
//! The desk never starts a process. It hands out a [`LaunchPermit`] for exactly one reviewed
//! plan, which the host's execution service consumes.

use std::collections::{HashMap, VecDeque};

use super::execution::ExecutionResult;
use super::{
    Approval, ApprovalGate, CallId, CommandState, ExecutionBinding, InvocationId, LaunchPermit,
    ReviewPlan,
};
use crate::{Error, Result};

/// Unshared results kept in memory. Older ones are dropped, never written anywhere.
const MAX_HELD_OUTPUTS: usize = 8;

struct Session {
    binding: ExecutionBinding,
    gate: ApprovalGate,
}

#[derive(Default)]
pub struct CommandDesk {
    session: Option<Session>,
    /// A proposal's current identity in the gate. A re-run gets a new one, because the gate
    /// treats a finished call as final.
    keys: HashMap<CallId, CallId>,
    plans: HashMap<CallId, ReviewPlan>,
    reruns: u64,
    outputs: VecDeque<(CallId, InvocationId, ExecutionResult)>,
}

impl CommandDesk {
    pub fn new() -> Self {
        Self::default()
    }

    fn gate(&mut self) -> Result<&mut ApprovalGate> {
        self.session
            .as_mut()
            .map(|session| &mut session.gate)
            .ok_or_else(|| Error::conflict("Prepare this command again before reviewing it."))
    }

    fn key(&self, call: &CallId) -> Result<CallId> {
        self.keys
            .get(call)
            .cloned()
            .ok_or_else(|| Error::conflict("Prepare this command again before reviewing it."))
    }

    /// A command is running, or one ended with an unknown outcome that was not reconciled.
    pub fn busy(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.gate.busy())
    }

    /// The plan currently shown for this proposal, if it has been prepared in this session.
    pub fn plan(&self, call: &CallId) -> Option<&ReviewPlan> {
        self.plans.get(call)
    }

    pub fn state(&self, call: &CallId) -> Option<CommandState> {
        let key = self.keys.get(call)?;
        self.session.as_ref()?.gate.state(key)
    }

    /// Puts a freshly prepared plan up for review. This approves nothing.
    ///
    /// A plan from a different project or host instance starts a new session: everything
    /// prepared before it must be prepared again. That is refused while a command runs.
    pub fn review(&mut self, call: &CallId, plan: ReviewPlan) -> Result<()> {
        let same_session = self
            .session
            .as_ref()
            .is_some_and(|session| session.binding == plan.binding);
        if !same_session {
            if self.busy() {
                return Err(Error::conflict(
                    "A command is running or its outcome is unknown.",
                ));
            }
            self.session = Some(Session {
                binding: plan.binding.clone(),
                gate: ApprovalGate::new(plan.binding.clone()),
            });
            self.keys.clear();
            self.plans.clear();
        }
        match self.state(call) {
            Some(CommandState::NeedsReview | CommandState::Approved) => {
                // A changed or refreshed plan replaces the old one and voids its approval.
                let key = self.key(call)?;
                self.gate()?.edit(&key, plan.clone())?;
            }
            Some(CommandState::Rejected) => {
                return Err(Error::conflict(
                    "This proposal was rejected. It cannot be put up for review again.",
                ));
            }
            Some(CommandState::Running) => {
                return Err(Error::conflict("This command is running."));
            }
            Some(CommandState::Finished(_)) => {
                // A re-run. The gate treats a finished call as final, so the new attempt
                // gets its own identity; the user must review and approve it afresh.
                self.reruns += 1;
                let key = CallId {
                    turn: format!("{}/rerun-{}", call.turn, self.reruns),
                    call: call.call.clone(),
                };
                self.gate()?.propose(key.clone(), plan.clone())?;
                self.keys.insert(call.clone(), key);
            }
            None => {
                // First review in this session, under the model turn's own identity, so
                // that the same invocation proposed twice in one turn is refused.
                self.gate()?.propose(call.clone(), plan.clone())?;
                self.keys.insert(call.clone(), call.clone());
            }
        }
        self.plans.insert(call.clone(), plan);
        Ok(())
    }

    /// The user opened the editor. Approval is void from this moment, whatever follows.
    pub fn editing(&mut self, call: &CallId) -> Result<()> {
        let key = self.key(call)?;
        self.gate()?.invalidate(&key)
    }

    pub fn reject(&mut self, call: &CallId) -> Result<()> {
        let key = self.key(call)?;
        self.gate()?.reject(&key)?;
        self.plans.remove(call);
        Ok(())
    }

    /// The user pressed Run on exactly `displayed`. Synchronous, so a second click cannot
    /// produce a second approval. The approval must then be consumed with [`Self::begin`].
    pub fn approve(&mut self, call: &CallId, displayed: &ReviewPlan) -> Result<Approval> {
        let key = self.key(call)?;
        self.gate()?.approve(&key, displayed)
    }

    /// Consumes the approval against a plan prepared again just now. Any difference, or a
    /// command that started meanwhile, refuses and sends the proposal back to review.
    pub fn begin(&mut self, approval: Approval, current: &ReviewPlan) -> Result<LaunchPermit> {
        self.gate()?.begin(approval, current)
    }

    /// Records the observation for the running attempt and keeps its output in memory.
    pub fn finished(
        &mut self,
        call: &CallId,
        invocation: &InvocationId,
        result: ExecutionResult,
    ) -> Result<()> {
        let key = self.key(call)?;
        self.gate()?.finish(&key, result.outcome())?;
        self.outputs.retain(|(held, _, _)| held != call);
        if self.outputs.len() >= MAX_HELD_OUTPUTS {
            self.outputs.pop_front();
        }
        self.outputs
            .push_back((call.clone(), invocation.clone(), result));
        Ok(())
    }

    /// The launch was refused after approval was consumed: nothing started.
    pub fn failed_to_start(&mut self, call: &CallId) -> Result<()> {
        let key = self.key(call)?;
        self.gate()?.finish(&key, super::Outcome::FailedToStart)
    }

    /// The latest unshared result for a proposal, while it is still held.
    pub fn output(&self, call: &CallId) -> Option<(&InvocationId, &ExecutionResult)> {
        self.outputs
            .iter()
            .find(|(held, _, _)| held == call)
            .map(|(_, invocation, result)| (invocation, result))
    }

    /// Drops a held result, after it was shared or the user chose not to share it.
    pub fn forget_output(&mut self, call: &CallId) {
        self.outputs.retain(|(held, _, _)| held != call);
    }

    /// The host established that nothing from this project still runs. The session ends:
    /// every proposal needs fresh preparation and review, and no earlier approval survives.
    /// Refused while a command is actually running.
    pub fn reconciled(&mut self) -> Result<()> {
        let running = self.keys.values().any(|key| {
            self.session
                .as_ref()
                .and_then(|session| session.gate.state(key))
                == Some(CommandState::Running)
        });
        if running {
            return Err(Error::conflict("A command is still running."));
        }
        self.session = None;
        self.keys.clear();
        self.plans.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use contextual_protocol::commands::PrepareCommand;

    use super::*;
    use crate::commands::Outcome;
    use crate::commands::execution::{ExecutionProgress, ExecutionStatus};
    use crate::host::{LocalHost, WorkspaceHost};
    use crate::location::{Location, ProjectRoot};

    struct Project {
        _root: tempfile::TempDir,
        path: String,
        host: LocalHost,
    }

    impl Project {
        async fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().to_str().unwrap().to_owned();
            let host = LocalHost::new();
            host.allow_root(&path).await.unwrap();
            Self {
                _root: root,
                path,
                host,
            }
        }

        /// Host preparation only: nothing is executed by it.
        async fn plan(&self, args: &[&str]) -> ReviewPlan {
            let prepared = self
                .host
                .prepare_command(PrepareCommand {
                    root: self.path.clone(),
                    project_epoch: "project-1".into(),
                    command_json: serde_json::json!({
                        "version": 1,
                        "purpose": "Exercise the desk.",
                        "command": {
                            "kind": "exec",
                            "program": std::env::current_exe().unwrap().to_str().unwrap(),
                            "args": args
                        },
                        "cwd": ".",
                        "timeoutSeconds": 30,
                        "environment": []
                    })
                    .to_string(),
                })
                .await
                .unwrap();
            ReviewPlan::from_prepared(
                ProjectRoot {
                    location: Location::Local,
                    path: self.path.clone(),
                },
                prepared,
            )
            .unwrap()
        }
    }

    fn call(id: &str) -> CallId {
        CallId {
            turn: "turn-1".into(),
            call: id.into(),
        }
    }

    fn other_call() -> CallId {
        call("call_2")
    }

    fn result(status: ExecutionStatus) -> ExecutionResult {
        ExecutionResult {
            status,
            exit_code: (status == ExecutionStatus::Success).then_some(0),
            spawned: true,
            cancelled: false,
            timed_out: false,
            output: ExecutionProgress::default(),
            detail: None,
            history_error: None,
        }
    }

    #[tokio::test]
    async fn run_needs_review_then_approval_of_exactly_what_was_shown() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let call = call("call_1");
        // Nothing can be approved before it was prepared and shown.
        let plan = project.plan(&["a"]).await;
        assert!(desk.approve(&call, &plan).is_err());

        desk.review(&call, plan.clone()).unwrap();
        assert_eq!(desk.state(&call), Some(CommandState::NeedsReview));
        // Plans have no debug output, so that an environment cannot end up in a log.
        assert!(desk.plan(&call) == Some(&plan));

        // Approving something other than what is shown sends it back to review.
        let other = project.plan(&["b"]).await;
        assert!(desk.approve(&call, &other).is_err());
        assert_eq!(desk.state(&call), Some(CommandState::NeedsReview));

        let approval = desk.approve(&call, &plan).unwrap();
        let permit = desk.begin(approval, &plan).unwrap();
        assert!(permit.plan() == &plan);
        assert!(desk.busy());
        assert_eq!(desk.state(&call), Some(CommandState::Running));

        let invocation = permit.invocation().clone();
        desk.finished(&call, &invocation, result(ExecutionStatus::Success))
            .unwrap();
        assert!(!desk.busy());
        assert_eq!(
            desk.state(&call),
            Some(CommandState::Finished(Outcome::Success))
        );
        // The output is held in memory until it is shared or dismissed.
        assert_eq!(desk.output(&call).unwrap().0, &invocation);
        desk.forget_output(&call);
        assert!(desk.output(&call).is_none());
    }

    #[tokio::test]
    async fn opening_the_editor_or_a_changed_plan_voids_approval() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let call = call("call_1");
        let plan = project.plan(&["a"]).await;
        desk.review(&call, plan.clone()).unwrap();

        // Editing starts after Run was pressed: the approval is void even if the editor is
        // closed without a change.
        let approval = desk.approve(&call, &plan).unwrap();
        desk.editing(&call).unwrap();
        assert!(desk.begin(approval, &plan).is_err());
        assert_eq!(desk.state(&call), Some(CommandState::NeedsReview));

        // A revised plan needs its own review; the old plan can no longer be approved.
        let revised = project.plan(&["b"]).await;
        let approval = desk.approve(&call, &plan).unwrap();
        desk.review(&call, revised.clone()).unwrap();
        assert!(desk.begin(approval, &revised).is_err());
        assert!(desk.approve(&call, &plan).is_err());
        let approval = desk.approve(&call, &revised).unwrap();
        // The preparation changed between approval and launch.
        assert!(desk.begin(approval, &plan).is_err());
        assert!(!desk.busy());
    }

    #[tokio::test]
    async fn one_command_at_a_time_and_approvals_never_queue() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let (first, second) = (call("call_1"), call("call_2"));
        let (plan_a, plan_b) = (project.plan(&["a"]).await, project.plan(&["b"]).await);
        desk.review(&first, plan_a.clone()).unwrap();
        desk.review(&second, plan_b.clone()).unwrap();

        let approval_a = desk.approve(&first, &plan_a).unwrap();
        let approval_b = desk.approve(&second, &plan_b).unwrap();
        let permit = desk.begin(approval_a, &plan_a).unwrap();
        // The sibling's approval did not survive the first launch.
        assert!(desk.begin(approval_b, &plan_b).is_err());
        assert!(desk.approve(&second, &plan_b).is_err());

        desk.finished(
            &first,
            permit.invocation(),
            result(ExecutionStatus::Success),
        )
        .unwrap();
        // Finishing starts nothing: the sibling still needs its own approval.
        assert_eq!(desk.state(&second), Some(CommandState::NeedsReview));
        let approval = desk.approve(&second, &plan_b).unwrap();
        desk.begin(approval, &plan_b).unwrap();
    }

    #[tokio::test]
    async fn a_rerun_is_a_new_review_and_a_rejection_is_final() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let call = call("call_1");
        let plan = project.plan(&["a"]).await;
        desk.review(&call, plan.clone()).unwrap();
        let approval = desk.approve(&call, &plan).unwrap();
        let permit = desk.begin(approval, &plan).unwrap();
        desk.finished(&call, permit.invocation(), result(ExecutionStatus::Success))
            .unwrap();

        // The finished attempt cannot be approved again as it stands.
        assert!(desk.approve(&call, &plan).is_err());
        // Preparing it again puts it back under review, with a fresh approval needed.
        desk.review(&call, plan.clone()).unwrap();
        assert_eq!(desk.state(&call), Some(CommandState::NeedsReview));
        let approval = desk.approve(&call, &plan).unwrap();
        let again = desk.begin(approval, &plan).unwrap();
        assert_ne!(again.invocation(), permit.invocation());
        desk.finished(&call, again.invocation(), result(ExecutionStatus::Success))
            .unwrap();

        let other = other_call();
        let plan_b = project.plan(&["b"]).await;
        desk.review(&other, plan_b.clone()).unwrap();
        desk.reject(&other).unwrap();
        assert_eq!(desk.state(&other), Some(CommandState::Rejected));
        assert!(desk.approve(&other, &plan_b).is_err());
        assert!(desk.plan(&other).is_none());
        // Preparing it again does not bring it back.
        assert!(desk.review(&other, plan_b.clone()).is_err());
        assert!(desk.approve(&other, &plan_b).is_err());
    }

    #[tokio::test]
    async fn the_same_invocation_twice_in_one_model_turn_is_reviewed_once() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let plan = project.plan(&["a"]).await;
        desk.review(&call("call_1"), plan.clone()).unwrap();
        assert!(desk.review(&other_call(), plan.clone()).is_err());
        assert!(desk.state(&other_call()).is_none());
        assert!(desk.approve(&other_call(), &plan).is_err());

        // The same invocation in a later model turn is a separate proposal.
        let later = CallId {
            turn: "turn-2".into(),
            call: "call_1".into(),
        };
        desk.review(&later, plan.clone()).unwrap();
        // A sibling running in between does not let the later one start unreviewed.
        let approval = desk.approve(&later, &plan).unwrap();
        desk.begin(approval, &plan).unwrap();
        assert!(desk.approve(&call("call_1"), &plan).is_err());
    }

    #[tokio::test]
    async fn an_unknown_outcome_blocks_until_reconciled_and_reconciling_approves_nothing() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let (first, second) = (call("call_1"), call("call_2"));
        let (plan_a, plan_b) = (project.plan(&["a"]).await, project.plan(&["b"]).await);
        desk.review(&first, plan_a.clone()).unwrap();
        desk.review(&second, plan_b.clone()).unwrap();
        let approval = desk.approve(&first, &plan_a).unwrap();
        let permit = desk.begin(approval, &plan_a).unwrap();

        // Reconciling is refused while the command is still running.
        assert!(desk.reconciled().is_err());
        desk.finished(
            &first,
            permit.invocation(),
            result(ExecutionStatus::Unknown),
        )
        .unwrap();
        assert!(desk.busy());
        assert!(desk.approve(&second, &plan_b).is_err());

        desk.reconciled().unwrap();
        assert!(!desk.busy());
        // Nothing prepared earlier can be approved: it has to be prepared and shown again.
        assert!(desk.approve(&second, &plan_b).is_err());
        assert!(desk.plan(&second).is_none());
        desk.review(&second, plan_b.clone()).unwrap();
        let approval = desk.approve(&second, &plan_b).unwrap();
        desk.begin(approval, &plan_b).unwrap();
    }

    #[tokio::test]
    async fn a_new_host_instance_starts_a_new_session() {
        let project = Project::new().await;
        let mut desk = CommandDesk::new();
        let call = call("call_1");
        let plan = project.plan(&["a"]).await;
        desk.review(&call, plan.clone()).unwrap();
        let approval = desk.approve(&call, &plan).unwrap();

        // The same project served by a new host instance: a different preparation epoch.
        let replacement = LocalHost::new();
        replacement.allow_root(&project.path).await.unwrap();
        let moved = Project {
            _root: tempfile::tempdir().unwrap(),
            path: project.path.clone(),
            host: replacement,
        };
        let fresh = moved.plan(&["a"]).await;
        assert!(fresh.binding != plan.binding);
        let other = other_call();
        desk.review(&other, fresh.clone()).unwrap();

        // The approval from the earlier session is worthless, and its plan is gone.
        assert!(desk.begin(approval, &plan).is_err());
        assert!(desk.plan(&call).is_none());
        let approval = desk.approve(&other, &fresh).unwrap();
        desk.begin(approval, &fresh).unwrap();
    }
}
