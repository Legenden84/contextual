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
  Chat and file previews are centred in the entire area to the right of the explorer,
  capped at the 820px reading width plus horizontal padding. Their horizontal centre
  never changes when panels open or close.
- Changes and chat history dock side by side when two standard sidebar widths fit in
  the centred column's natural right gutter, with history rightmost. Otherwise they
  share one slot, with the most recently opened panel on top. The covered panel keeps
  its state but is not painted or interactive; its header button brings it forward.
- When even one sidebar cannot fit in the natural gutter, the chat narrows symmetrically
  just enough to fit one. Opening a second panel does not narrow it further. At extreme
  zoom or narrow widths, the slot is capped at one third of the available area and its
  contents scroll horizontally rather than disappearing.
- A new proposal opens Changes in front. Updating an existing proposal, searching history
  or resizing the window does not change the stacking order. Header buttons can hide
  the visible panel or bring a covered panel forward.
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
- Chat history is docked on the right and open by default. It uses the standard sidebar
  width and a hairline separator, not a floating popover, and stays open while composing.
- Selecting a saved chat explicitly loads it and closes history. New chat also closes
  history. Search, rename and deletion remain available in the panel.
- Close history or use the header toggle without shifting the messages or composer's
  horizontal centre. Their width changes only when space is needed for a sidebar.
  Closing the front panel reveals the other if it is still open; on wider windows
  both remain visible side by side. Opening history from Diff or Inspect returns to Chat.

## Native Source Control workflow

- Pull requests belong in the Source Control sidebar, not the centre tab bar.
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
