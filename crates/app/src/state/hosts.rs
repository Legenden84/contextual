//! Workspace hosts by location (plan §6.2): the in-process host for local projects and one
//! supervised agent per WSL distro, started on first use.
//!
//! `CONTEXTUAL_DEV_REMOTE=1` (debug builds) treats every project as living in a "dev" distro
//! served by a locally spawned `contextual-agent`, so the remote path can be exercised on Linux.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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
}

/// The directory of the running executable (bundled agents live next to it).
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default()
}

impl Hosts {
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
        Self {
            local: Arc::new(LocalHost::new()),
            remotes: HashMap::new(),
            distros,
            dev_remote,
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
            return Arc::new(LocalAgentLauncher {
                binary: exe_dir().join("contextual-agent"),
                app_version,
                env: Vec::new(),
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
