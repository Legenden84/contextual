# CodeView

Read-only editor view used for every code surface: muted line-number gutter, current-line band, indentation guides, VS Code Dark+ / GitHub Light syntax colours.

`Contextual.codeView(code, { path | lang, currentLine, wrap, start, activeGuide })` returns HTML. Language comes from the file extension or `lang`. Tabs are 4 columns; ligatures are off.
