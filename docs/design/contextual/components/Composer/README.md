# Composer

Message input: the context bar, a text box, the deployment picker and the per-message context mode.

"Resend full context" sends the selected files again with the message (use after editing files); "Only send new message" relies on what the conversation already holds. Ctrl+Enter sends.

**Attach images…** opens the native file picker with multiple selection. **Paste image** reads
an image or copied image files from the clipboard; the text box also accepts the normal paste
shortcut. Image files can be dropped anywhere in the chat pane, including the thread and the
composer. The chat surface highlights during a file drag when attachments are available.
Ordinary text paste is unchanged.

Attachments are read on the UI machine, independently of the project's location. In WSL dev
runs, Windows paths delivered by a drop, picker or clipboard are translated with `wslpath`,
which respects configured drive mounts. This requires the desktop to deliver the file event:
WSLg does not reliably forward Windows Explorer drags. When no drop reaches the app, use
**Attach images…** to select the file on a mounted Windows drive, or run the Windows build.

Supported formats are PNG, JPEG, GIF and WebP, up to 20 MiB per source image. Images are prepared
off the UI thread and scaled to a maximum side of 2048 pixels. GIFs are sent as a still image.
Each attachment shows its name, sent dimensions, estimated tokens and a Remove action.
Errors appear beside the attachment controls; Send waits until preparation finishes.

Images are sent only when the user presses Send, with or without text, and are kept in the saved
conversation. Starting or opening another chat clears unsent images. Attachment controls are
unavailable while a response is in progress.
