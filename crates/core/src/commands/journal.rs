//! Append-only execution observations, outside the project and separate from chat pruning.
//!
//! Intent and completion are separate immutable files, correlated by an app invocation ID.
//! Publication uses a synced temporary file, a no-clobber hard link, and directory sync.
//! Errors or interrupted intent writes never permit launch; a late blocking write can leave
//! an unknown observation, but cannot launch anything.
//!
//! Both implementations rely on filesystem support for hard links and for flushing a
//! directory; a filesystem lacking either fails closed. On Unix the directory is opened
//! read-only and fsynced. On Windows a directory can only be flushed through a handle with
//! write access, the new record is flushed again under its published name, and ancestors
//! the user cannot write to are not flushed. Windows documents no directory-flush
//! durability contract, so power-loss durability there rests on NTFS behaviour, not on a
//! guarantee. Other platforms fail closed at initialization.
//!
//! Privacy on Unix is enforced through directory and file modes. On Windows it is
//! inherited from the directory's ACL, which this code cannot inspect: the default location
//! is inside the user's profile. Either way this is app-owned storage, not a tamper-proof
//! log against processes with the same credentials.
//!
//! Recovery returns observations, never authorization. The execution engine checks these
//! observations under its process-wide host reservation before publishing a new intent.
//! Unknown records block their canonical project root. Older unknown records lacking that root
//! conservatively block their execution location.
//!
//! Cross-process ownership is an operating-system lock on a per-project file in this
//! directory. The supervising process holds it from before the recovery check until after
//! the completion record, and the system releases it if that process dies. It excludes every
//! process that shares this directory; it does not exclude another user's journal.
//!
//! Two more immutable files accompany an invocation that reaches launch: a marker written
//! before the start is attempted, and the started tree's identity. Reconciliation takes the
//! project lock and then unblocks an unknown invocation only on evidence: the marker was never
//! written, the tree was bound to an owner that no longer holds the lock, the recorded process
//! group no longer exists, or the host restarted. It records that evidence in a fourth file.
//! It never launches, retries, signals or approves anything, and the outcome stays unknown.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::InvocationId;
use super::execution::{
    Containment, ExecutionCompletion, ExecutionIntent, ExecutionJournal, ExecutionStatus,
    OwnershipGuard, ReconciledInvocation, ReconciliationReport, TerminationEvidence, TreeIdentity,
    current_boot_id, recorded_group_is_absent,
};
use crate::error::ErrorCode;
use crate::location::Location;
use crate::{Error, Result};

/// Version 2 intents are followed by a launch marker before any start is attempted.
/// Version 1 intents predate the marker, so its absence proves nothing about them.
const VERSION: u32 = 2;
const FIRST_VERSION_WITH_LAUNCH_MARKER: u32 = 2;
/// Each record excludes output and the inherited environment.
const MAX_RECORD_BYTES: usize = 256 * 1024;
/// Fail rather than return a silently incomplete recovery listing. No replay records are evicted.
const MAX_RECOVERY_RECORDS: usize = 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredIntent {
    version: u32,
    recorded_at: String,
    intent: ExecutionIntent,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCompletion {
    version: u32,
    recorded_at: String,
    completion: ExecutionCompletion,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredLaunching {
    version: u32,
    recorded_at: String,
    invocation: InvocationId,
    containment: Containment,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredLaunched {
    version: u32,
    recorded_at: String,
    invocation: InvocationId,
    identity: TreeIdentity,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredReconciliation {
    version: u32,
    recorded_at: String,
    invocation: InvocationId,
    evidence: TerminationEvidence,
}

/// Historical observations only. No approval, prepared environment, or unshared output.
#[derive(Debug, Clone)]
pub struct ExecutionRecord {
    pub intent: ExecutionIntent,
    pub intended_at: String,
    pub completion: Option<ExecutionCompletion>,
    pub completed_at: Option<String>,
    /// Missing, malformed, mismatched, or unreadable completion records remain unknown.
    pub recovery_warning: Option<String>,
    /// Evidence that an unknown invocation no longer runs. It does not change the outcome.
    pub reconciliation: Option<TerminationEvidence>,
    pub reconciled_at: Option<String>,
}

impl ExecutionRecord {
    pub fn status(&self) -> ExecutionStatus {
        self.completion
            .as_ref()
            .map_or(ExecutionStatus::Unknown, |completion| completion.status)
    }

    /// Termination must be established before more work is admitted for this project.
    pub fn blocks_execution(&self) -> bool {
        self.status() == ExecutionStatus::Unknown && self.reconciliation.is_none()
    }
}

/// Immutable-file implementation of the write-ahead journal.
///
/// The directory must be chosen by trusted application code, outside the project and outside
/// the automatically pruned thread store. Clones and separately opened instances share the
/// filesystem's no-clobber protection, not an in-memory duplicate cache.
#[derive(Clone)]
pub struct FileExecutionJournal {
    dir: PathBuf,
}

impl FileExecutionJournal {
    /// Creates or opens a private journal directory on a supported filesystem.
    /// It does not launch commands, restore approval, or reconcile unknown process outcomes.
    pub async fn open(dir: PathBuf) -> Result<Self> {
        blocking(move || {
            ensure_supported()?;
            if !dir.is_absolute() {
                return Err(Error::invalid(
                    "Execution journal directory must be absolute.",
                ));
            }
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt as _;
                builder.mode(0o700);
            }
            builder.create(&dir)?;
            check_directory(&dir)?;
            let dir = dunce::canonicalize(dir)?;

            sync_created_directories(&dir)?;
            Ok(Self { dir })
        })
        .await
    }

    /// `<application data directory>/executions`, independent of saved-chat retention.
    pub async fn open_default() -> Result<Self> {
        let dir = directories::ProjectDirs::from("", "", "contextual")
            .ok_or_else(|| Error::internal("No application data directory."))?
            .data_dir()
            .join("executions");
        Self::open(dir).await
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Loads one observation. A missing or corrupt intent is an error, not an empty history.
    /// A missing or corrupt completion leaves the intact intent's outcome unknown.
    pub async fn load(&self, invocation: &InvocationId) -> Result<ExecutionRecord> {
        let dir = self.dir.clone();
        let invocation = invocation.clone();
        blocking(move || {
            check_directory(&dir)?;
            load_record(&dir, &invocation)
        })
        .await
    }

    /// Loads bounded recovery history, oldest first, retaining exact local/WSL project identity.
    ///
    /// Any corrupt intent, orphan completion, unexpected file, or oversized listing fails the
    /// recovery operation. Callers must not turn that error into an empty successful recovery.
    /// Temporary files left by a crash are ignored: they were never published as intent.
    pub async fn records(&self) -> Result<Vec<ExecutionRecord>> {
        let dir = self.dir.clone();
        blocking(move || {
            check_directory(&dir)?;
            let mut ids = Vec::new();
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| Error::invalid("Unrecognized execution journal entry."))?;
                if let Some(id) = name.strip_suffix(".intent.json") {
                    if ids.len() >= MAX_RECOVERY_RECORDS {
                        return Err(Error::conflict(
                            "Execution recovery limit reached. History was not truncated.",
                        ));
                    }
                    ids.push(InvocationId::parse(id)?);
                } else if let Some(id) = [
                    ".completion.json",
                    ".launching.json",
                    ".launched.json",
                    ".reconciled.json",
                ]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                {
                    let id = InvocationId::parse(id)?;
                    if !fs::symlink_metadata(intent_path(&dir, &id))?.is_file() {
                        return Err(Error::conflict(
                            "Execution completion has no regular intent record.",
                        ));
                    }
                } else if is_owner_lock(name) {
                    // Ownership locks carry no history.
                } else if !(name.starts_with('.') && name.ends_with(".tmp")) {
                    return Err(Error::invalid("Unrecognized execution journal entry."));
                }
            }
            let mut records = ids
                .iter()
                .map(|id| load_record(&dir, id))
                .collect::<Result<Vec<_>>>()?;
            records.sort_by(|left, right| {
                left.intended_at.cmp(&right.intended_at).then_with(|| {
                    left.intent
                        .invocation
                        .as_str()
                        .cmp(right.intent.invocation.as_str())
                })
            });
            Ok(records)
        })
        .await
    }
}

#[async_trait]
impl ExecutionJournal for FileExecutionJournal {
    async fn check_recovery(
        &self,
        project: &crate::location::ProjectRoot,
        canonical_root: &Path,
    ) -> Result<()> {
        let records = self.records().await?;
        let unresolved = records
            .iter()
            .any(|record| record.blocks_execution() && concerns(record, project, canonical_root));
        if unresolved {
            return Err(Error::conflict(
                "An earlier command has an unknown outcome. \
                 Termination must be established before another command can run.",
            ));
        }
        Ok(())
    }

    async fn acquire_ownership(
        &self,
        project: &crate::location::ProjectRoot,
        canonical_root: &Path,
    ) -> Result<OwnershipGuard> {
        let dir = self.dir.clone();
        let path = owner_lock_path(&dir, &project.location, canonical_root)?;
        blocking(move || {
            check_directory(&dir)?;
            lock_project(&path)
        })
        .await
    }

    async fn record_launching(
        &self,
        invocation: &InvocationId,
        containment: Containment,
    ) -> Result<()> {
        let stored = StoredLaunching {
            version: VERSION,
            recorded_at: crate::threads::now_iso(),
            invocation: invocation.clone(),
            containment,
        };
        self.publish_for_intent(
            stored.invocation.clone(),
            ".launching.json",
            encode(&stored)?,
        )
        .await
    }

    async fn record_launched(
        &self,
        invocation: &InvocationId,
        identity: &TreeIdentity,
    ) -> Result<()> {
        validate_identity(identity)?;
        let stored = StoredLaunched {
            version: VERSION,
            recorded_at: crate::threads::now_iso(),
            invocation: invocation.clone(),
            identity: identity.clone(),
        };
        self.publish_for_intent(
            stored.invocation.clone(),
            ".launched.json",
            encode(&stored)?,
        )
        .await
    }

    async fn reconcile(
        &self,
        project: &crate::location::ProjectRoot,
        canonical_root: &Path,
    ) -> Result<ReconciliationReport> {
        // Held for the whole operation: no process can be supervising, or starting, a
        // command for this project while its history is judged.
        let _ownership = self
            .acquire_ownership(project, canonical_root)
            .await
            .map_err(|error| {
                if error.code == ErrorCode::Conflict {
                    Error::conflict(
                        "A process still owns command execution for this project. \
                         Nothing was reconciled.",
                    )
                } else {
                    error
                }
            })?;
        let records = self.records().await?;
        let dir = self.dir.clone();
        let project = project.clone();
        let canonical_root = canonical_root.to_path_buf();
        blocking(move || {
            let mut report = ReconciliationReport::default();
            for record in records {
                if !record.blocks_execution() || !concerns(&record, &project, &canonical_root) {
                    continue;
                }
                let invocation = record.intent.invocation.clone();
                let (evidence, detail) = match termination_evidence(&dir, &record) {
                    Ok(evidence) => match publish_reconciliation(&dir, &invocation, evidence) {
                        Ok(()) => (Some(evidence), None),
                        Err(_) => (
                            None,
                            Some("The reconciliation could not be recorded.".to_owned()),
                        ),
                    },
                    Err(reason) => (None, Some(reason.to_owned())),
                };
                report.invocations.push(ReconciledInvocation {
                    invocation,
                    evidence,
                    detail,
                });
            }
            Ok(report)
        })
        .await
    }

    async fn record_intent(&self, intent: &ExecutionIntent) -> Result<()> {
        validate_intent(intent)?;
        if intent.canonical_root.is_none() {
            return Err(Error::invalid(
                "New execution intent requires the canonical project root.",
            ));
        }
        let stored = StoredIntent {
            version: VERSION,
            recorded_at: crate::threads::now_iso(),
            intent: intent.clone(),
        };
        validate_header(stored.version, &stored.recorded_at)?;
        let bytes = encode(&stored)?;
        let dir = self.dir.clone();
        blocking(move || {
            check_directory(&dir)?;
            // Do not repair an orphan completion by inserting a new intent under it.
            match fs::symlink_metadata(completion_path(&dir, &stored.intent.invocation)) {
                Ok(_) => {
                    return Err(Error::conflict(
                        "Invocation already has a completion record.",
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            publish_new(&dir, &intent_path(&dir, &stored.intent.invocation), &bytes)
        })
        .await
    }

    async fn record_completion(&self, completion: &ExecutionCompletion) -> Result<()> {
        validate_completion(completion)?;
        let stored = StoredCompletion {
            version: VERSION,
            recorded_at: crate::threads::now_iso(),
            completion: completion.clone(),
        };
        validate_header(stored.version, &stored.recorded_at)?;
        let bytes = encode(&stored)?;
        let dir = self.dir.clone();
        blocking(move || {
            check_directory(&dir)?;
            let intent = load_intent(&dir, &stored.completion.invocation)?;
            if intent.intent.call != stored.completion.call {
                return Err(Error::conflict(
                    "Execution completion does not match its original call.",
                ));
            }
            publish_new(
                &dir,
                &completion_path(&dir, &stored.completion.invocation),
                &bytes,
            )
        })
        .await
    }
}

impl FileExecutionJournal {
    /// Publishes one more immutable file for an invocation whose intent already exists.
    async fn publish_for_intent(
        &self,
        invocation: InvocationId,
        suffix: &'static str,
        bytes: Vec<u8>,
    ) -> Result<()> {
        let dir = self.dir.clone();
        blocking(move || {
            check_directory(&dir)?;
            load_intent(&dir, &invocation)?;
            publish_new(&dir, &record_path(&dir, &invocation, suffix), &bytes)
        })
        .await
    }
}

/// Same scope as recovery admission: this project's canonical root at this location, plus
/// older records that lack a canonical root.
fn concerns(
    record: &ExecutionRecord,
    project: &crate::location::ProjectRoot,
    canonical_root: &Path,
) -> bool {
    record.intent.project.location == project.location
        && record
            .intent
            .canonical_root
            .as_deref()
            .is_none_or(|root| Path::new(root) == canonical_root)
}

fn is_owner_lock(name: &str) -> bool {
    name.strip_suffix(".owner.lock")
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn owner_lock_path(dir: &Path, location: &Location, canonical_root: &Path) -> Result<PathBuf> {
    use sha2::{Digest as _, Sha256};

    if !canonical_root.is_absolute() {
        return Err(Error::invalid(
            "Execution ownership requires a canonical absolute project root.",
        ));
    }
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(location)?);
    hash.update([0]);
    hash.update(canonical_root.as_os_str().as_encoded_bytes());
    let name: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(dir.join(format!("{name}.owner.lock")))
}

/// Holds the project's lock for as long as the returned value lives. The lock belongs to
/// one open file, so a second attempt conflicts even within one process, and the operating
/// system releases it when the owning process dies.
///
/// Unix uses `flock`. Windows opens the file without sharing, which no other handle can do
/// while this one is open. Filesystems or platforms without either fail closed.
fn lock_project(path: &Path) -> Result<OwnershipGuard> {
    const OWNED: &str = "Another process owns command execution for this project.";
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const ERROR_SHARING_VIOLATION: i32 = 32;
        options.share_mode(0);
        match options.open(path) {
            Ok(file) => Ok(Box::new(file)),
            Err(error) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
                Err(Error::conflict(OWNED))
            }
            Err(error) => Err(error.into()),
        }
    }
    #[cfg(any(
        all(target_os = "linux", not(target_env = "uclibc")),
        target_os = "macos"
    ))]
    {
        use nix::errno::Errno;
        use nix::fcntl::{Flock, FlockArg};

        let file = options.open(path)?;
        if !fs::symlink_metadata(path)?.is_file() {
            return Err(Error::forbidden(
                "Execution ownership lock is not a regular file.",
            ));
        }
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => Ok(Box::new(lock)),
            Err((_, Errno::EWOULDBLOCK)) => Err(Error::conflict(OWNED)),
            Err(_) => Err(Error::tool(
                "Execution ownership could not be locked on this filesystem.",
            )),
        }
    }
    #[cfg(not(any(
        windows,
        all(target_os = "linux", not(target_env = "uclibc")),
        target_os = "macos"
    )))]
    {
        let _ = (options, path, OWNED);
        Err(Error::tool(
            "Cross-process execution ownership is not supported on this platform.",
        ))
    }
}

fn validate_identity(identity: &TreeIdentity) -> Result<()> {
    let TreeIdentity::UnixGroup { group, boot_id } = identity;
    if *group <= 1
        || boot_id
            .as_deref()
            .is_some_and(|id| !bounded_text(id, 64) || id.chars().any(char::is_control))
    {
        return Err(Error::invalid("Invalid process-tree identity."));
    }
    Ok(())
}

/// Reads one optional companion record. Present-but-unusable is an error, never "absent".
fn read_optional<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => read_record(path).map(Some),
    }
}

/// Decides, for one unknown invocation whose project lock the caller holds, whether its
/// termination is established. The error is the reason shown for an unresolved invocation.
fn termination_evidence(
    dir: &Path,
    record: &ExecutionRecord,
) -> std::result::Result<TerminationEvidence, &'static str> {
    const UNREADABLE: &str = "Its launch history could not be read.";
    let id = &record.intent.invocation;
    let version = load_intent(dir, id).map_err(|_| UNREADABLE)?.version;
    if version < FIRST_VERSION_WITH_LAUNCH_MARKER {
        return Err("It was recorded before launches were tracked.");
    }
    let launching: Option<StoredLaunching> =
        read_optional(&record_path(dir, id, ".launching.json")).map_err(|_| UNREADABLE)?;
    let Some(launching) = launching else {
        return Ok(TerminationEvidence::NeverLaunched);
    };
    if &launching.invocation != id
        || validate_header(launching.version, &launching.recorded_at).is_err()
    {
        return Err(UNREADABLE);
    }
    match launching.containment {
        Containment::OwnerBound => Ok(TerminationEvidence::OwnerBoundTreeReleased),
        Containment::Unverifiable => {
            Err("Its process tree was not contained in a way that can be checked.")
        }
        Containment::ProcessGroup => {
            let launched: Option<StoredLaunched> =
                read_optional(&record_path(dir, id, ".launched.json")).map_err(|_| UNREADABLE)?;
            let Some(launched) = launched else {
                return Err("It may have started, but its process group was never recorded.");
            };
            if &launched.invocation != id
                || validate_header(launched.version, &launched.recorded_at).is_err()
                || validate_identity(&launched.identity).is_err()
            {
                return Err(UNREADABLE);
            }
            let TreeIdentity::UnixGroup { group, boot_id } = launched.identity;
            if let (Some(recorded), Some(current)) = (&boot_id, current_boot_id())
                && recorded != &current
            {
                return Ok(TerminationEvidence::HostRestarted);
            }
            match recorded_group_is_absent(group) {
                Ok(true) => Ok(TerminationEvidence::ProcessGroupAbsent),
                Ok(false) => Err("A process group with its recorded identity still exists. \
                     The command, or an unrelated program that reused the identity, is running."),
                Err(_) => Err("Its process group could not be observed."),
            }
        }
    }
}

fn publish_reconciliation(
    dir: &Path,
    invocation: &InvocationId,
    evidence: TerminationEvidence,
) -> Result<()> {
    let stored = StoredReconciliation {
        version: VERSION,
        recorded_at: crate::threads::now_iso(),
        invocation: invocation.clone(),
        evidence,
    };
    publish_new(
        dir,
        &record_path(dir, invocation, ".reconciled.json"),
        &encode(&stored)?,
    )
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Error::internal("Execution journal worker stopped."))?
}

fn ensure_supported() -> Result<()> {
    if cfg!(any(unix, windows)) {
        Ok(())
    } else {
        Err(Error::tool(
            "Durable command execution history is not supported on this platform yet. \
             Command execution remains unavailable.",
        ))
    }
}

fn check_directory(dir: &Path) -> Result<()> {
    ensure_supported()?;
    let metadata = fs::symlink_metadata(dir)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::forbidden(
            "Execution journal must be a regular private directory.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::forbidden(
                "Execution journal directory must not be accessible to other users.",
            ));
        }
    }
    Ok(())
}

fn sync_directory(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // Required to open a directory at all.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        // FlushFileBuffers is refused on a handle without write access.
        OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)?
            .sync_all()?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        ensure_supported()
    }
}

/// Persists newly created ancestor directory entries as well as the journal itself.
fn sync_created_directories(dir: &Path) -> Result<()> {
    sync_directory(dir)?;
    for ancestor in dir.ancestors().skip(1) {
        match sync_directory(ancestor) {
            Ok(()) => {}
            // An ancestor this user cannot open for writing was not created by this user
            // just now. The journal directory itself was flushed above.
            Err(_) if cfg!(windows) => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Flushes a record through its published name after linking.
///
/// On Unix the link is a directory entry and the directory sync covers it. On Windows the
/// file's own metadata changed too, and it can only be flushed through a writable handle.
fn sync_published(destination: &Path) -> Result<()> {
    #[cfg(windows)]
    OpenOptions::new()
        .write(true)
        .open(destination)?
        .sync_all()?;
    #[cfg(not(windows))]
    let _ = destination;
    Ok(())
}

fn intent_path(dir: &Path, id: &InvocationId) -> PathBuf {
    dir.join(format!("{}.intent.json", id.as_str()))
}

fn record_path(dir: &Path, id: &InvocationId, suffix: &str) -> PathBuf {
    dir.join(format!("{}{suffix}", id.as_str()))
}

fn completion_path(dir: &Path, id: &InvocationId) -> PathBuf {
    dir.join(format!("{}.completion.json", id.as_str()))
}

fn encode(record: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(record)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(Error::invalid(
            "Execution history record exceeds the size limit.",
        ));
    }
    Ok(bytes)
}

/// Publication is atomic and never replaces an existing invocation or observation.
fn publish_new(dir: &Path, destination: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);

        // Unlike rename, hard_link refuses to overwrite an existing destination on Unix and
        // on Windows. Both names are on the same filesystem. Filesystems lacking this
        // operation fail closed.
        fs::hard_link(&temporary, destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::conflict("Execution journal record already exists; it was not replaced.")
            } else {
                error.into()
            }
        })?;
        sync_published(destination)?;
        sync_directory(dir)
    })();
    // A crash may leave this harmless extra link. Recovery never interprets .tmp files.
    let _ = fs::remove_file(&temporary);
    result
}

fn read_record<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::invalid(
            "Execution history record is not a regular file.",
        ));
    }
    if metadata.len() > MAX_RECORD_BYTES as u64 {
        return Err(Error::invalid(
            "Execution history record exceeds the size limit.",
        ));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(Error::invalid(
            "Execution history record exceeds the size limit.",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| Error::invalid("Execution history record is malformed."))
}

fn validate_header(version: u32, recorded_at: &str) -> Result<()> {
    if !(1..=VERSION).contains(&version)
        || recorded_at.len() > 64
        || time::OffsetDateTime::parse(recorded_at, &time::format_description::well_known::Rfc3339)
            .is_err()
    {
        return Err(Error::invalid(
            "Unsupported or malformed execution history header.",
        ));
    }
    Ok(())
}

fn bounded_text(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.len() <= limit && !text.contains('\0')
}

fn validate_call(call: &super::CallId) -> Result<()> {
    if !bounded_text(&call.turn, 256)
        || !bounded_text(&call.call, 256)
        || call.turn.chars().any(char::is_control)
        || call.call.chars().any(char::is_control)
    {
        return Err(Error::invalid("Invalid execution history call identity."));
    }
    Ok(())
}

fn validate_intent(intent: &ExecutionIntent) -> Result<()> {
    validate_call(&intent.call)?;
    intent.request.validate()?;
    if !bounded_text(&intent.project.path, 32 * 1024)
        || intent
            .canonical_root
            .as_deref()
            .is_some_and(|root| !bounded_text(root, 32 * 1024))
        || !bounded_text(&intent.cwd, 32 * 1024)
        || !bounded_text(&intent.program, 32 * 1024)
        || !bounded_text(&intent.environment_policy, 256)
        || intent.args.len() > 128
        || intent
            .args
            .iter()
            .any(|arg| arg.len() > super::MAX_REQUEST_BYTES || arg.contains('\0'))
    {
        return Err(Error::invalid("Invalid execution history invocation."));
    }
    if let Location::Wsl { distro } = &intent.project.location
        && (!bounded_text(distro, 256) || distro.chars().any(char::is_control))
    {
        return Err(Error::invalid("Invalid execution history location."));
    }
    Ok(())
}

fn validate_completion(completion: &ExecutionCompletion) -> Result<()> {
    validate_call(&completion.call)?;
    let valid = match completion.status {
        ExecutionStatus::Success => {
            completion.spawned
                && completion.exit_code == Some(0)
                && !completion.cancelled
                && !completion.timed_out
        }
        ExecutionStatus::Failure => {
            completion.spawned
                && completion.exit_code != Some(0)
                && !completion.cancelled
                && !completion.timed_out
        }
        ExecutionStatus::FailedToStart => {
            !completion.spawned
                && completion.exit_code.is_none()
                && !completion.cancelled
                && !completion.timed_out
        }
        ExecutionStatus::Cancelled => completion.cancelled && !completion.timed_out,
        ExecutionStatus::TimedOut => {
            completion.spawned && completion.timed_out && !completion.cancelled
        }
        ExecutionStatus::Unknown => !(completion.cancelled && completion.timed_out),
    };
    if !valid
        || (!completion.spawned
            && (completion.exit_code.is_some()
                || completion.stdout_truncated
                || completion.stderr_truncated
                || completion.output_incomplete))
    {
        return Err(Error::invalid(
            "Contradictory execution completion observation.",
        ));
    }
    Ok(())
}

fn load_intent(dir: &Path, id: &InvocationId) -> Result<StoredIntent> {
    let stored: StoredIntent = read_record(&intent_path(dir, id))?;
    validate_header(stored.version, &stored.recorded_at)?;
    validate_intent(&stored.intent)?;
    if &stored.intent.invocation != id {
        return Err(Error::conflict(
            "Execution intent identity does not match its filename.",
        ));
    }
    Ok(stored)
}

fn load_record(dir: &Path, id: &InvocationId) -> Result<ExecutionRecord> {
    let stored = load_intent(dir, id)?;
    let mut record = ExecutionRecord {
        intent: stored.intent,
        intended_at: stored.recorded_at,
        completion: None,
        completed_at: None,
        recovery_warning: None,
        reconciliation: None,
        reconciled_at: None,
    };
    // An unusable reconciliation record is no evidence: the invocation keeps blocking.
    if let Ok(Some(stored)) =
        read_optional::<StoredReconciliation>(&record_path(dir, id, ".reconciled.json"))
        && &stored.invocation == id
        && validate_header(stored.version, &stored.recorded_at).is_ok()
    {
        record.reconciliation = Some(stored.evidence);
        record.reconciled_at = Some(stored.recorded_at);
    }
    let completion = (|| {
        let stored: StoredCompletion = read_record(&completion_path(dir, id))?;
        validate_header(stored.version, &stored.recorded_at)?;
        validate_completion(&stored.completion)?;
        if &stored.completion.invocation != id || stored.completion.call != record.intent.call {
            return Err(Error::conflict(
                "Execution completion identity does not match its intent.",
            ));
        }
        Ok(stored)
    })();
    match completion {
        Ok(stored) => {
            record.completion = Some(stored.completion);
            record.completed_at = Some(stored.recorded_at);
        }
        Err(error) => {
            record.recovery_warning = Some(
                if error.code == ErrorCode::NotFound {
                    "No confirmed completion was recorded. Outcome unknown; do not replay."
                } else {
                    "Completion could not be verified. Outcome unknown; do not replay."
                }
                .into(),
            );
        }
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CallId, Command, CommandRequest};
    use crate::location::ProjectRoot;
    use contextual_protocol::commands::CommandPlatform;

    /// The canonical project root on record: absolute on the platform under test.
    const ROOT: &str = if cfg!(windows) {
        "C:\\project"
    } else {
        "/project"
    };

    fn intent() -> ExecutionIntent {
        ExecutionIntent {
            invocation: InvocationId::new(),
            call: CallId {
                turn: "turn-1".into(),
                call: "call-1".into(),
            },
            project: ProjectRoot {
                location: Location::Local,
                path: "/project".into(),
            },
            canonical_root: Some(ROOT.into()),
            platform: CommandPlatform::Linux,
            environment_policy: super::super::prepare::ENVIRONMENT_POLICY.into(),
            request: CommandRequest {
                version: 1,
                purpose: "Test execution history without starting a process.".into(),
                command: Command::Exec {
                    program: "tool".into(),
                    args: vec!["a b".into()],
                },
                cwd: ".".into(),
                timeout_seconds: 30,
                environment: Vec::new(),
            },
            cwd: "/project".into(),
            program: "/bin/tool".into(),
            args: vec!["a b".into()],
        }
    }

    fn completion(intent: &ExecutionIntent) -> ExecutionCompletion {
        ExecutionCompletion {
            invocation: intent.invocation.clone(),
            call: intent.call.clone(),
            status: ExecutionStatus::Success,
            exit_code: Some(0),
            spawned: true,
            cancelled: false,
            timed_out: false,
            stdout_truncated: true,
            stderr_truncated: false,
            output_incomplete: false,
        }
    }

    #[tokio::test]
    async fn observations_roundtrip_without_authorization_or_output() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        let completion = completion(&intent);
        journal.record_intent(&intent).await.unwrap();
        journal.record_completion(&completion).await.unwrap();

        let reopened = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let record = reopened.load(&intent.invocation).await.unwrap();
        assert_eq!(record.intent, intent);
        assert_eq!(record.completion, Some(completion));
        assert_eq!(record.status(), ExecutionStatus::Success);
        assert!(record.recovery_warning.is_none());
        assert!(record.completed_at.is_some());

        let stored: serde_json::Value = serde_json::from_slice(
            &fs::read(intent_path(journal.dir(), &intent.invocation)).unwrap(),
        )
        .unwrap();
        let object = stored["intent"].as_object().unwrap();
        for forbidden in [
            "approval",
            "binding",
            "hostEpoch",
            "environment",
            "stdout",
            "stderr",
        ] {
            assert!(!object.contains_key(forbidden), "{forbidden}");
        }
        // Only explicitly permitted model overrides occur inside the reviewed request.
        assert_eq!(
            stored["intent"]["request"]["environment"],
            serde_json::json!([])
        );
    }

    #[tokio::test]
    async fn interrupted_execution_restores_as_unknown_not_failed_or_pending() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();

        let reopened = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let record = reopened.load(&intent.invocation).await.unwrap();
        assert_eq!(record.status(), ExecutionStatus::Unknown);
        assert!(record.blocks_execution());
        assert!(record.completion.is_none());
        assert!(record.recovery_warning.is_some());
    }

    #[tokio::test]
    async fn replay_cannot_replace_the_original_intent_after_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let original = intent();
        journal.record_intent(&original).await.unwrap();

        let reopened = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let mut changed = original.clone();
        changed.args.push("--different".into());
        assert_eq!(
            reopened.record_intent(&changed).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(
            reopened.load(&original.invocation).await.unwrap().intent,
            original
        );
    }

    #[tokio::test]
    async fn independently_opened_writers_cannot_both_publish_the_same_invocation() {
        let directory = tempfile::tempdir().unwrap();
        let first = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let second = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        let (one, two) = tokio::join!(first.record_intent(&intent), second.record_intent(&intent));
        assert_ne!(one.is_ok(), two.is_ok());
        assert_eq!(first.records().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn completion_requires_an_intent_and_matching_call() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        let mut completed = completion(&intent);
        assert!(journal.record_completion(&completed).await.is_err());

        journal.record_intent(&intent).await.unwrap();
        completed.call.call = "another-call".into();
        assert!(journal.record_completion(&completed).await.is_err());
        assert!(
            journal
                .load(&intent.invocation)
                .await
                .unwrap()
                .blocks_execution()
        );
    }

    #[tokio::test]
    async fn completion_is_immutable_even_when_a_second_write_is_identical() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        let completed = completion(&intent);
        journal.record_intent(&intent).await.unwrap();
        journal.record_completion(&completed).await.unwrap();
        assert_eq!(
            journal
                .record_completion(&completed)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn corrupt_completion_preserves_unknown_intent_and_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        fs::write(
            completion_path(journal.dir(), &intent.invocation),
            b"{partial",
        )
        .unwrap();

        let record = journal.load(&intent.invocation).await.unwrap();
        assert!(record.blocks_execution());
        assert!(record.recovery_warning.is_some());
        assert!(
            journal
                .record_completion(&completion(&intent))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn corrupt_intent_fails_recovery_instead_of_disappearing() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        fs::write(intent_path(journal.dir(), &intent.invocation), b"{partial").unwrap();
        assert!(journal.records().await.is_err());
    }

    #[tokio::test]
    async fn oversized_completion_is_unknown_without_unbounded_reading() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        let file = File::create(completion_path(journal.dir(), &intent.invocation)).unwrap();
        file.set_len(MAX_RECORD_BYTES as u64 + 1).unwrap();
        assert!(
            journal
                .load(&intent.invocation)
                .await
                .unwrap()
                .blocks_execution()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_completion_is_not_followed() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(
            outside.path(),
            completion_path(journal.dir(), &intent.invocation),
        )
        .unwrap();
        assert!(
            journal
                .load(&intent.invocation)
                .await
                .unwrap()
                .blocks_execution()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_permissions_are_applied_at_creation() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        assert_eq!(
            fs::metadata(journal.dir()).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            fs::metadata(intent_path(journal.dir(), &intent.invocation))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }

    #[tokio::test]
    async fn unfinished_temporary_files_are_not_execution_intent() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        fs::write(journal.dir().join(".interrupted.tmp"), b"{partial").unwrap();
        assert!(journal.records().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn local_and_wsl_locations_survive_recovery_without_translation() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let local = intent();
        let mut remote = intent();
        remote.project.location = Location::Wsl {
            distro: "Ubuntu".into(),
        };
        journal.record_intent(&local).await.unwrap();
        journal.record_intent(&remote).await.unwrap();
        let records = journal.records().await.unwrap();
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .any(|record| record.intent.project == local.project)
        );
        assert!(
            records
                .iter()
                .any(|record| record.intent.project == remote.project)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_permissive_directory_is_rejected_without_changing_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing");
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();

        let result = FileExecutionJournal::open(path.clone()).await;
        let error = result.err().expect("permissive directory must be rejected");
        assert_eq!(error.code, ErrorCode::Forbidden);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn recovery_uses_canonical_root_instead_of_the_requested_alias() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let mut pending = intent();
        pending.project.path = "/old-alias".into();
        journal.record_intent(&pending).await.unwrap();

        let current = ProjectRoot {
            location: Location::Local,
            path: "/new-alias".into(),
        };
        assert!(
            journal
                .check_recovery(&current, Path::new(ROOT))
                .await
                .is_err()
        );
        assert!(
            journal
                .check_recovery(&current, Path::new("/different-project"))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn recovery_does_not_confuse_a_wsl_location_with_the_local_host() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let mut pending = intent();
        pending.project.location = Location::Wsl {
            distro: "Ubuntu".into(),
        };
        journal.record_intent(&pending).await.unwrap();

        let local = intent().project;
        assert!(
            journal
                .check_recovery(&local, Path::new(ROOT))
                .await
                .is_ok()
        );
        assert!(
            journal
                .check_recovery(&pending.project, Path::new(ROOT))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn confirmed_completion_allows_recovery_admission_without_replaying_it() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let finished = intent();
        journal.record_intent(&finished).await.unwrap();
        journal
            .record_completion(&completion(&finished))
            .await
            .unwrap();

        assert!(
            journal
                .check_recovery(&finished.project, Path::new(ROOT))
                .await
                .is_ok()
        );
        assert_eq!(journal.records().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn older_unknown_history_without_a_canonical_root_blocks_conservatively() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let pending = intent();
        journal.record_intent(&pending).await.unwrap();

        let path = intent_path(journal.dir(), &pending.invocation);
        let mut stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        stored["intent"]
            .as_object_mut()
            .unwrap()
            .remove("canonicalRoot");
        fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();

        let record = journal.load(&pending.invocation).await.unwrap();
        assert!(record.intent.canonical_root.is_none());
        assert!(record.blocks_execution());
        assert!(
            journal
                .check_recovery(&pending.project, Path::new("/another-root"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn new_intents_cannot_omit_the_canonical_root() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let mut pending = intent();
        pending.canonical_root = None;
        assert!(journal.record_intent(&pending).await.is_err());
        assert!(journal.records().await.unwrap().is_empty());
    }

    #[test]
    fn contradictory_success_is_not_a_verified_completion() {
        let mut completed = completion(&intent());
        completed.cancelled = true;
        assert!(validate_completion(&completed).is_err());
    }

    #[tokio::test]
    async fn publication_leaves_one_durable_record_and_no_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("a").join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        journal
            .record_completion(&completion(&intent))
            .await
            .unwrap();

        let mut names: Vec<String> = fs::read_dir(journal.dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let id = intent.invocation.as_str();
        assert_eq!(
            names,
            [format!("{id}.completion.json"), format!("{id}.intent.json")]
        );
        assert_eq!(journal.records().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_publication_leaves_nothing_that_recovery_reads_as_intent() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        // Something that is not a record already occupies the published name.
        fs::create_dir(intent_path(journal.dir(), &intent.invocation)).unwrap();

        assert_eq!(
            journal.record_intent(&intent).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        assert!(journal.load(&intent.invocation).await.is_err());
        // Recovery fails rather than reporting an empty history.
        assert!(journal.records().await.is_err());
        let project = intent.project.clone();
        assert!(
            journal
                .check_recovery(&project, Path::new(ROOT))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_journal_path_that_is_a_file_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal");
        fs::write(&path, b"").unwrap();
        assert!(FileExecutionJournal::open(path).await.is_err());
    }

    #[tokio::test]
    async fn a_removed_journal_directory_refuses_intent() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        fs::remove_dir(journal.dir()).unwrap();

        assert!(journal.record_intent(&intent()).await.is_err());
        assert!(journal.records().await.is_err());
    }

    /// Another process holding a record without sharing must never make it disappear from
    /// recovery or become replaceable.
    #[cfg(windows)]
    #[tokio::test]
    async fn records_locked_by_another_handle_fail_closed() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        journal
            .record_completion(&completion(&intent))
            .await
            .unwrap();
        let exclusive = |path: PathBuf| {
            OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(path)
                .unwrap()
        };

        // An unreadable completion is an unknown outcome that blocks the project.
        let held = exclusive(completion_path(journal.dir(), &intent.invocation));
        let record = journal.load(&intent.invocation).await.unwrap();
        assert_eq!(record.status(), ExecutionStatus::Unknown);
        assert!(record.recovery_warning.is_some());
        assert!(
            journal
                .check_recovery(&intent.project, Path::new(ROOT))
                .await
                .is_err()
        );
        assert_eq!(
            journal
                .record_completion(&completion(&intent))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        drop(held);

        // An unreadable intent fails recovery; it is not skipped.
        let held = exclusive(intent_path(journal.dir(), &intent.invocation));
        assert!(journal.load(&intent.invocation).await.is_err());
        assert!(journal.records().await.is_err());
        assert!(
            journal
                .check_recovery(&intent.project, Path::new(ROOT))
                .await
                .is_err()
        );
        assert_eq!(
            journal.record_intent(&intent).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        drop(held);

        assert_eq!(
            journal.load(&intent.invocation).await.unwrap().status(),
            ExecutionStatus::Success
        );
    }

    /// Creating a symbolic link on Windows needs a privilege or developer mode; without
    /// it this test reports that it could not check anything.
    #[cfg(windows)]
    #[tokio::test]
    async fn symlinked_records_and_directories_are_not_followed() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let directory = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap();
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        let outside = directory.path().join("outside.json");
        fs::write(&outside, b"{}").unwrap();
        if symlink_file(&outside, completion_path(journal.dir(), &intent.invocation)).is_err() {
            eprintln!("skipped: this account cannot create symbolic links");
            return;
        }

        let record = journal.load(&intent.invocation).await.unwrap();
        assert_eq!(record.status(), ExecutionStatus::Unknown);
        assert_eq!(fs::read(&outside).unwrap(), b"{}");

        let alias = directory.path().join("alias");
        symlink_dir(journal.dir(), &alias).unwrap();
        assert!(FileExecutionJournal::open(alias).await.is_err());
    }

    // ----- M3: cross-process ownership and reconciliation -----

    fn project() -> crate::location::ProjectRoot {
        intent().project
    }

    async fn journal_in(directory: &tempfile::TempDir) -> FileExecutionJournal {
        FileExecutionJournal::open(directory.path().join("journal"))
            .await
            .unwrap()
    }

    /// An invocation that reached launch with this containment and has no completion.
    async fn launched(journal: &FileExecutionJournal, containment: Containment) -> ExecutionIntent {
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        journal
            .record_launching(&intent.invocation, containment)
            .await
            .unwrap();
        intent
    }

    async fn reconcile(journal: &FileExecutionJournal) -> ReconciliationReport {
        journal
            .reconcile(&project(), Path::new(ROOT))
            .await
            .unwrap()
    }

    async fn blocked(journal: &FileExecutionJournal) -> bool {
        journal
            .check_recovery(&project(), Path::new(ROOT))
            .await
            .is_err()
    }

    #[tokio::test]
    async fn ownership_excludes_every_other_holder_until_it_is_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let first = journal_in(&directory).await;
        let second = journal_in(&directory).await;
        let root = Path::new(ROOT);

        let held = first.acquire_ownership(&project(), root).await.unwrap();
        // The same journal, an independently opened one, and reconciliation are all refused.
        for journal in [&first, &second] {
            let refused = journal.acquire_ownership(&project(), root).await;
            assert_eq!(refused.err().unwrap().code, ErrorCode::Conflict);
            assert_eq!(
                journal.reconcile(&project(), root).await.unwrap_err().code,
                ErrorCode::Conflict
            );
        }
        // Another project, and the same path at another location, are separate.
        let _other = second
            .acquire_ownership(&project(), &Path::new(ROOT).join("other"))
            .await
            .unwrap();
        let wsl = crate::location::ProjectRoot {
            location: Location::Wsl {
                distro: "Ubuntu".into(),
            },
            path: "/project".into(),
        };
        let _wsl = second.acquire_ownership(&wsl, root).await.unwrap();

        drop(held);
        let _next = second.acquire_ownership(&project(), root).await.unwrap();
        // Lock files are not history.
        assert!(first.records().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_intent_that_never_reached_launch_is_reconciled_as_never_launched() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        assert!(blocked(&journal).await);

        let report = reconcile(&journal).await;
        assert!(!report.blocked());
        assert_eq!(report.invocations.len(), 1);
        assert_eq!(report.invocations[0].invocation, intent.invocation);
        assert_eq!(
            report.invocations[0].evidence,
            Some(TerminationEvidence::NeverLaunched)
        );
        assert!(!blocked(&journal).await);

        // The outcome is still unknown; only its blocking effect is resolved, durably.
        let record = journal_in(&directory)
            .await
            .load(&intent.invocation)
            .await
            .unwrap();
        assert_eq!(record.status(), ExecutionStatus::Unknown);
        assert_eq!(
            record.reconciliation,
            Some(TerminationEvidence::NeverLaunched)
        );
        assert!(record.completion.is_none() && !record.blocks_execution());
        // Nothing is left to reconcile, and the original intent still cannot be replayed.
        assert!(reconcile(&journal).await.invocations.is_empty());
        assert_eq!(
            journal.record_intent(&intent).await.unwrap_err().code,
            ErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn an_intent_from_before_launch_markers_cannot_be_reconciled() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let intent = intent();
        let stored = StoredIntent {
            version: 1,
            recorded_at: crate::threads::now_iso(),
            intent: intent.clone(),
        };
        fs::write(
            intent_path(journal.dir(), &intent.invocation),
            serde_json::to_vec(&stored).unwrap(),
        )
        .unwrap();

        let report = reconcile(&journal).await;
        assert!(report.blocked());
        assert!(report.invocations[0].detail.is_some());
        assert!(blocked(&journal).await);
        assert!(
            journal
                .load(&intent.invocation)
                .await
                .unwrap()
                .reconciliation
                .is_none()
        );
    }

    #[tokio::test]
    async fn containment_decides_what_a_released_owner_proves() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;

        // Nothing checkable: stays blocked.
        launched(&journal, Containment::Unverifiable).await;
        assert!(reconcile(&journal).await.blocked());
        assert!(blocked(&journal).await);

        // A process group whose identity was never recorded: stays blocked.
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        launched(&journal, Containment::ProcessGroup).await;
        assert!(reconcile(&journal).await.blocked());
        assert!(blocked(&journal).await);

        // An owner-bound tree ended with its owner, whose lock we now hold.
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        launched(&journal, Containment::OwnerBound).await;
        let report = reconcile(&journal).await;
        assert_eq!(
            report.invocations[0].evidence,
            Some(TerminationEvidence::OwnerBoundTreeReleased)
        );
        assert!(!blocked(&journal).await);
    }

    #[cfg(any(
        all(target_os = "linux", not(target_env = "uclibc")),
        target_os = "macos"
    ))]
    #[tokio::test]
    async fn a_recorded_process_group_must_be_gone_before_it_is_reconciled() {
        use std::os::unix::process::CommandExt as _;

        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let intent = launched(&journal, Containment::ProcessGroup).await;
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exec sleep 30"])
            .process_group(0)
            .spawn()
            .unwrap();
        let identity = TreeIdentity::UnixGroup {
            group: child.id() as i32,
            boot_id: current_boot_id(),
        };
        journal
            .record_launched(&intent.invocation, &identity)
            .await
            .unwrap();

        // Running: unresolved, however often it is asked.
        for _ in 0..2 {
            let report = reconcile(&journal).await;
            assert!(report.blocked() && report.invocations[0].detail.is_some());
            assert!(blocked(&journal).await);
        }

        // Reconciliation never ends a process; the test does, then reaps it.
        child.kill().unwrap();
        child.wait().unwrap();
        let report = reconcile(&journal).await;
        assert_eq!(
            report.invocations[0].evidence,
            Some(TerminationEvidence::ProcessGroupAbsent)
        );
        assert!(!blocked(&journal).await);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_process_group_from_an_earlier_boot_is_reconciled_as_restarted() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let intent = launched(&journal, Containment::ProcessGroup).await;
        assert!(current_boot_id().is_some());
        // This test's own group certainly exists, yet belongs to another boot on record.
        let identity = TreeIdentity::UnixGroup {
            group: nix::unistd::getpgrp().as_raw(),
            boot_id: Some("00000000-0000-0000-0000-000000000000".into()),
        };
        journal
            .record_launched(&intent.invocation, &identity)
            .await
            .unwrap();

        assert_eq!(
            reconcile(&journal).await.invocations[0].evidence,
            Some(TerminationEvidence::HostRestarted)
        );
    }

    #[tokio::test]
    async fn reconciliation_is_scoped_and_leaves_recorded_outcomes_alone() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let finished = intent();
        journal.record_intent(&finished).await.unwrap();
        journal
            .record_completion(&completion(&finished))
            .await
            .unwrap();
        let mut elsewhere = intent();
        elsewhere.canonical_root = Some(format!("{ROOT}-elsewhere"));
        journal.record_intent(&elsewhere).await.unwrap();

        assert!(reconcile(&journal).await.invocations.is_empty());
        let other = journal.load(&elsewhere.invocation).await.unwrap();
        assert!(other.blocks_execution() && other.reconciliation.is_none());
        let done = journal.load(&finished.invocation).await.unwrap();
        assert_eq!(done.status(), ExecutionStatus::Success);
        assert!(done.reconciliation.is_none());
    }

    #[tokio::test]
    async fn unusable_launch_or_reconciliation_records_keep_the_project_blocked() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;

        // A launch marker that cannot be read is not "absent".
        let unreadable = intent();
        journal.record_intent(&unreadable).await.unwrap();
        fs::write(
            record_path(journal.dir(), &unreadable.invocation, ".launching.json"),
            b"{",
        )
        .unwrap();
        assert!(reconcile(&journal).await.blocked());
        assert!(blocked(&journal).await);

        // A reconciliation record that cannot be read is not evidence, and is not replaced.
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let intent = intent();
        journal.record_intent(&intent).await.unwrap();
        fs::write(
            record_path(journal.dir(), &intent.invocation, ".reconciled.json"),
            b"{",
        )
        .unwrap();
        assert!(blocked(&journal).await);
        let report = reconcile(&journal).await;
        assert!(report.blocked());
        assert!(blocked(&journal).await);
    }

    #[tokio::test]
    async fn launch_records_need_an_intent_and_are_immutable() {
        let directory = tempfile::tempdir().unwrap();
        let journal = journal_in(&directory).await;
        let absent = InvocationId::new();
        assert!(
            journal
                .record_launching(&absent, Containment::OwnerBound)
                .await
                .is_err()
        );

        let intent = launched(&journal, Containment::ProcessGroup).await;
        assert_eq!(
            journal
                .record_launching(&intent.invocation, Containment::OwnerBound)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        let invalid = TreeIdentity::UnixGroup {
            group: 1,
            boot_id: None,
        };
        assert!(
            journal
                .record_launched(&intent.invocation, &invalid)
                .await
                .is_err()
        );
    }
}
