//! App models (GPUI entities, plan §6.4). `AppModel` owns everything above the open project:
//! the screen, settings, recents, the Foundry connection and the workspace host.

pub mod chat;
pub mod hosts;
pub mod project;
pub mod scm;

use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::llm::FoundryClient;
use contextual_core::llm::config::{ConfigProblems, FoundryConfig};
use contextual_core::location::{Location, ProjectRoot};
use contextual_core::logic::foundry_status::{FoundryStatus, ReadyHealth};
use contextual_core::logic::types::Deployment;
use contextual_core::recents::{AppState, RecentProject, StateStore};
use contextual_core::settings::{
    API_KEY_SECRET, SecretStore, Settings, SettingsStore, resolve_foundry_config_from_env,
};
use contextual_protocol::fs::Scan;
use gpui_kit::{AppContext, Context, Entity, EventEmitter};
use tokio_util::sync::CancellationToken;

use crate::runtime;
use crate::secrets::AppSecrets;
pub use project::{Project, Tab};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Workspace,
    Settings,
}

/// Settings were saved explicitly: reveal the chat without replacing its draft or thread.
pub struct ReturnToChat;

/// Result of the last "Test connection".
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionTest {
    Idle,
    Running,
    Ok { latency_ms: u64 },
    Failed(String),
}

pub struct AppModel {
    pub screen: Screen,
    /// Where Settings returns to.
    back_to: Screen,
    pub settings: Settings,
    settings_store: Option<SettingsStore>,
    pub state: AppState,
    state_store: Option<StateStore>,
    pub secrets: Arc<AppSecrets>,
    pub foundry: FoundryStatus,
    pub config: Option<FoundryConfig>,
    pub connection_test: ConnectionTest,
    hosts: hosts::Hosts,
    /// Host of the open project (local, or the agent in its WSL distro).
    project_host: Option<Arc<dyn WorkspaceHost>>,
    pub project: Option<Entity<Project>>,
    /// Source Control of the open project.
    pub scm: Option<Entity<scm::ScmModel>>,
    pub scanning: bool,
    /// Last error to show (opening a folder, saving settings…).
    pub error: Option<String>,
    pub folder_dialog_open: bool,
}

impl EventEmitter<ReturnToChat> for AppModel {}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

impl AppModel {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let settings_store = SettingsStore::open_default().ok();
        let (settings, error) = match settings_store.as_ref().map(|s| s.load()) {
            Some(Ok(s)) => (s, None),
            Some(Err(e)) => (Settings::default(), Some(e.message)),
            None => (Settings::default(), None),
        };
        let state_store = StateStore::open_default().ok();
        let state = state_store.as_ref().map(|s| s.load()).unwrap_or_default();
        // Read once: turning command proposals on or off applies from the next start.
        let propose_commands = settings.chat.propose_commands;
        let mut this = Self {
            screen: Screen::Welcome,
            back_to: Screen::Welcome,
            settings,
            settings_store,
            state,
            state_store,
            secrets: Arc::new(AppSecrets::new()),
            foundry: FoundryStatus::Checking,
            config: None,
            connection_test: ConnectionTest::Idle,
            hosts: hosts::Hosts::new(propose_commands),
            project_host: None,
            project: None,
            scm: None,
            scanning: false,
            error,
            folder_dialog_open: false,
        };
        this.refresh_foundry();
        this
    }

    /// Host of the open project, or the local one.
    pub fn host(&mut self) -> Arc<dyn WorkspaceHost> {
        match &self.project_host {
            Some(h) => h.clone(),
            None => self.hosts.get(&Location::Local),
        }
    }

    /// Whether approved commands can run for a project at this location (the user's
    /// setting as it was at startup).
    pub fn commands_enabled(&self, location: &Location) -> bool {
        self.hosts.commands_enabled(location)
    }

    /// Host for a location (the folder dialog browses where the user points it).
    pub fn host_for(&mut self, location: &Location) -> Arc<dyn WorkspaceHost> {
        self.hosts.get(location)
    }

    pub fn detect_location(&self, input: &str) -> Result<ProjectRoot, String> {
        self.hosts.detect(input)
    }

    /// WSL distros to offer under Places (Windows; the dev distro in dev-remote mode).
    pub fn distros(&self) -> &[contextual_core::wsl::Distro] {
        &self.hosts.distros
    }

    /// Re-resolves the Foundry config from settings, env vars and the keychain.
    fn refresh_foundry(&mut self) {
        match resolve_foundry_config_from_env(&self.settings, self.secrets.as_ref()) {
            Ok(mut cfg) => {
                // Every client gets the same effective limit as the context bar, including
                // custom model entries. This only changes the runtime config, not Settings.
                cfg.context_limit = Some(self.settings.deployment(Some(&cfg)).limit);
                self.foundry = FoundryStatus::Ready {
                    health: ReadyHealth {
                        endpoint: cfg.endpoint.clone(),
                        deployment: cfg.deployment.clone(),
                    },
                };
                self.config = Some(cfg);
            }
            Err(ConfigProblems { missing, invalid }) => {
                self.foundry = FoundryStatus::Unconfigured { missing, invalid };
                self.config = None;
            }
        }
    }

    pub fn deployment(&self) -> Deployment {
        self.settings.deployment(self.config.as_ref())
    }

    pub fn has_api_key(&self) -> bool {
        std::env::var(contextual_core::llm::config::ENV_API_KEY).is_ok_and(|k| !k.trim().is_empty())
            || self.secrets.get(API_KEY_SECRET).ok().flatten().is_some()
    }

    fn save_state(&mut self) {
        if let Some(store) = &self.state_store
            && let Err(e) = store.save(&self.state)
        {
            self.error = Some(format!("Couldn't save recent projects: {}", e.message));
        }
    }

    /// Saves settings for inline controls, reporting failures through the app error.
    pub fn save_settings(
        &mut self,
        settings: Settings,
        api_key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.try_save_settings(settings, api_key, cx) {
            self.error = Some(error);
            cx.notify();
        }
    }

    /// Persists and applies settings. The form stays open if either write fails.
    pub fn try_save_settings(
        &mut self,
        settings: Settings,
        api_key: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let store = self.settings_store.as_ref().ok_or_else(|| {
            "Couldn't save settings: no settings directory is available.".to_string()
        })?;
        store
            .save(&settings)
            .map_err(|e| format!("Couldn't save settings: {}", e.message))?;
        if let Some(key) = api_key {
            let result = if key.trim().is_empty() {
                self.secrets.delete(API_KEY_SECRET)
            } else {
                self.secrets.set(API_KEY_SECRET, key.trim())
            };
            result.map_err(|e| format!("Couldn't store the API key: {}", e.message))?;
        }
        let rescan = settings.workspace != self.settings.workspace;
        self.settings = settings;
        self.refresh_foundry();
        self.connection_test = ConnectionTest::Idle;
        if rescan {
            self.rescan(cx);
        }
        cx.notify();
        Ok(())
    }

    /// Explicit Save returns to Chat; ordinary inline settings changes do not navigate.
    pub fn finish_settings(&mut self, cx: &mut Context<Self>) {
        self.screen = if self.project.is_some() {
            cx.emit(ReturnToChat);
            Screen::Workspace
        } else {
            Screen::Welcome
        };
        cx.notify();
    }

    pub fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.screen != Screen::Settings {
            self.back_to = self.screen;
            self.screen = Screen::Settings;
            cx.notify();
        }
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.screen = if self.back_to == Screen::Workspace && self.project.is_none() {
            Screen::Welcome
        } else {
            self.back_to
        };
        cx.notify();
    }

    pub fn close_project(&mut self, cx: &mut Context<Self>) {
        self.project = None;
        self.scm = None;
        self.project_host = None;
        self.screen = Screen::Welcome;
        self.state.last_open = None;
        self.save_state();
        cx.notify();
    }

    pub fn clear_recents(&mut self, cx: &mut Context<Self>) {
        self.state.recents.clear();
        self.save_state();
        cx.notify();
    }

    /// Scans `path` and opens it as the project. Without a location, it's detected from the path
    /// (`\\wsl.localhost\Ubuntu\…` → WSL, `C:\…` → local…).
    pub fn open_folder(
        &mut self,
        path: String,
        location: Option<Location>,
        cx: &mut Context<Self>,
    ) {
        let root = match location {
            Some(location) => ProjectRoot { location, path },
            None => match self.hosts.detect(&path) {
                Ok(r) => r,
                Err(e) => {
                    self.error = Some(e);
                    cx.notify();
                    return;
                }
            },
        };
        let host = self.hosts.get(&root.location);
        let path = root.path.clone();
        let location = root.location.clone();
        let location_for_key = root.location.clone();
        let options = self.settings.workspace.scan_options();
        self.scanning = true;
        self.error = None;
        self.folder_dialog_open = false;
        cx.notify();
        let project_host = host.clone();
        let legacy = self.hosts.legacy_threads(&location);
        let job = runtime::spawn(cx, async move {
            host.allow_root(&path).await?;
            let scan = host
                .scan(Scan {
                    root: path,
                    options,
                })
                .await?;
            // Bring this project's chats over from the old app, once (best effort).
            let key = ProjectRoot {
                location: location_for_key,
                path: scan.root.clone(),
            }
            .key();
            if let (Some(legacy), Ok(store)) = (
                legacy,
                contextual_core::threads::ThreadStore::open_default(),
            ) && let Some((dir, root)) = legacy.resolve(&scan.root).await
                && let Ok(n) = store.import_legacy(&dir, &root, &key).await
                && n > 0
            {
                tracing::info!("imported {n} saved chat files from the old app");
            }
            Ok(scan)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.scanning = false;
                match result {
                    Ok(scan) => {
                        this.state.opened(RecentProject {
                            name: scan.name.clone(),
                            path: scan.root.clone(),
                            location: location.clone(),
                            opened_at: now_rfc3339(),
                        });
                        this.save_state();
                        let project = cx.new(|_| Project::from_scan(scan, location));
                        this.project_host = Some(project_host.clone());
                        let host = project_host;
                        let scm = cx.new(|cx| scm::ScmModel::new(project.clone(), host, cx));
                        // The status bar shows the branch: re-render when it changes.
                        cx.observe(&scm, |_, _, cx| cx.notify()).detach();
                        this.project = Some(project);
                        this.scm = Some(scm);
                        this.screen = Screen::Workspace;
                    }
                    Err(e) => this.error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Rescans the open project with the current settings.
    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let root = project.read(cx).root.clone();
        let host = self.host();
        let options = self.settings.workspace.scan_options();
        self.scanning = true;
        cx.notify();
        let job = runtime::spawn(cx, async move { host.scan(Scan { root, options }).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.scanning = false;
                match result {
                    Ok(scan) => project.update(cx, |p, cx| {
                        p.apply_rescan(scan);
                        cx.notify()
                    }),
                    Err(e) => this.error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A 16-token round trip with the current (saved) configuration.
    pub fn test_connection(&mut self, cx: &mut Context<Self>) {
        let Some(cfg) = self.config.clone() else {
            self.connection_test = ConnectionTest::Failed(match &self.foundry {
                FoundryStatus::Unconfigured { missing, invalid } => ConfigProblems {
                    missing: missing.clone(),
                    invalid: invalid.clone(),
                }
                .message(),
                _ => "Foundry is not configured.".into(),
            });
            cx.notify();
            return;
        };
        self.connection_test = ConnectionTest::Running;
        cx.notify();
        let job = runtime::spawn(cx, async move {
            let client = FoundryClient::new(cfg)?;
            client.test_connection(&CancellationToken::new()).await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.connection_test = match result {
                    Ok(ms) => ConnectionTest::Ok { latency_ms: ms },
                    Err(e) => ConnectionTest::Failed(e.message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        cx.notify();
    }

    /// Stay on Welcome unless a project was explicitly requested on the command line.
    /// Recent projects remain available there, but are never reopened automatically.
    pub fn startup(&mut self, cli_path: Option<String>, cx: &mut Context<Self>) {
        if let Some(path) = cli_path {
            self.open_folder(path, None, cx);
        }
    }
}
