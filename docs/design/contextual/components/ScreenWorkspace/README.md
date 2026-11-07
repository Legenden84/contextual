# ScreenWorkspace

The main workspace with a project open, files selected and a finished assistant response that proposes changes.

- Left: explorer (12 files, ~7.5k tokens) with Skills & Rules below; one file carries the changed-on-disk badge and the excluded `packages.lock.json` shows its reason on hover.
- Centre: chat thread with the Response/Diff toggle per assistant message, then the composer with the context bar at rest.
- Right: Changes panel, 5 files, 4 selected; the conflicting file is left out.

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
