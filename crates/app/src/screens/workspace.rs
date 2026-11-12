//! Workspace (design spec: ScreenWorkspace): sidebar and persistent tabs for Chat, file previews,
//! Diff and Inspect. Feature tabs use the centre and right columns beneath the shared tab bar;
//! Chat and previews keep a stable column beside responsive Changes and history docks.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use contextual_theme::layout;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, canvas, div,
};

use crate::state::chat::{ChatModel, Toast};
use crate::state::scm::{OpenPullRequest, ScmModel};
use crate::state::{AppModel, Project, ReturnToChat, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, IconButton, Tag, file_glyph};
use crate::views::branch_picker::BranchPicker;
use crate::views::changes_panel::{ChangesPanel, OpenDiffFile};
use crate::views::chat_panel::ChatPanel;
use crate::views::chats_menu::ChatsMenu;
use crate::views::code_preview::CodePreview;
use crate::views::context_bar::{ContextCache, context_bar};
use crate::views::diff_cache::DiffStore;
use crate::views::diff_screen::DiffScreen;
use crate::views::git_diff::GitDiffView;
use crate::views::inspect_screen::InspectScreen;
use crate::views::pr_dialog::PrDialog;
use crate::views::save_dialog::SaveDialog;
use crate::views::sidebar::Sidebar;
use crate::views::toast::toast;
use contextual_core::logic::git_tabs::parse_git_tab;

/// Centre tab bar height.
const TABS_H: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DockPanel {
    Changes,
    History,
}

/// Only chat state rendered or mounted by the workspace itself. Child views observe
/// their own data; streamed text must not invalidate the surrounding chrome.
/// Proposal identities are immutable, so this never copies or compares file contents.
#[derive(Clone, Debug, PartialEq)]
struct WorkspaceChatState {
    changes: Option<(u64, String)>,
    save_dialog_open: bool,
    pr_open: bool,
    toast: Option<Toast>,
}

impl WorkspaceChatState {
    fn read(chat: &ChatModel) -> Self {
        Self {
            changes: chat
                .change_message()
                .map(|m| (chat.thread_generation, m.id.clone())),
            save_dialog_open: chat.save_dialog_open,
            pr_open: chat.pr_open,
            toast: chat.toast.clone(),
        }
    }
}

/// Visibility and stacking are independent of the window width. Resizing must
/// never close a panel or change which one was opened most recently.
struct DockState {
    changes_key: Option<(u64, String)>,
    changes_open: bool,
    history_open: bool,
    front: DockPanel,
}

impl DockState {
    fn new() -> Self {
        Self {
            changes_key: None,
            changes_open: false,
            history_open: true,
            front: DockPanel::History,
        }
    }

    fn sync_changes(&mut self, key: Option<(u64, String)>) {
        if self.changes_key == key {
            return;
        }
        self.changes_open = key.is_some();
        self.changes_key = key;
        if self.changes_open {
            self.front = DockPanel::Changes;
        }
    }

    fn sync_history(&mut self, open: bool) {
        if open && !self.history_open {
            self.front = DockPanel::History;
        }
        self.history_open = open;
    }

    fn toggle_changes(&mut self, visible: bool) {
        // A covered panel is raised, not closed.
        self.changes_open = !visible;
        if self.changes_open {
            self.front = DockPanel::Changes;
        }
    }
}

struct ChatLayout {
    left_inset: f32,
    width: f32,
    panel_width: f32,
    show_changes: bool,
    show_history: bool,
}

fn compact_workspace(available: f32) -> bool {
    available < layout::CHAT_MIN_WIDTH + layout::CHANGES_WIDTH
}

fn docked_explorer(viewport: f32) -> bool {
    viewport >= layout::EXPLORER_WIDTH + layout::CHAT_MIN_WIDTH
}

/// Centre the reading column in the area left after the visible docks.
/// Two docks share the workspace when both fit alongside a full reading column;
/// otherwise they share one slot. Compact layouts keep panels on demand.
fn chat_layout(
    available: f32,
    changes_open: bool,
    history_open: bool,
    front: DockPanel,
) -> ChatLayout {
    let available = available.max(0.0);
    let max_width = layout::CONTENT_MAX_WIDTH + 2.0 * contextual_theme::space::S4;
    let compact = compact_workspace(available);
    let panel_width = layout::CHANGES_WIDTH.min(available);
    let fits_both = available >= max_width + 2.0 * layout::CHANGES_WIDTH;
    let show_changes =
        !compact && changes_open && (!history_open || fits_both || front == DockPanel::Changes);
    let show_history =
        !compact && history_open && (!changes_open || fits_both || front == DockPanel::History);
    let reserved = (usize::from(show_changes) + usize::from(show_history)) as f32 * panel_width;
    let remaining = (available - reserved).max(0.0);
    let width = max_width.min(remaining);

    ChatLayout {
        left_inset: (remaining - width) / 2.0,
        width,
        panel_width,
        show_changes,
        show_history,
    }
}

const CHAT_REFLOW_DURATION: Duration = Duration::from_millis(180);

/// Animate layout, not text scale: the thread, budget and composer remain one
/// reading column. The total dock reservation also eases when a second dock opens.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ChatGeometry {
    left_inset: f32,
    width: f32,
    dock_width: f32,
}

impl ChatGeometry {
    fn from_layout(panes: &ChatLayout) -> Self {
        let count = usize::from(panes.show_changes) + usize::from(panes.show_history);
        Self {
            left_inset: panes.left_inset,
            width: panes.width,
            dock_width: count as f32 * panes.panel_width,
        }
    }

    /// Include both reading margins in Chat's native scroll surface, but not the docks.
    fn chat_surface_width(self) -> f32 {
        self.width + 2.0 * self.left_inset
    }

    fn interpolate(self, target: Self, progress: f32) -> Self {
        let lerp = |from: f32, to: f32| from + (to - from) * progress;
        Self {
            left_inset: lerp(self.left_inset, target.left_inset),
            width: lerp(self.width, target.width),
            dock_width: lerp(self.dock_width, target.dock_width),
        }
    }
}

struct ChatReflow {
    from: ChatGeometry,
    target: ChatGeometry,
    started: Instant,
}

impl ChatReflow {
    fn settled(target: ChatGeometry, now: Instant) -> Self {
        Self {
            from: target,
            target,
            started: now,
        }
    }

    fn running(&self, now: Instant) -> bool {
        self.from != self.target
            && now.saturating_duration_since(self.started) < CHAT_REFLOW_DURATION
    }

    fn sample(&self, now: Instant) -> ChatGeometry {
        if !self.running(now) {
            return self.target;
        }
        let progress = (now.saturating_duration_since(self.started).as_secs_f32()
            / CHAT_REFLOW_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        // Ease out without overshoot. No bouncing or scaling of code/text.
        let eased = 1.0 - (1.0 - progress).powi(3);
        self.from.interpolate(self.target, eased)
    }

    fn retarget(&mut self, target: ChatGeometry, now: Instant) {
        if self.target == target {
            return;
        }
        // A quick second toggle starts from what is currently on screen, not
        // from the previous endpoint.
        self.from = self.sample(now);
        self.target = target;
        self.started = now;
    }
}

pub struct WorkspaceScreen {
    app: Entity<AppModel>,
    project: Entity<Project>,
    sidebar: Entity<Sidebar>,
    previews: HashMap<String, AnyView>,
    scm: Entity<ScmModel>,
    branch_picker: Option<Entity<BranchPicker>>,
    pr_dialog: Option<Entity<PrDialog>>,
    chat: Entity<ChatModel>,
    chat_panel: Entity<ChatPanel>,
    changes: Entity<ChangesPanel>,
    diff: Entity<DiffScreen>,
    inspect: Entity<InspectScreen>,
    chats: Entity<ChatsMenu>,
    docks: DockState,
    /// Compact navigation is explicit: default-open history must not replace Chat
    /// merely because the window was resized.
    compact_panel: Option<DockPanel>,
    compact_project: bool,
    /// Actual allocated width, excluding any surrounding native window chrome.
    workspace_width: Option<Pixels>,
    chat_reflow: Option<ChatReflow>,
    reflow_available: Option<f32>,
    reflow_frame_pending: bool,
    save_dialog: Option<Entity<SaveDialog>>,
    context: ContextCache,
    _subs: Vec<Subscription>,
}

impl WorkspaceScreen {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subs = vec![
            cx.observe_window_activation(window, |this: &mut Self, window, cx| {
                if window.is_window_active() {
                    // Git may have changed in another shell while the app was inactive.
                    this.scm.update(cx, |scm, cx| scm.refresh(cx));
                }
            }),
            cx.observe_in(&project, window, |this: &mut Self, project, window, cx| {
                // Feature tabs have their own persistent entities, not file previews.
                let tabs = project.read(cx).tabs.clone();
                this.previews
                    .retain(|p, _| tabs.contains(&Tab::Preview(p.clone())));
                for tab in tabs {
                    let Tab::Preview(t) = tab else {
                        continue;
                    };
                    if !this.previews.contains_key(&t) {
                        let host = this.app.update(cx, |a, _| a.host());
                        let view: AnyView = match parse_git_tab(&t) {
                            Some(g) => {
                                let scm = this.scm.clone();
                                cx.new(|cx| {
                                    GitDiffView::new(
                                        g.path,
                                        g.staged,
                                        project.clone(),
                                        scm,
                                        host,
                                        cx,
                                    )
                                })
                                .into()
                            }
                            None => cx
                                .new(|cx| {
                                    CodePreview::new(t.clone(), project.clone(), host, window, cx)
                                })
                                .into(),
                        };
                        this.previews.insert(t, view);
                    }
                }
                cx.notify();
            }),
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe_in(&app, window, |this, _, _: &ReturnToChat, window, cx| {
                this.compact_panel = None;
                this.compact_project = false;
                this.project.update(cx, |p, cx| {
                    p.activate(Tab::Chat);
                    cx.notify();
                });
                this.chat_panel
                    .update(cx, |panel, cx| panel.focus_input(window, cx));
                cx.notify();
            }),
        ];
        let host = app.update(cx, |a, _| a.host());
        let scm = app.read(cx).scm.clone().unwrap_or_else(|| {
            let (project, host) = (project.clone(), host.clone());
            cx.new(|cx| ScmModel::new(project, host, cx))
        });
        let commands = {
            let location = project.read(cx).location.clone();
            app.read(cx).commands_enabled(&location)
        };
        let chat = cx.new(|cx| {
            let mut chat = ChatModel::new(project.clone(), host, cx);
            chat.commands_enabled = commands;
            // A command may change any file: afterwards the project is scanned again,
            // which also refreshes Source Control.
            let rescan_app = app.clone();
            chat.on_files_changed(move |cx| rescan_app.update(cx, |a, cx| a.rescan(cx)));
            chat
        });
        let diffs = cx.new(|_| DiffStore::default());
        let mut subs = subs;
        let mut chat_state = WorkspaceChatState::read(chat.read(cx));
        let mut budget_state = {
            let model = chat.read(cx);
            (
                model.context_revision,
                model.live_context.map(|live| live.tokens()),
                model.attachments.iter().map(|a| a.tokens).sum::<u64>(),
            )
        };
        subs.push(
            cx.observe_in(&chat, window, move |this: &mut Self, chat, window, cx| {
                // Finish reading the entity before notifying or otherwise mutating cx.
                let (next_budget, next) = {
                    let model = chat.read(cx);
                    (
                        (
                            model.context_revision,
                            model.live_context.map(|live| live.tokens()),
                            model.attachments.iter().map(|a| a.tokens).sum::<u64>(),
                        ),
                        WorkspaceChatState::read(model),
                    )
                };
                if next_budget != budget_state {
                    budget_state = next_budget;
                    // Chat owns its own gauge. The preview gauge is rendered here, so it
                    // must observe accounting updates even when no proposals/dialogs change.
                    if matches!(this.project.read(cx).active, Tab::Preview(_)) {
                        cx.notify();
                    }
                }
                if next == chat_state {
                    return;
                }
                chat_state = next;
                this.docks.sync_changes(chat_state.changes.clone());
                // The Save and pull request dialogs live while the model says they're open.
                let open = chat_state.save_dialog_open;
                if open && this.save_dialog.is_none() {
                    let (app, chat) = (this.app.clone(), chat.clone());
                    this.save_dialog = Some(cx.new(|cx| SaveDialog::new(app, chat, window, cx)));
                } else if !open {
                    this.save_dialog = None;
                }
                let pr = chat_state.pr_open;
                if pr && this.pr_dialog.is_none() {
                    this.sidebar.update(cx, |sidebar, cx| sidebar.show_scm(cx));
                    let root = this.scm.read(cx).repository_root().map(str::to_owned);
                    if let Some(root) = root {
                        let (app, chat) = (this.app.clone(), chat.clone());
                        this.pr_dialog =
                            Some(cx.new(|cx| PrDialog::new(app, root, chat, window, cx)));
                    }
                } else if !pr {
                    this.pr_dialog = None;
                }
                cx.notify();
            }),
        );
        subs.push(
            cx.observe_in(&scm, window, |this: &mut Self, scm, window, cx| {
                let open = scm.read(cx).branch_picker_open;
                if open && this.branch_picker.is_none() {
                    let (app, scm) = (this.app.clone(), scm.clone());
                    this.branch_picker = Some(cx.new(|cx| BranchPicker::new(app, scm, window, cx)));
                } else if !open {
                    this.branch_picker = None;
                }
                cx.notify();
            }),
        );
        subs.push(
            cx.subscribe(&scm, |this: &mut Self, _, _: &OpenPullRequest, cx| {
                this.chat.update(cx, |chat, cx| {
                    chat.pr_open = true;
                    cx.notify();
                });
            }),
        );
        let chat_panel = cx.new(|cx| {
            ChatPanel::new(
                app.clone(),
                project.clone(),
                chat.clone(),
                diffs.clone(),
                window,
                cx,
            )
        });
        // The settings warning is owned by the composer but overlays the whole workspace.
        // Typing, attachments and streamed text do not change that overlay's visibility.
        let mut settings_prompt_open = chat_panel.read(cx).settings_prompt_open();
        subs.push(cx.observe(&chat_panel, move |_, panel, cx| {
            let open = panel.read(cx).settings_prompt_open();
            if open != settings_prompt_open {
                settings_prompt_open = open;
                cx.notify();
            }
        }));
        let chats = cx.new(|cx| ChatsMenu::new(chat.clone(), window, cx));
        // History owns search, rename and row updates. The workspace only needs to
        // know whether its dock opened or closed.
        let mut history_open = chats.read(cx).open;
        subs.push(cx.observe(&chats, move |this: &mut Self, chats, cx| {
            let open = chats.read(cx).open;
            if open != history_open {
                history_open = open;
                this.docks.sync_history(open);
                cx.notify();
            }
        }));
        crate::dev::install(&app, &project, &chat, &chat_panel, &chats, window, cx);
        let sidebar =
            cx.new(|cx| Sidebar::new(app.clone(), project.clone(), scm.clone(), window, cx));
        crate::dev::open(&app, &sidebar, &project, &scm, &chat, cx);
        let changes = cx.new(|cx| ChangesPanel::new(chat.clone(), diffs.clone(), cx));
        let diff = cx.new(|cx| DiffScreen::new(chat.clone(), diffs, cx));
        subs.push(
            cx.subscribe(&changes, |this: &mut Self, _, event: &OpenDiffFile, cx| {
                this.diff
                    .update(cx, |diff, cx| diff.open_file(&event.path, cx));
            }),
        );
        Self {
            sidebar,
            scm,
            branch_picker: None,
            pr_dialog: None,
            chat_panel,
            changes,
            diff,
            inspect: cx.new(|cx| InspectScreen::new(chat.clone(), window, cx)),
            chats,
            docks: DockState::new(),
            compact_panel: None,
            compact_project: false,
            workspace_width: None,
            chat_reflow: None,
            reflow_available: None,
            reflow_frame_pending: false,
            save_dialog: None,
            chat,
            app,
            project,
            previews: HashMap::new(),
            context: ContextCache::default(),
            _subs: subs,
        }
    }

    fn chat_geometry(
        &mut self,
        panes: &ChatLayout,
        available: f32,
        animate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ChatGeometry {
        let now = Instant::now();
        let target = ChatGeometry::from_layout(panes);
        // Native resizing follows the window immediately. Animate dock changes,
        // not startup, zoom, compact navigation or entry into a feature tab.
        let snap = !animate || self.reflow_available != Some(available);
        self.reflow_available = Some(available);
        if snap || self.chat_reflow.is_none() {
            self.chat_reflow = Some(ChatReflow::settled(target, now));
        }
        let reflow = self.chat_reflow.as_mut().expect("reflow initialised");
        reflow.retarget(target, now);
        let geometry = reflow.sample(now);
        if reflow.running(now) && !self.reflow_frame_pending {
            self.reflow_frame_pending = true;
            let entity = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                entity
                    .update(cx, |this, cx| {
                        this.reflow_frame_pending = false;
                        cx.notify();
                    })
                    .ok();
            });
        }
        geometry
    }

    fn tab(
        &self,
        id: SharedString,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let c = &cx.tokens().colors;
        div()
            .id(id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(sp(2))
            .whitespace_nowrap()
            .px(sp(3))
            .h_full()
            .border_r_1()
            .border_color(hsla(c.border))
            .cursor_pointer()
            .text_color(hsla(if active { c.ink_strong } else { c.ink_muted }))
            .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
            .when(active, |d| {
                d.bg(hsla(c.surface_100)).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(hsla(c.accent)),
                )
            })
    }
}

impl Render for WorkspaceScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let p = self.project.read(cx);
        let active = p.active.clone();
        let tabs = p.tabs.clone();

        let chat_tab = self
            .tab("tab-chat".into(), active == Tab::Chat, cx)
            .on_click(cx.listener(|t, _, window, cx| {
                t.compact_panel = None;
                t.compact_project = false;
                t.project.update(cx, |p, cx| {
                    p.activate(Tab::Chat);
                    cx.notify()
                });
                t.chat_panel
                    .update(cx, |panel, cx| panel.focus_input(window, cx));
            }))
            .child(
                div()
                    .text_color(hsla(c.accent_ink))
                    .child(Icon::new(IconName::Bot).size(px(14.0))),
            )
            .child("Chat");
        let open_tabs: Vec<_> = tabs
            .iter()
            .map(|tab| {
                let (id, name, tag, glyph) = match tab {
                    Tab::Preview(path) => {
                        let git = parse_git_tab(path);
                        let shown = git.as_ref().map(|g| g.path.as_str()).unwrap_or(path);
                        let name = shown.rsplit('/').next().unwrap_or(shown).to_string();
                        let tag = match &git {
                            Some(g) if g.staged => "staged",
                            Some(_) => "changes",
                            None => "read-only",
                        };
                        let glyph = file_glyph(&name, cx).into_any_element();
                        (format!("preview:{path}"), name, Some(tag), glyph)
                    }
                    Tab::Diff => (
                        "diff".into(),
                        "Diff".into(),
                        None,
                        Icon::new(IconName::File).size(sp(4)).into_any_element(),
                    ),
                    Tab::Inspect => (
                        "inspect".into(),
                        "Inspect".into(),
                        None,
                        Icon::new(gpui_kit::assets::IconName::Brain)
                            .size(sp(4))
                            .into_any_element(),
                    ),
                    Tab::Chat => (
                        "chat".into(),
                        "Chat".into(),
                        None,
                        Icon::new(IconName::Bot).size(sp(4)).into_any_element(),
                    ),
                };
                let (activate, close) = (tab.clone(), tab.clone());
                let close_label = format!("Close {name}");
                self.tab(SharedString::from(format!("tab:{id}")), &active == tab, cx)
                    .on_click(cx.listener(move |t, _, _, cx| {
                        t.compact_panel = None;
                        t.compact_project = false;
                        t.project.update(cx, |p, cx| {
                            p.activate(activate.clone());
                            cx.notify()
                        })
                    }))
                    .child(glyph)
                    .child(SharedString::from(name))
                    .when_some(tag, |d, tag| d.child(Tag::new(tag)))
                    .child(
                        IconButton::new(
                            SharedString::from(format!("close:{id}")),
                            IconName::Close,
                            close_label,
                        )
                        .on_click(cx.listener(move |t, _, window, cx| {
                            cx.stop_propagation();
                            t.project.update(cx, |p, cx| {
                                p.close_tab(&close);
                                cx.notify()
                            });
                            if t.project.read(cx).active == Tab::Chat {
                                t.chat_panel
                                    .update(cx, |panel, cx| panel.focus_input(window, cx));
                            }
                        })),
                    )
            })
            .collect();

        let chat = self.chat.read(cx);
        let has_changes = chat.change_message().is_some();
        let feature_view = matches!(active, Tab::Diff | Tab::Inspect);
        let history_open = self.chats.read(cx).open && !feature_view;
        // Budget from the allocated workspace, not the native viewport: decorations
        // and ancestor layout can leave less room than the window reports.
        // Store logical pixels so zoom is applied exactly once, here and in theme::px.
        let viewport =
            self.workspace_width.map(f32::from).unwrap_or_default() / crate::theme::zoom_factor();
        let explorer_docked = docked_explorer(viewport);
        let available = (viewport
            - if explorer_docked {
                layout::EXPLORER_WIDTH
            } else {
                0.0
            })
        .max(0.0);
        let compact = compact_workspace(available);
        let project_visible = !explorer_docked && self.compact_project;
        let compact_changes = compact
            && !feature_view
            && !project_visible
            && has_changes
            && self.compact_panel == Some(DockPanel::Changes);
        let compact_history = compact
            && !feature_view
            && !project_visible
            && history_open
            && self.compact_panel == Some(DockPanel::History);
        let auxiliary_view = project_visible || compact_changes || compact_history;
        // ChatPanel centres its own messages and composer. Its outer scroll surface must
        // also own the empty reading margins so native wheel events reach the conversation.
        let full_width_chat = active == Tab::Chat && !auxiliary_view;
        let panes = chat_layout(
            available,
            has_changes && self.docks.changes_open,
            history_open,
            self.docks.front,
        );
        let changes_visible = !feature_view && (panes.show_changes || compact_changes);
        let history_visible = !feature_view && (panes.show_history || compact_history);
        let animate = !feature_view
            && !auxiliary_view
            && !compact
            && !self.app.read(cx).settings.reduce_motion;
        let geometry = self.chat_geometry(&panes, available, animate, window, cx);
        let dock_count = usize::from(panes.show_changes) + usize::from(panes.show_history);
        let animated_panel_width = geometry.dock_width / dock_count.max(1) as f32;

        let body = if project_visible {
            div()
                .id("compact-project")
                .flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_x_scroll()
                .child(self.sidebar.clone())
                .into_any_element()
        } else if compact_changes {
            self.changes.clone().into_any_element()
        } else if compact_history {
            self.chats.clone().into_any_element()
        } else {
            match &active {
                Tab::Preview(path) => {
                    let view = self
                        .previews
                        .get(path)
                        .map(|v| v.clone().into_any_element())
                        .unwrap_or_else(|| div().into_any_element());
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .child(div().flex().flex_col().flex_1().min_h_0().child(view))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .justify_center()
                                .px(sp(4))
                                .pb(sp(2))
                                .child(div().w_full().max_w(px(layout::CONTENT_MAX_WIDTH)).child(
                                    context_bar(
                                        &self.app,
                                        &self.project,
                                        Some(&self.chat),
                                        &self.context,
                                        cx,
                                    ),
                                )),
                        )
                        .into_any_element()
                }
                Tab::Chat => self.chat_panel.clone().into_any_element(),
                Tab::Diff => self.diff.clone().into_any_element(),
                Tab::Inspect => self.inspect.clone().into_any_element(),
            }
        };

        let tab_bar = div()
            .id("center-tabs")
            .flex()
            .flex_none()
            .min_w_0()
            .when(compact, |d| d.flex_col())
            .border_b_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .type_style(contextual_theme::type_style::UI_BODY)
            .child(
                div()
                    .id("tab-strip")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(TABS_H))
                    .when(compact, |d| d.flex_none().w_full())
                    .overflow_x_scroll()
                    .child(chat_tab)
                    .children(open_tabs),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(sp(1))
                    .px(sp(2))
                    .when(compact, |d| {
                        d.flex_wrap()
                            .py(sp(1))
                            .border_t_1()
                            .border_color(hsla(c.border))
                    })
                    .when(!explorer_docked, |d| {
                        d.child(
                            Button::new(
                                "toggle-project",
                                if project_visible {
                                    "Back to chat"
                                } else {
                                    "Project"
                                },
                            )
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .selected(project_visible)
                            .on_click(cx.listener(
                                |t, _, window, cx| {
                                    t.compact_project = !t.compact_project;
                                    t.compact_panel = None;
                                    if !t.compact_project {
                                        t.project.update(cx, |p, cx| {
                                            p.activate(Tab::Chat);
                                            cx.notify();
                                        });
                                        t.chat_panel.update(cx, |panel, cx| {
                                            panel.focus_input(window, cx);
                                        });
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .child(
                        Button::new(
                            "toggle-changes",
                            if changes_visible {
                                "Hide changes"
                            } else {
                                "Show changes"
                            },
                        )
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .selected(changes_visible)
                        .disabled(!has_changes)
                        .on_click(cx.listener(move |t, _, window, cx| {
                            t.compact_project = false;
                            if matches!(t.project.read(cx).active, Tab::Diff | Tab::Inspect) {
                                t.project.update(cx, |p, cx| {
                                    p.activate(Tab::Chat);
                                    cx.notify();
                                });
                            }
                            if compact {
                                t.compact_panel =
                                    (!changes_visible).then_some(DockPanel::Changes);
                                if changes_visible {
                                    t.chat_panel.update(cx, |panel, cx| {
                                        panel.focus_input(window, cx);
                                    });
                                }
                            } else {
                                t.docks.toggle_changes(changes_visible);
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(
                            "chats",
                            if history_visible {
                                "Hide history"
                            } else {
                                "Show history"
                            },
                        )
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .selected(history_visible)
                        .on_click(cx.listener(move |t, _, window, cx| {
                            t.compact_project = false;
                            if matches!(t.project.read(cx).active, Tab::Diff | Tab::Inspect) {
                                t.project.update(cx, |p, cx| {
                                    p.activate(Tab::Chat);
                                    cx.notify();
                                });
                            }
                            if compact {
                                t.compact_panel = (!history_visible).then_some(DockPanel::History);
                                if history_visible {
                                    t.chat_panel.update(cx, |panel, cx| {
                                        panel.focus_input(window, cx);
                                    });
                                } else {
                                    t.chats.update(cx, |m, cx| {
                                        m.open = true;
                                        cx.notify();
                                    });
                                }
                                cx.notify();
                                return;
                            }
                            if history_visible {
                                t.docks.sync_history(false);
                                t.chats.update(cx, |m, cx| m.toggle(cx));
                            } else {
                                // History may already be open underneath Changes.
                                t.docks.sync_history(true);
                                t.docks.front = DockPanel::History;
                                t.chats.update(cx, |m, cx| {
                                    m.open = true;
                                    cx.notify();
                                });
                            }
                            // Raising already-open history changes dock priority, not
                            // ChatsMenu::open, so the workspace must invalidate itself.
                            cx.notify();
                        })),
                    ),
            );

        let main = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(hsla(c.surface_100))
            .child(tab_bar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(!feature_view && !auxiliary_view && !full_width_chat, |d| {
                        d.child(div().flex_none().w(px(geometry.left_inset)))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .when(!feature_view && !auxiliary_view, |d| {
                                d.flex_none()
                                    .min_w(px(layout::CHAT_MIN_WIDTH.min(available)))
                                    .w(px(if full_width_chat {
                                        geometry.chat_surface_width()
                                    } else {
                                        geometry.width
                                    }))
                            })
                            .child(body),
                    )
                    .when(!feature_view && !auxiliary_view, |d| {
                        d.child(
                            div()
                                .id("workspace-right-dock")
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .h_full()
                                .justify_end()
                                .when(full_width_chat, |d| {
                                    d.flex_none().w(px(geometry.dock_width))
                                })
                                .when(panes.show_changes, |d| {
                                    d.child(
                                        div()
                                            .id("changes-dock-slot")
                                            .flex()
                                            .flex_none()
                                            .w(px(animated_panel_width))
                                            .h_full()
                                            .min_h_0()
                                            .overflow_hidden()
                                            .child(self.changes.clone()),
                                    )
                                })
                                .when(panes.show_history, |d| {
                                    d.child(
                                        div()
                                            .id("history-dock-slot")
                                            .flex()
                                            .flex_none()
                                            .w(px(animated_panel_width))
                                            .h_full()
                                            .min_h_0()
                                            .overflow_hidden()
                                            .child(self.chats.clone()),
                                    )
                                }),
                        )
                    }),
            );

        let measured_width = self.workspace_width;
        let workspace = cx.entity().downgrade();

        div()
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(
                canvas(
                    |bounds, _, _| bounds.size.width,
                    move |_, width, window, _| {
                        if measured_width == Some(width) {
                            return;
                        }
                        // Layout is complete; update on the next frame rather than
                        // mutating an entity during paint. Settled layouts schedule nothing.
                        window.on_next_frame(move |_, cx| {
                            workspace
                                .update(cx, |this, cx| {
                                    if this.workspace_width != Some(width) {
                                        this.workspace_width = Some(width);
                                        cx.notify();
                                    }
                                })
                                .ok();
                        });
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            // First measure the allocation without guessing at dock widths.
            .when(measured_width.is_some(), |d| {
                d.when(explorer_docked, |d| d.child(self.sidebar.clone()))
                    .child(main)
            })
            .when_some(toast(&self.app, &self.chat, cx), |d, t| d.child(t))
            .when_some(self.save_dialog.clone(), |d, s| d.child(s))
            .when_some(self.pr_dialog.clone(), |d, s| d.child(s))
            .when_some(self.branch_picker.clone(), |d, s| d.child(s))
            .when_some(
                self.chat_panel
                    .update(cx, |panel, cx| panel.settings_prompt(cx)),
                |d, prompt| d.child(prompt),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_chat_width() -> f32 {
        layout::CONTENT_MAX_WIDTH + 2.0 * contextual_theme::space::S4
    }

    #[test]
    fn chat_and_panels_stay_inside_the_workspace() {
        for available in [
            0.0,
            300.0,
            layout::CHAT_MIN_WIDTH + layout::CHANGES_WIDTH,
            full_chat_width(),
            full_chat_width() + layout::CHANGES_WIDTH,
            full_chat_width() + 2.0 * layout::CHANGES_WIDTH,
            full_chat_width() + 4.0 * layout::CHANGES_WIDTH,
        ] {
            for changes in [false, true] {
                for history in [false, true] {
                    for front in [DockPanel::Changes, DockPanel::History] {
                        let panes = chat_layout(available, changes, history, front);
                        let panel_count =
                            usize::from(panes.show_changes) + usize::from(panes.show_history);
                        assert!(panes.left_inset >= 0.0);
                        assert!(panes.width >= 0.0);
                        assert!(
                            panes.left_inset + panes.width + panel_count as f32 * panes.panel_width
                                <= available + 0.001
                        );
                        assert_eq!(
                            panel_count > 0,
                            !compact_workspace(available) && (changes || history)
                        );
                        let remaining = available - panel_count as f32 * panes.panel_width;
                        assert!(
                            (panes.left_inset + panes.width / 2.0 - remaining / 2.0).abs() < 0.001
                        );
                        if panel_count > 0 {
                            assert!(panes.width >= layout::CHAT_MIN_WIDTH);
                        } else {
                            assert_eq!(panes.width, available.min(full_chat_width()));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn two_panels_fit_beside_a_full_reading_column() {
        let threshold = full_chat_width() + 2.0 * layout::CHANGES_WIDTH;
        for front in [DockPanel::Changes, DockPanel::History] {
            let below = chat_layout(threshold - 1.0, true, true, front);
            assert_eq!(below.show_changes, front == DockPanel::Changes);
            assert_eq!(below.show_history, front == DockPanel::History);
            assert_eq!(below.width, full_chat_width());

            for available in [threshold, threshold + layout::CHANGES_WIDTH] {
                let panes = chat_layout(available, true, true, front);
                assert!(panes.show_changes);
                assert!(panes.show_history);
                assert_eq!(panes.width, full_chat_width());
            }
        }
    }

    #[test]
    fn narrow_and_zoomed_windows_prioritise_chat() {
        let threshold = layout::CHAT_MIN_WIDTH + layout::CHANGES_WIDTH;
        for scale in [1.0, 1.5, 2.0] {
            for available in [300.0, threshold - 1.0] {
                let measured = available * scale;
                let design_width = measured / scale;
                for front in [DockPanel::Changes, DockPanel::History] {
                    let panes = chat_layout(design_width, true, true, front);
                    assert_eq!(panes.width, available);
                    assert_eq!(panes.left_inset, 0.0);
                    assert!(!panes.show_changes);
                    assert!(!panes.show_history);
                }
            }
        }
        let panes = chat_layout(threshold, true, true, DockPanel::History);
        assert_eq!(panes.width, layout::CHAT_MIN_WIDTH);
        assert!(panes.show_history);

        let explorer_threshold = layout::EXPLORER_WIDTH + layout::CHAT_MIN_WIDTH;
        assert!(!docked_explorer(explorer_threshold - 1.0));
        assert!(docked_explorer(explorer_threshold));

        let panes = chat_layout(-10.0, true, true, DockPanel::History);
        assert_eq!(panes.width, 0.0);
        assert_eq!(panes.left_inset, 0.0);
        assert_eq!(panes.panel_width, 0.0);
    }

    #[test]
    fn toggling_panels_preserves_full_width_whenever_one_fits() {
        let threshold = full_chat_width() + 2.0 * layout::CHANGES_WIDTH;
        for available in [threshold, threshold + layout::CHANGES_WIDTH] {
            for changes in [false, true] {
                for history in [false, true] {
                    let panes = chat_layout(available, changes, history, DockPanel::History);
                    assert_eq!(panes.width, full_chat_width());
                    let count = usize::from(panes.show_changes) + usize::from(panes.show_history);
                    let remaining = available - count as f32 * panes.panel_width;
                    assert_eq!(panes.left_inset, (remaining - full_chat_width()) / 2.0);
                }
            }
        }
        let below = chat_layout(threshold - 2.0, true, true, DockPanel::Changes);
        assert_eq!(below.width, full_chat_width());
        assert_eq!(below.left_inset, (layout::CHANGES_WIDTH - 2.0) / 2.0);
    }

    #[test]
    fn medium_windows_spend_the_empty_gutter_before_chat_width() {
        let available = full_chat_width() + layout::CHANGES_WIDTH;
        let panes = chat_layout(available, true, true, DockPanel::History);
        assert_eq!(panes.width, full_chat_width());
        assert_eq!(panes.left_inset, 0.0);

        let narrower = chat_layout(available - 100.0, true, true, DockPanel::Changes);
        assert_eq!(narrower.width, full_chat_width() - 100.0);
        assert_eq!(narrower.left_inset, 0.0);
        assert!(narrower.show_changes);
        assert!(!narrower.show_history);
    }

    #[test]
    fn chat_scroll_surface_owns_both_margins_without_overlapping_docks() {
        let available = full_chat_width() + 3.0 * layout::CHANGES_WIDTH;
        let closed =
            ChatGeometry::from_layout(&chat_layout(available, false, false, DockPanel::History));
        let opened =
            ChatGeometry::from_layout(&chat_layout(available, true, true, DockPanel::History));

        for (from, target) in [(closed, opened), (opened, closed)] {
            for step in 0..=10 {
                let geometry = from.interpolate(target, step as f32 / 10.0);
                let surface = geometry.chat_surface_width();
                assert!(
                    (surface + geometry.dock_width - available).abs() < 0.001,
                    "Chat and docks must exactly share the available width: {geometry:?}"
                );
                assert!(
                    (surface / 2.0 - (geometry.left_inset + geometry.width / 2.0)).abs() < 0.001,
                    "Widening the scroll surface must not move the reading column"
                );
            }
        }
    }

    #[test]
    fn reflow_moves_the_column_and_lands_exactly_on_target() {
        let now = Instant::now();
        let available = full_chat_width() + 3.0 * layout::CHANGES_WIDTH;
        let closed =
            ChatGeometry::from_layout(&chat_layout(available, false, false, DockPanel::History));
        let opened =
            ChatGeometry::from_layout(&chat_layout(available, false, true, DockPanel::History));
        let mut reflow = ChatReflow::settled(closed, now);
        assert!(!reflow.running(now));
        reflow.retarget(opened, now);
        assert_eq!(reflow.sample(now), closed);

        let middle = reflow.sample(now + CHAT_REFLOW_DURATION / 2);
        assert!(middle.left_inset < closed.left_inset);
        assert!(middle.left_inset > opened.left_inset);
        assert_eq!(middle.width, full_chat_width());
        assert!(
            (middle.left_inset + middle.width / 2.0 - (available - middle.dock_width) / 2.0).abs()
                < 0.001
        );

        let end = now + CHAT_REFLOW_DURATION;
        assert_eq!(reflow.sample(end), opened);
        assert!(!reflow.running(end));
    }

    #[test]
    fn history_reflow_preserves_chat_minimum_and_never_crosses_its_column() {
        let now = Instant::now();
        for available in [
            layout::CHAT_MIN_WIDTH + layout::CHANGES_WIDTH,
            1280.0 - layout::EXPLORER_WIDTH,
        ] {
            let closed = ChatGeometry::from_layout(&chat_layout(
                available,
                false,
                false,
                DockPanel::History,
            ));
            let opened =
                ChatGeometry::from_layout(&chat_layout(available, false, true, DockPanel::History));

            for (from, target) in [(closed, opened), (opened, closed)] {
                let mut reflow = ChatReflow::settled(from, now);
                reflow.retarget(target, now);
                for step in 0..=18 {
                    let geometry = reflow.sample(now + Duration::from_millis(step * 10));
                    assert!(geometry.width >= layout::CHAT_MIN_WIDTH);
                    assert!(
                        geometry.left_inset + geometry.width
                            <= available - geometry.dock_width + 0.001,
                        "Chat must end before the reserved history area: {geometry:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn reflow_retargets_without_jumping_or_restarting_on_notifications() {
        let now = Instant::now();
        let available = full_chat_width() + 3.0 * layout::CHANGES_WIDTH;
        let closed =
            ChatGeometry::from_layout(&chat_layout(available, false, false, DockPanel::History));
        let both =
            ChatGeometry::from_layout(&chat_layout(available, true, true, DockPanel::History));
        let mut reflow = ChatReflow::settled(closed, now);
        reflow.retarget(both, now);
        let interrupted = now + CHAT_REFLOW_DURATION / 3;
        let current = reflow.sample(interrupted);
        reflow.retarget(both, interrupted);
        assert_eq!(reflow.started, now);
        reflow.retarget(closed, interrupted);
        assert_eq!(reflow.sample(interrupted), current);
        assert_eq!(reflow.sample(interrupted + CHAT_REFLOW_DURATION), closed);

        // Reduced motion / resizing uses the same settled path as startup.
        let snapped = ChatReflow::settled(both, interrupted);
        assert_eq!(snapped.sample(interrupted), both);
        assert!(!snapped.running(interrupted));
    }

    #[test]
    fn latest_opened_panel_wins_without_notifications_stealing_priority() {
        let mut docks = DockState::new();
        let first = Some((0, "reply-1".to_string()));
        docks.sync_changes(first.clone());
        assert_eq!(docks.front, DockPanel::Changes);
        assert!(docks.history_open);

        // History search and chat updates do not count as opening a panel.
        docks.sync_history(true);
        assert_eq!(docks.front, DockPanel::Changes);
        docks.sync_history(false);
        docks.sync_history(true);
        assert_eq!(docks.front, DockPanel::History);
        docks.sync_changes(first.clone());
        assert_eq!(docks.front, DockPanel::History);

        // Raising a covered Changes panel does not close history.
        docks.toggle_changes(false);
        assert_eq!(docks.front, DockPanel::Changes);
        assert!(docks.history_open);
        docks.toggle_changes(true);
        assert!(!docks.changes_open);
        docks.sync_changes(first);
        assert!(!docks.changes_open);

        // A new proposal opens Changes, including a replaced conversation.
        docks.sync_changes(Some((1, "reply-1".to_string())));
        assert!(docks.changes_open);
        assert_eq!(docks.front, DockPanel::Changes);
        docks.sync_changes(None);
        assert!(!docks.changes_open);
        assert!(docks.history_open);
    }
}
