# ScreenDiff

The diff view opened in a Diff tab for one response, using the centre and right columns beneath the persistent workspace header. Chat, Inspect and file preview tabs remain available for switching; there is no Back to chat button in the native view.

Toolbar: response summary with total +/−, Unified/Split (live in this preview), Expand all, Apply. A file list on the left mirrors the Changes panel. `Order.cs` is expanded with collapsed unchanged regions and a word-level highlight on `!=` → `is not`; `OrderEndpoints.cs` shows the conflict banner and is not included in apply.

The split (side-by-side) layout is shown in `ScreenDiffSplit`.

Render with `Contextual.screens.diff({ mode: 'unified' | 'split' })`.
