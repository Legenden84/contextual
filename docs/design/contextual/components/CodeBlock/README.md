# CodeBlock

Code block in chat: a header bar with the file path (or language label), language, Wrap lines and Copy, over the same editor renderer.

`Contextual.codeBlock(code, { path | lang, maxLines, badge, wrap })`. With `maxLines` the block is cut and ends in "Show N more lines". Inline code in prose uses `Contextual.inlineCode(text)` (the pill).

## Native chat behaviour

- Code uses the read-only editor with syntax highlighting, selectable text and numbered gutters. Copy includes the full source, without line numbers.
- The collapsed preview adapts to the available conversation height. Expanding reveals the remaining source inside a bounded editor, not an unbounded message.
- The expanded card, including its header and footer, fits within the conversation viewport above the composer. It is remeasured when that viewport changes.
- Header and footer stay outside the editor's scrolling body. The footer offers "Show fewer lines" and explains scrollbar ownership.
- Target interaction: wheel and touchpad gestures anywhere in Chat scroll the conversation; code is navigated with its own draggable scrollbar or native editor keyboard navigation.
- Current implementation uses native scrolling: gestures over the conversation scroll the conversation; gestures over code or the composer scroll that control. Conversation-wide wheel routing is not yet implemented.
- The conversation has a separate scrollbar at the Chat pane edge; code scrollbars remain inside their cards. Expanding a block preserves the conversation's reading position.
