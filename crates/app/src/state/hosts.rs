//! Workspace hosts by location (plan §6.2): the in-process host for local projects and one
//! supervised agent per WSL distro, started on first use.
//!
//! `CONTEXTUAL_DEV_REMOTE=1` (debug builds) treats every project as living in a "dev" distro
//! served by a locally spawned `contextual-agent`, so the remote path can be exercised on Linux.
//!
 //! Command execution is always configured: the in-process host gets this platform's native
//! command backend, and agents are started in their explicit `--commands` mode for their
//! distro. Every command still requires the user's approval of the exact reviewed plan.
//! An unavailable backend or durable history leaves execution unavailable.
//!
//! `CONTEXTUAL_DEV_COMMANDS_JOURNAL=<absolute dir>` moves the durable history of the
//! in-process host and of a locally spawned agent (default: the app or agent data directory).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use contextual_core::commands::execution::ManagedCommandRunner;
use contextual_core::commands::journal::FileExecutionJournal;
use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::location::{Detected, Location, ProjectRoot, detect_for_this_os};
use contextual_core::remote::{AgentLauncher, LocalAgentLauncher, SupervisedHost};
use contextual_core::wsl::{Distro, WslLauncher, list_distros};

const DEV_DISTRO: &str = "dev";

pub struct Hosts {
    local: Arc<LocalHost>,
    remotes: HashMap<String, Arc<SupervisedHost>>,
    pub distros: Vec<Distro>,
    dev_remote: bool,
    /// The in-process host can run approved commands.
    local_commands: bool,
}

fn native_runner() -> Option<Arc<dyn ManagedCommandRunner>> {
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    return Some(Arc::new(
        contextual_core::commands::execution::linux::LinuxCommandRunner::new(),
    ));
    #[cfg(target_os = "macos")]
    return Some(Arc::new(
        contextual_core::commands::execution::macos::MacosCommandRunner::new(),
    ));
    #[cfg(windows)]
    return Some(Arc::new(
        contextual_core::commands::execution::windows::WindowsCommandRunner::new(),
    ));
    #[allow(unreachable_code)]
    None
}

/// The in-process host with the native command backend and durable history.
/// If either is unavailable, execution remains unavailable.
fn local_host_with_commands() -> Option<LocalHost> {
    let runner = native_runner()?;
    let journal = std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        runtime
            .block_on(async {
                match std::env::var("CONTEXTUAL_DEV_COMMANDS_JOURNAL") {
                    Ok(dir) => FileExecutionJournal::open(PathBuf::from(dir)).await,
                    Err(_) => FileExecutionJournal::open_default().await,
                }
            })
            .map_err(|error| {
                eprintln!(
                    "contextual: command history is unavailable, commands stay off: {}",
                    error.message
                )
            })
            .ok()
    })
    .join()
    .ok()??;
    Some(LocalHost::with_command_backend(runner, Arc::new(journal)))
}

/// The directory of the running executable (bundled agents live next to it).
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default()
}

impl Hosts {
    /// Configures local and agent hosts for commands that the user explicitly approves.
    pub fn new() -> Self {
        let dev_remote = cfg!(debug_assertions)
            && std::env::var("CONTEXTUAL_DEV_REMOTE").is_ok_and(|v| v == "1");
        let distros = if dev_remote {
            vec![Distro {
                name: DEV_DISTRO.into(),
                is_default: true,
            }]
        } else {
            list_distros()
        };
        let with_commands = local_host_with_commands();
        Self {
            local_commands: with_commands.is_some(),
            local: Arc::new(with_commands.unwrap_or_default()),
            remotes: HashMap::new(),
            distros,
            dev_remote,
        }
    }

    /// Whether this location is configured for approved commands. A WSL project runs them
    /// through its agent, which must also offer the capability; an older agent leaves them
    /// unavailable, with no local fallback.
    pub fn commands_enabled(&self, location: &Location) -> bool {
        match location {
            Location::Local => self.local_commands,
            Location::Wsl { .. } => true,
        }
    }

    pub fn default_distro(&self) -> Option<&str> {
        self.distros
            .iter()
            .find(|d| d.is_default)
            .or(self.distros.first())
            .map(|d| d.name.as_str())
    }

    /// Where what the user typed lives. In dev-remote mode everything is in the "dev" distro.
    pub fn detect(&self, input: &str) -> Result<ProjectRoot, String> {
        if self.dev_remote {
            return Ok(ProjectRoot {
                location: Location::Wsl {
                    distro: DEV_DISTRO.into(),
                },
                path: input.trim().to_string(),
            });
        }
        match detect_for_this_os(input, self.default_distro()) {
            Detected::Root(r) => Ok(r),
            Detected::NeedsWsl(p) => Err(format!(
                "{p} looks like a Linux path, but WSL isn't installed."
            )),
        }
    }

    pub fn get(&mut self, location: &Location) -> Arc<dyn WorkspaceHost> {
        match location {
            Location::Local => self.local.clone(),
            Location::Wsl { distro } => {
                let launcher = self.launcher(distro);
                self.remotes
                    .entry(distro.clone())
                    .or_insert_with(|| Arc::new(SupervisedHost::new(launcher)))
                    .clone()
            }
        }
    }

    fn launcher(&self, distro: &str) -> Arc<dyn AgentLauncher> {
        let app_version = env!("CARGO_PKG_VERSION").to_string();
        if self.dev_remote {
            // The dev agent runs on this machine but serves the "dev" distro's projects.
            let mut args = vec![
                "--commands".into(),
                "--location".into(),
                format!("wsl:{distro}"),
            ];
            if let Ok(journal) = std::env::var("CONTEXTUAL_DEV_COMMANDS_JOURNAL") {
                args.extend(["--journal".into(), format!("{journal}-agent")]);
            }
            return Arc::new(LocalAgentLauncher {
                binary: exe_dir().join("contextual-agent"),
                app_version,
                env: Vec::new(),
                command_location: Some(Location::Wsl {
                    distro: distro.to_string(),
                }),
                args,
            });
        }
        // Bundled static Linux agents (release packaging puts them next to the app); a dev
        // override points at a freshly built one.
        let mut binaries = Vec::new();
        if let Ok(p) = std::env::var("CONTEXTUAL_AGENT_BIN") {
            binaries.push(("x86_64".to_string(), PathBuf::from(p)));
        }
        for arch in ["x86_64", "aarch64"] {
            let p = exe_dir().join(format!("contextual-agent-{arch}-linux"));
            if p.is_file() {
                binaries.push((arch.to_string(), p));
            }
        }
        Arc::new(WslLauncher {
            distro: distro.to_string(),
            app_version,
            wsl: PathBuf::from("wsl.exe"),
            binaries,
            env: Vec::new(),
            commands: true,
        })
    }
}

/// Where the old app kept a project's chats: its thread store (`~/.cache/contextual/threads`,
/// inside WSL when the old app ran there) and the project path as the old server saw it.
pub enum LegacyThreads {
    /// This machine (Linux/macOS, or dev-remote mode).
    Local,
    /// Inside a WSL distro; `root` is translated (`C:\x` → `/mnt/c/x` for Windows projects).
    Wsl {
        distro: String,
        host: Arc<dyn WorkspaceHost>,
        windows_project: bool,
    },
}

impl LegacyThreads {
    pub async fn resolve(&self, root: &str) -> Option<(PathBuf, String)> {
        match self {
            LegacyThreads::Local => Some((
                contextual_core::threads::legacy_threads_dir()?,
                root.to_string(),
            )),
            LegacyThreads::Wsl {
                distro,
                host,
                windows_project,
            } => {
                // The distro user's home, from the agent's folder listing.
                let home = host
                    .list_dirs(contextual_protocol::fs::ListDirs {
                        path: None,
                        hidden: false,
                    })
                    .await
                    .ok()?
                    .path;
                let dir = format!(r"\\wsl.localhost\{distro}{home}/.cache/contextual/threads")
                    .replace('/', "\\");
                let root = if *windows_project {
                    contextual_core::workspace::paths::drive_to_mnt(root)?
                } else {
                    root.to_string()
                };
                Some((PathBuf::from(dir), root))
            }
        }
    }
}

impl Hosts {
    /// Where to look for a project's chats from the old app.
    pub fn legacy_threads(&mut self, location: &Location) -> Option<LegacyThreads> {
        if self.dev_remote || !cfg!(windows) {
            return Some(LegacyThreads::Local);
        }
        let (distro, windows_project) = match location {
            Location::Wsl { distro } => (distro.clone(), false),
            Location::Local => (self.default_distro()?.to_string(), true),
        };
        let host = self.get(&Location::Wsl {
            distro: distro.clone(),
        });
        Some(LegacyThreads::Wsl {
            distro,
            host,
            windows_project,
        })
    }
}

/// On Windows, puts this process in a job that kills its children (the WSL agents) when the app
/// exits, even if it crashes (plan §6.6).
pub fn kill_children_with_app() {
    #[cfg(windows)]
    {
        use std::sync::OnceLock;
        static JOB: OnceLock<win32job::Job> = OnceLock::new();
        let setup = || -> Result<win32job::Job, win32job::JobError> {
            let job = win32job::Job::create()?;
            let mut info = job.query_extended_limit_info()?;
            info.limit_kill_on_job_close();
            job.set_extended_limit_info(&info)?;
            job.assign_current_process()?;
            Ok(job)
        };
        match setup() {
            Ok(job) => {
                let _ = JOB.set(job);
            }
            Err(e) => eprintln!("contextual: couldn't create the process job: {e}"),
        }
    }
}
