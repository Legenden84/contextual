# CodeBlock

Code block in chat: a header bar with the file path (or language label), language, Wrap lines and Copy, over the same editor renderer.

`Contextual.codeBlock(code, { path | lang, maxLines, badge, wrap })`. With `maxLines` the block is cut and ends in "Show N more lines". Inline code in prose uses `Contextual.inlineCode(text)` (the pill).
