# ChangesPanel

Right-hand panel listing cumulative unsaved proposals to add, modify or delete files, each selectable for Save or Discard.

Header counts by status and total lines; rows show status letter, name, folder, +/− and a conflict marker. Conflicting files start unselected. The footer opens the diff or the Save dialog with the selected count.

**Discard N files** removes the checked proposals immediately without changing project files. Original replies remain in history, and the last Save's Undo remains available. A short explanation above the footer distinguishes discarding proposals from reverting disk changes. Save and Discard are disabled during a response, Save/Undo or a running command.

Discard decisions survive reopening the chat and are reported to the model when the conversation next continues. Discarded versions are no longer supplied as pending context.
