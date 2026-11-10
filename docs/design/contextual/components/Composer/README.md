# Composer

Message input: the context bar, a text box, image attachments and Send.

Messages always use incremental file context: only new or changed file contents are attached,
based on their hashes. Unchanged files remain available through the conversation without
duplicating their contents in the new turn. The model can request additional files as needed.
There is no context-mode selector. Ctrl+Enter sends.

## Responsive layout

The composer fills the available chat column up to the reading-width cap; its flex
children must be allowed to shrink. Context counts, budget warnings and estimated
cost wrap onto additional lines instead of overflowing behind an adjacent panel.
Inspect remains a separate control at the top right of the context summary.

The input grows from two to six lines before scrolling internally. Attachment lists
scroll after three standard row heights so a large batch does not consume the thread.
Activity labels and message metadata wrap when needed; Send / Stop do not shrink.

## Markdown typing

The composer remains a native plain-text input: Markdown markers stay visible and editable.
Typing `* `, `- `, `+ `, `1. ` or `1) ` on a new, unindented line gives the marker a small
two-space text indent. An asterisk without its following space stays literal. Existing
indentation is preserved, and bulk pasted text is not automatically reformatted.

With the caret anywhere on a list item, Tab indents that line by two more spaces and changes
an unordered bullet (`*`, `+` or `-`) to `-`. Shift+Tab removes up to two leading spaces or one
leading tab, keeping the marker. Numbering and checkbox state are preserved. These shortcuts
also work on empty items; Enter then continues at the new indentation level. The caret stays
at the same position relative to the content, and each indentation edit uses native undo.
Selected text, ordinary prose, quotes and fenced code retain native Tab behavior. Shift+Tab
on an already unindented item also falls through, allowing keyboard focus to leave the input.

Enter continues a bullet, increments a numbered list, or continues a `> ` quote. Checklists
such as `- [x] done` continue with an unchecked `- [ ] ` item. Enter on an empty item removes
its marker and indentation and inserts a regular line break. Shift+Enter inserts a plain
newline without continuing the marker. Enter with selected text replaces that selection with
a plain newline.

Typing assistance leaves fenced code and thematic breaks alone. Automatic edits use the
input's undo history rather than resetting the draft; IME composition is not reformatted.
This is Markdown typing assistance, not a rich-text editor or formatting toolbar.

## Activity and attachments

A single activity indicator sits between the attachment control and Send/Stop. Its fixed-size
code-flow mark has three horizontal strokes; a soft accent highlight passes through them in
a 1.8-second loop. Only stroke opacity changes: no bouncing, rotation, animated text, glow or
layout movement. The steady small-text label follows actual work: Preparing context, Waiting
for response, Writing response, Reading files, Checking edits, Saving changes, Undoing changes
or Stopping. Writing a response never means writing project files.

Image preparation uses the same indicator. The native image picker and an over-limit read
request instead show static states, Choose images and Waiting for your approval. Idle, completed
and failed operations have no activity mark. There is no duplicate Thinking label in the thread.
The mark keeps its animation identity across phase changes and has no independent background
timer. Settings → Appearance → Reduce motion uses a static mark with the same readable labels.

An icon-only Lucide **brain** control sits at the right of the context bar above the message
input, with the tooltip **Inspect conversation**. It opens the existing Inspect tab showing
the raw inputs and outputs, not private model reasoning. It is disabled until the chat has
a recorded transcript or a live round. Keyboard focus is visible; Enter or Space opens it.
The workspace header no longer contains a separate Inspect action.

An icon-only **+** control beside Send opens a small attachment menu, initially containing
**Images** with a leading image icon, which opens the native file picker with multiple selection. Its tooltip is
**Add attachment**. Enter or Space opens the menu and activates its item; Escape closes it.
Clicking outside dismisses the menu. More attachment types can be added here later.

The text box accepts images and copied image files through the normal paste shortcut.
Image files can be dropped anywhere in the chat pane, including the thread and the composer.
The chat surface highlights during a file drag when attachments are available.
There is no dedicated paste button or persistent drop instruction. Ordinary text paste is unchanged.

Attachments are read on the UI machine, independently of the project's location. In WSL dev
runs, Windows paths delivered by a drop, picker or clipboard are translated with `wslpath`,
which respects configured drive mounts. This requires the desktop to deliver the file event:
WSLg does not reliably forward Windows Explorer drags. When no drop reaches the app, use
**+ → Images** to select the file on a mounted Windows drive, or run the Windows build.

Supported formats are PNG, JPEG, GIF and WebP, up to 20 MiB per source image. Images are prepared
off the UI thread and scaled to a maximum side of 2048 pixels. GIFs are sent as a still image.
Each attachment shows its name, sent dimensions, estimated tokens and a Remove action.
Rejected attachments appear in a visible warning inside the composer. Oversized images are
not attached; the warning names the file, states the 20 MiB limit and asks for a smaller image.
Valid images from the same batch are still attached. This applies to drops, picks and pastes.
A preparation label appears only while loading; Send waits until preparation finishes.

Images are sent only when the user presses Send, with or without text, and are kept in the saved
conversation. Starting or opening another chat clears unsent images. Attachment controls are
unavailable while a response is in progress.
