# Composer

Message input: the context bar, a text box, image attachments and Send.

Messages always use incremental file context: only new or changed file contents are attached,
based on their hashes. Unchanged files remain available through the conversation without
duplicating their contents in the new turn. The model can request additional files as needed.
There is no context-mode selector. Ctrl+Enter sends.

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
