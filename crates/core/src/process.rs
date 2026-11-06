//! Running external tools (`git`, `gh`) with a fixed argument vector — never through a shell.
//!
//! - [`Cmd`] describes one invocation: program, arguments, working directory, timeout, extra
//!   environment and an optional [`CancellationToken`].
//! - [`CommandRunner`] is the seam: [`SystemRunner`] spawns real processes; tests implement the
//!   trait to fake `gh` (or anything else) while letting `git` run for real.
//!
//! Environment: the parent's environment is inherited minus variables that would redirect git
//! to another repository (`GIT_DIR`, `GIT_WORK_TREE`, …), plus `GIT_TERMINAL_PROMPT=0`,
//! `GH_PROMPT_DISABLED=1`, `GH_NO_UPDATE_NOTIFIER=1` (and `GCM_INTERACTIVE=never` on Windows), so
//! nothing ever waits for credentials. On Windows children get `CREATE_NO_WINDOW`.
//!
//! `PATH`: GUI apps on macOS don't inherit the login shell's `PATH`, and the WSL agent starts in
//! a non-interactive environment, so [`PathPolicy::LoginShell`] resolves it once from
//! `$SHELL -ilc 'printf %s "$PATH"'`, falling back to the current `PATH` plus
//! `/opt/homebrew/bin:/usr/local/bin`.

use async_trait::async_trait;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

/// Default timeout, as in the old server (`execFile` with 30 s).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Variables that point git at another repository or index; never inherited.
const GIT_LOCATION_VARS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

/// One process invocation.
#[derive(Debug, Clone)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub timeout: Duration,
    /// Set on top of the (filtered) inherited environment.
    pub env: Vec<(String, String)>,
    pub cancel: Option<CancellationToken>,
}

impl Cmd {
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            cwd: None,
            timeout: DEFAULT_TIMEOUT,
            env: Vec::new(),
            cancel: None,
        }
    }
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
    pub fn cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }
}

/// What a finished (or failed-to-start) command produced. Mirrors the old `run()` result:
/// `stderr` carries a readable reason when the process could not run at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    /// Exited with status 0.
    pub ok: bool,
    /// Exit code, if the process ran and exited normally.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
}

impl RunOutput {
    /// A successful result with this stdout (handy for fakes).
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            ok: true,
            code: Some(0),
            stdout: stdout.into(),
            ..Self::default()
        }
    }
    /// A failed result with this stderr (handy for fakes).
    pub fn failure(code: i32, stderr: impl Into<String>) -> Self {
        Self {
            ok: false,
            code: Some(code),
            stderr: stderr.into(),
            ..Self::default()
        }
    }
}

/// Runs commands. Implement it to fake tools in tests.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, cmd: Cmd) -> RunOutput;
}

/// How [`SystemRunner`] finds programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathPolicy {
    /// Use the inherited `PATH` as is (Windows, Linux dev runs).
    Inherit,
    /// Resolve `PATH` once from the login shell (macOS app, WSL agent).
    LoginShell,
    /// Use exactly this `PATH`.
    Fixed(OsString),
}

impl PathPolicy {
    /// The right policy for this build: the login shell on macOS, inherit elsewhere.
    pub fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            PathPolicy::LoginShell
        } else {
            PathPolicy::Inherit
        }
    }
}

/// Spawns real processes.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    policy: PathPolicy,
    extra_env: Vec<(String, String)>,
    resolved_path: Arc<OnceCell<Option<OsString>>>,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::new(PathPolicy::platform_default())
    }
}

impl SystemRunner {
    pub fn new(policy: PathPolicy) -> Self {
        Self {
            policy,
            extra_env: Vec::new(),
            resolved_path: Arc::new(OnceCell::new()),
        }
    }

    /// Adds an environment variable to every command (e.g. `GIT_CONFIG_GLOBAL` in tests).
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    /// The `PATH` children get (`None` = inherited unchanged).
    pub async fn search_path(&self) -> Option<OsString> {
        self.resolved_path
            .get_or_init(|| async {
                match &self.policy {
                    PathPolicy::Inherit => None,
                    PathPolicy::Fixed(p) => Some(p.clone()),
                    PathPolicy::LoginShell => Some(login_shell_path().await),
                }
            })
            .await
            .clone()
    }
}

/// `PATH` as the user's login shell sets it, or the current one plus the usual Homebrew dirs.
pub async fn login_shell_path() -> OsString {
    if let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) {
        let mut cmd = tokio::process::Command::new(shell);
        cmd.args(["-ilc", "printf %s \"$PATH\""])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await {
            // Interactive shells may print banners first; PATH is the last line.
            let text = String::from_utf8_lossy(&out.stdout);
            let last = text.lines().last().unwrap_or("").trim();
            if out.status.success() && !last.is_empty() {
                return OsString::from(last);
            }
        }
    }
    fallback_path(std::env::var_os("PATH"))
}

/// The current `PATH` with `/opt/homebrew/bin` and `/usr/local/bin` appended when missing.
pub fn fallback_path(current: Option<OsString>) -> OsString {
    let mut dirs: Vec<PathBuf> = current
        .as_deref()
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        if !dirs.iter().any(|d| d == Path::new(extra)) {
            dirs.push(PathBuf::from(extra));
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// Finds `program` in `path` (adding `.exe` on Windows). Returns it unchanged when it already
/// names a path or isn't found (spawning then fails with "not installed").
pub fn find_program(program: &str, path: Option<&std::ffi::OsStr>) -> PathBuf {
    let p = Path::new(program);
    if p.components().count() > 1 || p.is_absolute() {
        return p.to_path_buf();
    }
    let Some(path) = path else {
        return p.to_path_buf();
    };
    let names: Vec<String> = if cfg!(windows) && p.extension().is_none() {
        vec![format!("{program}.exe"), program.to_string()]
    } else {
        vec![program.to_string()]
    };
    for dir in std::env::split_paths(path) {
        for name in &names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    p.to_path_buf()
}

#[async_trait]
impl CommandRunner for SystemRunner {
    async fn run(&self, cmd: Cmd) -> RunOutput {
        let search_path = self.search_path().await;
        let program = match &search_path {
            Some(p) => find_program(&cmd.program, Some(p)),
            None => PathBuf::from(&cmd.program),
        };
        let mut command = tokio::process::Command::new(&program);
        command
            .args(&cmd.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &cmd.cwd {
            command.current_dir(cwd);
        }
        for var in GIT_LOCATION_VARS {
            command.env_remove(var);
        }
        if let Some(p) = &search_path {
            command.env("PATH", p);
        }
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1");
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.env("GCM_INTERACTIVE", "never");
            command.creation_flags(CREATE_NO_WINDOW);
        }
        for (k, v) in self.extra_env.iter().chain(cmd.env.iter()) {
            command.env(k, v);
        }

        let child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let stderr = if e.kind() == std::io::ErrorKind::NotFound {
                    format!("{} is not installed", cmd.program)
                } else {
                    format!("{}: {e}", cmd.program)
                };
                return RunOutput {
                    ok: false,
                    stderr,
                    ..RunOutput::default()
                };
            }
        };

        let cancel = cmd.cancel.clone().unwrap_or_default();
        // Dropping `wait` (timeout or cancel) kills the child (`kill_on_drop`).
        let wait = child.wait_with_output();
        tokio::select! {
            out = wait => match out {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                    let mut stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                    let ok = out.status.success();
                    if !ok && stderr.is_empty() {
                        stderr = format!("Command failed: {} {}", cmd.program, cmd.args.join(" "));
                    }
                    RunOutput { ok, code: out.status.code(), stdout, stderr, ..RunOutput::default() }
                }
                Err(e) => RunOutput { ok: false, stderr: format!("{}: {e}", cmd.program), ..RunOutput::default() },
            },
            _ = tokio::time::sleep(cmd.timeout) => RunOutput {
                ok: false,
                stderr: format!("{} timed out after {} s", cmd.program, cmd.timeout.as_secs()),
                timed_out: true,
                ..RunOutput::default()
            },
            _ = cancel.cancelled() => RunOutput {
                ok: false,
                stderr: "Cancelled.".into(),
                cancelled: true,
                ..RunOutput::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_without_a_shell() {
        let r = SystemRunner::new(PathPolicy::Inherit);
        let out = r.run(Cmd::new("echo", ["$HOME", "a b"])).await;
        assert!(out.ok);
        assert_eq!(out.stdout, "$HOME a b\n");
    }

    #[tokio::test]
    async fn missing_program() {
        let r = SystemRunner::new(PathPolicy::Inherit);
        let out = r
            .run(Cmd::new(
                "definitely-not-a-program-xyz",
                Vec::<String>::new(),
            ))
            .await;
        assert!(!out.ok);
        assert_eq!(out.stderr, "definitely-not-a-program-xyz is not installed");
    }

    #[tokio::test]
    async fn timeout_and_cancel() {
        let r = SystemRunner::new(PathPolicy::Inherit);
        let out = r
            .run(Cmd::new("sleep", ["5"]).timeout(Duration::from_millis(100)))
            .await;
        assert!(out.timed_out && !out.ok);

        let token = CancellationToken::new();
        let t2 = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            t2.cancel();
        });
        let started = std::time::Instant::now();
        let out = r.run(Cmd::new("sleep", ["5"]).cancel(token)).await;
        assert!(out.cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn env_is_filtered_and_set() {
        let r = SystemRunner::new(PathPolicy::Inherit).with_env("EXTRA_X", "1");
        let out = r
            .run(Cmd::new("env", Vec::<String>::new()).env("GIT_DIR", "/nope"))
            .await;
        assert!(out.stdout.contains("GIT_TERMINAL_PROMPT=0"));
        assert!(out.stdout.contains("EXTRA_X=1"));
        // Explicit per-command env still wins over the filter.
        assert!(out.stdout.contains("GIT_DIR=/nope"));
    }

    #[test]
    fn fallback_appends_homebrew() {
        let p = fallback_path(Some(OsString::from("/usr/bin:/usr/local/bin")));
        assert_eq!(
            p,
            OsString::from("/usr/bin:/usr/local/bin:/opt/homebrew/bin")
        );
    }

    #[tokio::test]
    async fn fixed_path_resolves_programs() {
        let r = SystemRunner::new(PathPolicy::Fixed(OsString::from(
            "/nonexistent:/usr/bin:/bin",
        )));
        let out = r.run(Cmd::new("true", Vec::<String>::new())).await;
        assert!(out.ok);
    }
}
