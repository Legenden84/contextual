# ChangesPanel

Right-hand panel listing cumulative unsaved proposals to add, modify or delete files, each selectable for Save.

Header counts by status and total lines; rows show status letter, name, folder, +/−, a conflict marker and a small ghost **Discard** button. Conflicting files start unselected. Clicking a file row opens its diff; clicking its checkbox or Discard does not. There is no separate Open diff button. The footer opens the Save dialog with the selected count.

**Discard** removes only that row's proposal immediately, regardless of its Save checkbox, without changing project files. Original replies remain in history, and the last Save's Undo remains available. A short explanation above the footer distinguishes discarding proposals from reverting disk changes. Save and per-file Discard are disabled during a response, Save/Undo or a running command.

Discard decisions survive reopening the chat and are reported to the model when the conversation next continues. Discarded versions are no longer supplied as pending context.
