# ScreenDiffSplit

The diff view in split layout: old file on the left, new file on the right, GitHub-style.

- Each side has its own line-number column; removed lines sit left, added lines right, paired row by row so a changed line lines up with its replacement and both get word-level highlights.
- An added line with no counterpart gets an empty grey filler cell on the other side (`diff-empty-bg`), and vice versa.
- Long lines wrap inside their half instead of scrolling, so both sides stay aligned.
- The diff's file list folds away in split mode to give each side room; the files button in the toolbar brings it back.
- Collapsed regions ("Expand N lines" and the `@@` header) span both columns.
- The Unified / Split toggle in the toolbar applies to every file in the view and is remembered per user (Settings → Appearance → Diff layout).

Render with `Contextual.screens.diff({ mode: 'split' })`.
