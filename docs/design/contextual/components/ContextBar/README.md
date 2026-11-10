# ContextBar

Context budget above the input: selected files, active rules, estimated next-input tokens,
a meter against the deployment's input limit, and estimated cost per API round.

The file count means explorer selection, not all files retained in the conversation.
The token total includes earlier messages and replies, instructions, new or changed files,
loaded skills, attachments, and usable pending proposals. Saving changes or deselecting files
does not erase earlier conversation. Starting a new chat does.

While a response streams, show a provisional estimate using its prepared input and reply so
far. Do not count selected files, attachments or the project tree twice. On completion,
replace the provisional round with stored history; on failure or Stop, discard it.
Both Chat and file-preview gauges must refresh without reopening the app. Cache stored
accounting separately so streaming does not rescan the transcript.

This is a heuristic estimate, not provider-reported usage or cumulative billed tokens.
Text and image estimates are approximate. The display rounds to 100 tokens and the draft
currently uses a fixed 96-token allowance rather than its actual typed length. Explain these
limitations in the tooltip.

Keep the summary focused on input context; do not show a completion-limit label or reasoning
disclaimer. The send-time safety check reserves output and an estimation margin, so a request
can be refused below 100% on this input gauge.

Under 75% the meter is `accent`; 75–94% turns `warning`; 95% and above turns `danger`.
Use "Near limit" below 100%, and "Over limit" only at 100% or above. Prefer an earlier-conversation
hint when history dominates rather than implying that deselecting a folder will erase it.

Cost is per API round, not per user message: automatic reads and edit repair may add rounds.
The estimate assumes 1,800 output tokens and no cache discount; actual usage and cost belong
on completed replies.
