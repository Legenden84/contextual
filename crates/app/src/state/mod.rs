//! App models (GPUI entities, plan §6.4). `AppModel` owns everything above the open project:
//! the screen, settings, recents, the Foundry connection and the workspace host.

pub mod project;

use std::sync::Arc;

use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::llm::FoundryClient;
use contextual_core::llm::config::{ConfigProblems, FoundryConfig};
use contextual_core::location::Location;
use contextual_core::logic::foundry_status::{FoundryStatus, ReadyHealth};
use contextual_core::logic::types::Deployment;
use contextual_core::recents::{AppState, RecentProject, StateStore};
use contextual_core::settings::{
    API_KEY_SECRET, SecretStore, Settings, SettingsStore, resolve_foundry_config_from_env,
};
use contextual_protocol::fs::Scan;
use gpui_kit::{AppContext, Context, Entity, Window};
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
    host: Arc<dyn WorkspaceHost>,
    pub project: Option<Entity<Project>>,
    pub scanning: bool,
    /// Last error to show (opening a folder, saving settings…).
    pub error: Option<String>,
    pub folder_dialog_open: bool,
}

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
            host: Arc::new(LocalHost::new()),
            project: None,
            scanning: false,
            error,
            folder_dialog_open: false,
        };
        this.refresh_foundry();
        this
    }

    pub fn host(&self) -> Arc<dyn WorkspaceHost> {
        self.host.clone()
    }

    /// Re-resolves the Foundry config from settings, env vars and the keychain.
    fn refresh_foundry(&mut self) {
        match resolve_foundry_config_from_env(&self.settings, self.secrets.as_ref()) {
            Ok(cfg) => {
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
        Settings::deployment(self.config.as_ref())
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

    /// Saves settings (and the API key, when given) and applies them.
    pub fn save_settings(
        &mut self,
        settings: Settings,
        api_key: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let rescan = settings.workspace != self.settings.workspace;
        self.settings = settings;
        if let Some(store) = &self.settings_store
            && let Err(e) = store.save(&self.settings)
        {
            self.error = Some(format!("Couldn't save settings: {}", e.message));
        }
        if let Some(key) = api_key {
            let result = if key.trim().is_empty() {
                self.secrets.delete(API_KEY_SECRET)
            } else {
                self.secrets.set(API_KEY_SECRET, key.trim())
            };
            if let Err(e) = result {
                self.error = Some(format!("Couldn't store the API key: {}", e.message));
            }
        }
        self.refresh_foundry();
        self.connection_test = ConnectionTest::Idle;
        if rescan {
            self.rescan(cx);
        }
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

    /// Scans `path` and opens it as the project.
    pub fn open_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let options = self.settings.workspace.scan_options();
        self.scanning = true;
        self.error = None;
        self.folder_dialog_open = false;
        cx.notify();
        let job = runtime::spawn(cx, async move {
            host.allow_root(&path).await?;
            host.scan(Scan {
                root: path,
                options,
            })
            .await
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
                            location: Location::Local,
                            opened_at: now_rfc3339(),
                        });
                        this.save_state();
                        this.project = Some(cx.new(|_| Project::from_scan(scan)));
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
        let host = self.host.clone();
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

    /// The project to reopen on start: a CLI argument wins over the last open one.
    pub fn startup(
        &mut self,
        cli_path: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = cli_path.or_else(|| self.state.last_open.as_ref().map(|r| r.path.clone()));
        if let Some(path) = path {
            self.open_folder(path, cx);
        }
    }
}
