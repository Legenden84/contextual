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

The card is one line when there is nothing to decide, and grows only by what the moment needs.

- **Frame.** `radius-md`, `surface-200`, 1px outline, no shadow, `space-3` above. The outline
  is `accent` while the card waits for the user, `danger` when the command ended badly
  (non-zero exit, did not start, timed out), and `border` otherwise.
- **Header line.** `space-3` / `space-2` padding, one row:
  - a lead icon in `ink-muted`: a terminal, or a chevron when the card has output. With
    output, the whole line folds and unfolds it;
  - the command in `code`: a faint `$`, the program in `ink-strong`, arguments in `ink`,
    arguments that start with `-` in `syn-keyword`. An argument that is empty or holds
    anything but plain characters is shown in single quotes in `syn-string`, so its
    boundaries stay clear. A one-line script is shown as written; a longer one reads
    "bash script · 3 lines" here and is shown whole in the body. The line wraps;
  - an "Edited by you" tag (warning tone) when the request differs from what the model
    proposed;
  - the status on the right in `ui-small`: a mark, the words, and the time it took
    ("exit 0 · 18 ms", "Running · 12 s"). The spinner stands still with Reduce motion.
- **Body**, indented to sit under the command text, rows `space-2` apart, each only when it
  applies:
  - the purpose in the model's words (`ui-body`), while the command can still be run;
  - a script of more than one line, complete, in `code-small` on `surface-100`;
  - one muted `ui-small` line of facts: "in `<directory>`" when it is not the project root,
    each environment override as `NAME=value`, and, while the command can still be run,
    "timeout 120 s" and the effects note with a warning triangle in `warning`;
  - **output**, while running and after: one `code-small` block on `surface-100`,
    `radius-sm`, holding the last 24 lines of the output and then of the error output
    (introduced by a faint "error output" when both exist), with faint "… 38 earlier lines"
    and "… more was produced than was kept" where true. A finished command that printed
    nothing says "No output";
  - **details**, when opened: muted labels in a fixed column (three `space-8` steps) with
    *Program* or *Interpreter* and *Script*, *Arguments* as one `pill-bg` pill per argument
    (an empty one reads "(empty)" in `ink-faint`), *Executable* and *Directory* as resolved by
    the host, *Timeout*, *Environment* and *Runs on*. This is the exact form of what Run
    approves;
  - what will be shared on Continue (`ink-muted`), why an action is unavailable or what went
    wrong (`warning`), and what the model was told (`ink-muted`; with a `success` check when
    it was given the result).
- **Footer.** Extra-small buttons. Decisions on the left, the primary one first, each
  followed by the one sentence that belongs to it; **Details** / **Hide details** and
  **Run again…** as ghost buttons on the right.

## States

| Status | Colour | Left | Right |
| --- | --- | --- | --- |
| Preparing / Not prepared | `ink-muted` | Prepare again, Skip | Details |
| Waiting for you | `accent-ink` | **Run** or **Run again** (primary, play icon), Edit, Skip | Details |
| Starting | `ink-muted` | none | Details |
| Running · 12 s | `ink-muted` | Stop (square icon), the stop note | Details |
| exit 0 · 18 ms | `success` | "Output stays local." | Details, Run again… |
| exit N / no exit code / did not start / timed out | `danger` | as above | Details, Run again… |
| stopped | `warning` | as above | Details, Run again… |
| Outcome unknown: … | `warning` | Check whether it still runs | Details |
| Skipped / Not run | `ink-muted` | none | Details |
| Could not be read as a command | `warning` | none | none |

Run and Edit are disabled, with the reason shown, while another command runs in the project,
while one ended with an unknown outcome, or while the model is replying. Once the model has
been told about a proposal (the conversation continued), its card offers only Details and says
what the model was told.

**Run again…** opens the review again ("Waiting for you"); nothing runs until Run is pressed
there. **Skip** tells the model the proposal was rejected.

**Keyboard.** With an empty composer, the send shortcut (Ctrl+Enter, ⌘Enter on macOS) runs
the first command that is waiting; that card shows the keys next to Run. With text in the
composer the shortcut sends the message, as always.

**Editing** keeps the header line and replaces the body with a note, a `code-small` text area holding the request, and
**Review changes** (primary) / Cancel. Opening the editor voids any approval at once. An edit
is checked by the host and comes back as "Needs review"; it is never run from the editor.

## Continuing after a command

Running a command sends nothing to the model. The card has no **Share output…** button
and does not open a sharing preview. Captured output remains available locally; removing
the button does not enable automatic sharing.

Under the cards of the latest reply sits the **Continue row**: a primary extra-small
**Continue** button and one muted sentence summarising what the model will be told.
Continue sends one request. A command that ran without a staged result is reported as
"not shared"; a proposal the user has not decided on is reported as not run, and its card
closes. The row is disabled while a command runs or the model replies.

The underlying result-staging support remains for debug automation. If a result is staged,
the card still describes it and offers **Don't share** to withdraw it before Continue.

A result that shares no outcome ("rejected", "not run", "not shared") carries the call's
identity, that word and a sentence. Nothing about how a command ended.

## Copy

- Effects, on the card: "May change files · not covered by Undo". Nothing is known about what
  a command will do, so every command that can still run carries it. The full sentence
  ("Commands may modify files or access the network. Save/Undo does not undo command
  effects.") is in Settings.
- Stop: "Stopping ends the command. It does not undo what it already did."
- Output: "Output stays local."
- Buttons are verbs: Run, Run again, Edit, Skip, Stop, Prepare again, Run again…,
  Review changes, Check whether it still runs, Don't share, Details, Hide details, Continue.

## Accessibility

Status is always text, never colour or a mark alone. Disabled actions keep their labels and
are accompanied by a sentence that says why. The exact argument boundaries are always one
click away in Details, and quoting on the header line keeps them unambiguous before that.
