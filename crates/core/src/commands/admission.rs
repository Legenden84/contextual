//! Process-wide admission for commands on the native local host.
//!
//! Reservation is synchronous and never waits for another command to finish. The canonical
//! project root is the key, not a chat ID, project-open epoch, or LocalHost instance.
//! Unknown outcomes and interrupted workers retain their slots.
//!
//! This is not authorization and does not provide cross-process ownership. Native execution
//! must remain disabled until separate app/agent processes also have an ownership mechanism.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use super::InvocationId;
use super::execution::{ExecutionResult, ExecutionStatus};
use crate::{Error, Result};

/// Bound retained unknown slots without evicting them to make room for more executions.
const MAX_SLOTS: usize = 256;

static LOCAL: LazyLock<Arc<ExecutionAdmission>> =
    LazyLock::new(|| Arc::new(ExecutionAdmission::default()));

pub(crate) fn local_registry() -> Arc<ExecutionAdmission> {
    Arc::clone(&LOCAL)
}

#[derive(Clone, Copy)]
enum State {
    Reserved,
    Blocked,
}

struct Slot {
    invocation: InvocationId,
    state: State,
}

/// Admission state for an execution host, separate from user approval.
///
/// Production LocalHosts all use the process-wide native registry. There is deliberately no
/// public reset or "assume stopped" operation. Reconciliation must establish termination and
/// history integrity before a future implementation may release a blocked slot.
#[derive(Default)]
pub struct ExecutionAdmission {
    slots: Mutex<HashMap<PathBuf, Slot>>,
}

impl ExecutionAdmission {
    pub(crate) fn reserve(
        self: &Arc<Self>,
        canonical_root: &Path,
        invocation: &InvocationId,
    ) -> Result<AdmissionLease> {
        if !canonical_root.is_absolute() {
            return Err(Error::invalid(
                "Execution admission requires a canonical absolute project root.",
            ));
        }
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| Error::internal("Execution admission state is unavailable."))?;
        if let Some(slot) = slots.get(canonical_root) {
            return Err(Error::conflict(match slot.state {
                State::Reserved => "A command is already preparing or running for this project.",
                State::Blocked => {
                    "A previous command's outcome or execution history is unresolved. \
                     Another command cannot start."
                }
            }));
        }
        if slots.len() >= MAX_SLOTS {
            return Err(Error::conflict(
                "Execution admission limit reached. Unresolved commands were not discarded.",
            ));
        }
        let root = canonical_root.to_path_buf();
        slots.insert(
            root.clone(),
            Slot {
                invocation: invocation.clone(),
                state: State::Reserved,
            },
        );
        Ok(AdmissionLease {
            admission: Arc::clone(self),
            root,
            invocation: invocation.clone(),
            finished: false,
        })
    }
}

/// Worker-owned reservation. Dropping it without an observation blocks instead of guessing
/// whether the worker reached spawn or completed cleanup.
pub(crate) struct AdmissionLease {
    admission: Arc<ExecutionAdmission>,
    root: PathBuf,
    invocation: InvocationId,
    finished: bool,
}

impl AdmissionLease {
    pub(crate) fn finish(mut self, result: &ExecutionResult) {
        if let Ok(mut slots) = self.admission.slots.lock()
            && let Some(slot) = slots.get_mut(&self.root)
            && slot.invocation == self.invocation
        {
            if result.status == ExecutionStatus::Unknown || result.history_error.is_some() {
                slot.state = State::Blocked;
            } else {
                slots.remove(&self.root);
            }
        }
        // A poisoned registry remains unavailable to all future reservations.
        self.finished = true;
    }
}

impl Drop for AdmissionLease {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Ok(mut slots) = self.admission.slots.lock()
            && let Some(slot) = slots.get_mut(&self.root)
            && slot.invocation == self.invocation
        {
            slot.state = State::Blocked;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::ExecutionProgress;

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

    #[test]
    fn reservations_are_immediate_and_do_not_queue() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ExecutionAdmission::default());
        let lease = registry.reserve(root.path(), &InvocationId::new()).unwrap();
        assert!(registry.reserve(root.path(), &InvocationId::new()).is_err());

        lease.finish(&result(ExecutionStatus::Success));
        let next = registry.reserve(root.path(), &InvocationId::new()).unwrap();
        next.finish(&result(ExecutionStatus::Success));
    }

    #[test]
    fn unknown_outcome_retains_the_slot() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ExecutionAdmission::default());
        let lease = registry.reserve(root.path(), &InvocationId::new()).unwrap();
        lease.finish(&result(ExecutionStatus::Unknown));
        assert!(registry.reserve(root.path(), &InvocationId::new()).is_err());
    }

    #[test]
    fn dropping_an_unfinished_worker_retains_the_slot() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ExecutionAdmission::default());
        let lease = registry.reserve(root.path(), &InvocationId::new()).unwrap();
        drop(lease);
        assert!(registry.reserve(root.path(), &InvocationId::new()).is_err());
    }

    #[test]
    fn known_process_completion_with_unsaved_history_retains_the_slot() {
        let root = tempfile::tempdir().unwrap();
        let registry = Arc::new(ExecutionAdmission::default());
        let lease = registry.reserve(root.path(), &InvocationId::new()).unwrap();
        let mut completed = result(ExecutionStatus::Success);
        completed.history_error = Some("Completion was not confirmed saved.".into());
        lease.finish(&completed);
        assert!(registry.reserve(root.path(), &InvocationId::new()).is_err());
    }

    #[test]
    fn an_unresolved_project_does_not_block_another_project() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let registry = Arc::new(ExecutionAdmission::default());
        let lease = registry
            .reserve(first.path(), &InvocationId::new())
            .unwrap();
        lease.finish(&result(ExecutionStatus::Unknown));

        let other = registry
            .reserve(second.path(), &InvocationId::new())
            .unwrap();
        other.finish(&result(ExecutionStatus::Success));
    }
}
