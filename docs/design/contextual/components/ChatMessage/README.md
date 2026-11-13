# ChatMessage

Chat messages: the user's request with a context chip, and the assistant's response rendered as markdown with code blocks.

The assistant header shows model, deployment, time, tokens in/out and cost, plus the Response/Diff toggle. File blocks retain their `### FILE:` / `### EDIT:` / `### DELETE:` markers in the source; the rendered header uses a file glyph, semibold monospace filename, muted directory and proposal status on a raised surface.

An accepted prompt appears in the thread and chat history immediately, before context reading or a response completes. Its history row says “In progress” while the request runs. Conversation checkpoints preserve the prompt even when the request fails or is stopped; incomplete attempts do not count as completed model turns.

While the assistant reply is active, a small activity line sits directly below its model header, including before the first visible text arrives. It uses the existing code-flow mark and observable phase labels such as “Waiting for response”, “Writing response” and “Checking edits”, including capacity-wait countdowns. It disappears when the operation ends. These labels do not claim access to private model reasoning.

Only the trailing file action being written animates, using the quiet code-flow mark without moving text or layout. Reduce motion makes the mark static. Received drafts and validated proposals have a circular green check, labelled “Draft received” and “Ready to review” respectively; neither means saved. Conflicts remain “Needs review”, and interrupted proposals without a reviewable change never receive a success check. Context-read rows animate during fetching and receive a check only after successful completion without refused or held-back paths. The composer activity indicator is unchanged.

With several models, each message of the user is followed by a short entry under the routing model's name: its context-read row if it read files, then a routing row, "Chose <model>" with what applies of "one tier above <model> (unsure)", "switched from <model>" and "<n> files handed over", the routing model's one-sentence reason below it in muted ink, and the handed-over paths in small code type. Its header carries the routing model's own tokens and cost. When routing fails, the row has a warning mark and says "Could not choose a model · using <model>" with the cause. The entry has no reply text. The answer follows under the chosen model's name. In Inspect, routing rounds carry a "routing" tag and every round names its model.

An invalid routing reply receives one corrective follow-up using the same router and its
existing context; the follow-up requires a decision without further file reads. Both rounds
remain inspectable and their usage is included in the routing entry. If routing still fails,
the chat keeps its current model only when that model is still offered. Otherwise the routing
entry says that no answering model was called and asks the user to choose a model with the
composer selector and resend the request. A routing failure never automatically selects the
strongest model.

A footer row summarises the detected changes with a Review diff button.
