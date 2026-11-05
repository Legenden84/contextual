# DiffFile

One file in a GitHub-style diff: header with status, path and +/− bar, Copy file content, Copy diff and Include in apply; body in unified or split view.

`Contextual.diffFile({ path, status, old, new, include, conflict, collapsed }, { mode })` computes the line diff itself (LCS), pairs removed/added lines for word-level highlights, and collapses unchanged runs into "Expand N lines" rows with the `@@` header and enclosing member. A `conflict` string adds the amber banner.
