# ScreenWorkspace

The main workspace with a project open, files selected and a finished assistant response that proposes changes.

- Left: explorer (12 files, ~7.5k tokens) with Skills & Rules below; one file carries the changed-on-disk badge and the excluded `packages.lock.json` shows its reason on hover.
- Centre: chat thread with the Response/Diff toggle per assistant message, then the composer with the context bar at rest.
- Right: Changes panel, 5 files, 4 selected; the conflicting file is left out.

## Native workspace tabs

- A persistent header spans the centre and right columns, with Chat pinned first.
- File previews, Diff and Inspect open as closable tabs in opening order. Reopening a
  feature selects its existing tab rather than adding a duplicate.
- Switching tabs preserves the composer draft and the feature view state.
- Diff and Inspect use the full area beneath the header, hiding the right sidebar.
  Chat and file previews are capped at the 820px reading width plus horizontal padding.
  Centre the reading column in the area remaining after the explorer and all visible
  right panels have been subtracted. Responses, context summary and composer share
  this column; neither side should carry an oversized empty gutter.
- Changes and chat history dock side by side when two standard sidebar widths fit
  alongside a full reading column, with history rightmost. Otherwise they share one
  slot, with the most recently opened panel on top. The covered panel keeps its state
  but is not painted or interactive; its header button brings it forward.
- Dock changes move the reading column into place with a 180ms ease-out transition.
  Position, width and dock reservation move together; code and text are never scaled.
  A second toggle retargets from the current position without jumping. Ordinary chat
  updates do not restart the transition. Initial layout, window resizing, zoom and
  compact navigation use their final geometry immediately. Reduce motion disables
  the transition. No animation frames are requested after it settles.
- Dock a right panel only when there is room for its standard width and at least a
  480px chat column. Below that threshold, History and Changes become explicit
  full-width views beneath the header. Their buttons open or hide them; Chat returns
  to the draft. Default-open history and incoming proposals never replace Chat merely
  because the window is narrow. Compact navigation does not discard panel state.
- When the explorer would leave less than 480px for the main area, collapse it into a
  Project button. Project opens Files / Rules / Source control beneath the same header;
  Back to chat returns to the conversation. The existing sidebar state is retained.
- At compact widths the tab strip and auxiliary navigation occupy separate rows.
  Tabs scroll horizontally; navigation buttons wrap instead of pushing Chat offscreen.
  Breakpoints use design-space pixels, so application zoom follows the same rules.
- A new proposal opens Changes in front in docked layouts. Updating an existing proposal,
  searching history or resizing the window does not change the stacking order. Header
  buttons can hide the visible panel or bring a covered panel forward.
- Closing the active tab selects its next neighbour, then its previous neighbour,
  falling back to Chat. Closing an inactive tab does not change the selection.
- Starting or opening another conversation closes Diff and Inspect and selects Chat;
  project file previews remain open.

## Native startup and chat history

- Normal launches show Welcome with Open folder and recent projects. The last project
  is never reopened automatically. Explicit CLI paths, project links and developer
  screen flags remain supported; `--no-restore` is accepted as a compatibility no-op.
- Opening a project, including a recent project, starts a fresh conversation with no
  previous messages or file context. Loading the history list never loads a conversation.
- Chat history is docked on the right and open by default when there is room. It uses the
  standard sidebar width and a hairline separator, not a floating popover. Compact
  windows start with Chat visible and expose history through the header button.
- Selecting a saved chat explicitly loads it and closes history. New chat also closes
  history. Search, rename and deletion remain available in the panel.
- Closing history recentres the messages and composer in the newly available area,
  using the same eased transition as opening it. Closing the front dock reveals the
  other if it is still open; on wider windows both remain visible side by side.
  Opening history from Diff or Inspect returns to Chat. Compact views show one
  auxiliary panel at a time and return to Chat when hidden.

## Native Source Control workflow

- Pull requests belong in the Source Control sidebar, not the centre tab bar.
- The opened folder is always the workspace. Discover repositories during scanning;
  there is no workspace-mode setting or setup dialog.
- With nested repositories, show their workspace-relative names, branches and pending
  file counts above the Git controls. Select the first discovered repository initially
  and retain the selection across rescans while it exists.
- Repository selection scopes commit, push, branch and PR actions only. Chat, file
  selection and Save/Undo remain workspace-wide. Commit-message drafts are retained
  separately per repository.
- List pending files from all discovered repositories, grouped under repository names
  and file counts. Each group has independently collapsible Staged changes and Changes
  sections, its own conflict notice, and a clean-state message when empty.
- Row and section staging actions always target their containing repository without
  changing the selected commit/push scope. One repository's status error must not hide
  the other groups.
- A diff tab keeps its original repository even after another repository is selected.
  Branch and pull-request reviews name their repository; a PR review keeps that scope
  until closed. Never combine commits or pushes across repositories.
- Keep branch selection, commit, push/publish and pull-request comparison together.
- Give the next action emphasis: commit with pending changes, push/publish with a clean
  working tree, or compare once the branch is pushed. Comparison remains available before
  pushing; it does not push or create a PR.
- The pull-request section explains that uncommitted changes are not included.
- “Compare & pull request” opens the base/compare review with commits and file diffs.
  Creation is a separate explicit action; its label names a push when commits are unpushed.
- Existing pull requests offer “Open on GitHub” and pushing new commits rather than
  creating another PR. Closing the review leaves Source Control visible.

Render with `Contextual.screens.workspace({ height: 900 })`.
