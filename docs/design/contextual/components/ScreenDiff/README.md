# ScreenDiff

The diff view opened full-width over the chat for one response.

Toolbar: Back to chat, response summary with total +/−, Unified/Split (live in this preview), Expand all, Apply. A file list on the left mirrors the Changes panel. `Order.cs` is expanded with collapsed unchanged regions and a word-level highlight on `!=` → `is not`; `OrderEndpoints.cs` shows the conflict banner and is not included in apply.

The split (side-by-side) layout is shown in `ScreenDiffSplit`.

Render with `Contextual.screens.diff({ mode: 'unified' | 'split' })`.
