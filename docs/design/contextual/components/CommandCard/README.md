# CommandCard

The review card for one command the model proposed. It sits under the assistant reply that
proposed it, one card per proposal, in the chat's reading column.

This component is not part of the imported design kit (see `SOURCE.md`): it was specified here
for approved command execution and uses only existing tokens and controls. It is shown only
when the user has turned on "Let the assistant propose commands" in Settings (the Commands
group: one switch, off by default, with the effects note and "Takes effect the next time you
start Contextual").

## Principle

The model proposes; the user runs. Nothing on the card starts anything except **Run**, and
Run approves exactly what the card shows at that moment. The card states what it knows and no
more: a command that ran is "Finished: exit code 3", not "Failed safely"; an interrupted one
is "Outcome unknown".

## Anatomy

- **Frame.** `radius-md`, 1px `border`, no shadow, `space-3` above. Same weight as a code
  block or a diff card.
- **Header.** `surface-200` with a `border` hairline below. "Proposed command" in
  `ink-strong` semibold, then the status in the status colour, then an "Edited by you" tag
  (warning tone) when the request on screen differs from what the model proposed. `ui-small`.
- **Body.** `space-3` padding, rows `space-2` apart. Each row is a muted `ui-small` label in a
  fixed column (three `space-8` steps) and its value.
  - *Purpose (model-provided)* in the UI sans. The label says whose words they are.
  - *Program* and one row per *Argument N*, or *Interpreter* and *Script*, in `code-small`.
    One row per argument makes the boundaries exact; an empty argument reads "(empty)" in
    `ink-faint`. A script keeps its line breaks.
  - *Executable* (the resolved path) and *Directory* (the resolved path, then the requested
    one) once the host has prepared the command. Before that, only the requested directory.
  - *Timeout*, *Environment* ("No overrides" or `NAME=value` rows) and *Runs on*
    ("This computer (Linux)", "WSL: Ubuntu").
- **Output**, while running and after: the last 24 lines of each stream in `code-small` on
  `surface-200`, `radius-sm`, each with a muted caption ("Output", "Error output", how many
  lines exist, and whether more was produced than was kept). After the command ends, a muted
  line says the output is kept in memory only and has not been sent to the model.
- **Notes**, `ui-small`: the effects note in `ink-muted` while the command can still run; the
  stop note while it runs; why an action is unavailable, or what went wrong, in `warning`.
- **Actions**, extra-small buttons, `space-2` apart.

## States

| Status | Colour | Actions |
| --- | --- | --- |
| Preparing / Not prepared | `ink-muted` | Prepare again, Reject |
| Needs review | `warning` | **Run** (primary), Edit, Reject |
| Needs review to run again | `warning` | **Run again** (primary), Edit |
| Starting | `ink-muted` | none |
| Running | `ink-muted` | Stop |
| Finished: exit code 0 | `success` | Review to run again, Share result… |
| Finished: exit code N / Did not start | `danger` | Review to run again, Share result… |
| Stopped / Stopped: timed out | `warning` | Review to run again, Share result… |
| Outcome unknown | `warning` | Check whether it still runs |
| Rejected / Not run | `ink-muted` | none |
| Could not be read as a command | `warning` | none |

Run and Edit are disabled, with the reason shown, while another command runs in the project,
while one ended with an unknown outcome, or while the model is replying. Once the model has
been told about a proposal (the conversation continued), its card offers nothing and says what
the model was told.

**Editing** replaces the body with a note, a `code-small` text area holding the request, and
**Review changes** (primary) / Cancel. Opening the editor voids any approval at once. An edit
is checked by the host and comes back as "Needs review"; it is never run from the editor.

## Sharing a result

Running a command sends nothing to the model. Sharing is a separate decision, in two steps.

**Share result…** (offered while the finished command's output is still held in memory)
replaces the card body with the preview:

- the warning, in `warning`: "Output may contain secrets or contents of files you did not
  select."
- a tick box per stream, "Include output" and "Include error output", each with an editable
  `code-small` text area when ticked, so a secret can be removed by hand;
- "The model will receive exactly this:" and the structured result as it will be sent, in
  `code-small` on `surface-200`. It follows every tick and keystroke, and states which streams
  were omitted and whether the text was edited;
- "What you share is sent to the model and becomes part of the saved chat.";
- **Use this result** (primary) / Cancel.

*Use this result* sends nothing. The card then says what the model will receive when the
conversation continues, with **Don't share** to withdraw it.

Under the cards of the latest reply sits the **Continue row**: a primary extra-small
**Continue** button and one muted sentence, for example "The model will be told: 1 result
shared, 1 rejected, 1 ran without sharing its result." Continue is the only thing that tells
the model; it sends one request. A proposal the user has not decided on is reported as not
run, and its card closes. The row is disabled while a command runs or the model replies.

A result that shares no outcome ("rejected", "not run", "not shared") carries the call's
identity, that word and a sentence. Nothing about how a command ended.

## Copy

- Effects: "Commands may modify files or access the network. Save/Undo does not undo command
  effects."
- Stop: "Stopping ends the command. It does not undo what it already did."
- Output: "This output is kept in memory only. It has not been sent to the model."
- Buttons are verbs: Run, Run again, Edit, Reject, Stop, Prepare again, Review to run again,
  Review changes, Check whether it still runs, Share result…, Use this result, Don't share,
  Continue.

## Accessibility

Status is always text, never colour alone. Disabled actions keep their labels and are
accompanied by a sentence that says why.
