<!--
Start with one or two short paragraphs, 120 words at most, written the way
you would explain the change to a colleague at their desk: what a person saw,
and what they see now. Name the symptom, not a category. No headers, bullets
or file names here: internal names go under How.

A reviewer should understand the change and see that it works before reading
a line of How. Delete any section below that adds nothing.
-->

## Before and after

<!--
Evidence made for this pull request, from the same input on main and on this
branch. Give each figure a bold title of two to five words, then one sentence:
what input was replayed, what differs, and where to look. Every figure answers
a question a reviewer would ask; if it does not, cut it.

  **Pinning a button.** Same replay on both builds: on main the card covers
  the pin, now it opens beside it.

  | main | this branch |
  |---|---|
  | ![main: the card covers the pin](./main.png) | ![this branch: beside it](./branch.png) |

Pairs go in a table, wide subjects in one stacked image, motion in one GIF with
both builds in it. The overlay comes from replays on the demo's desktop, the
asides window from ASIDES_WINDOW_CAPTURE with a history of its own
(CONTRIBUTING.md): sample content only, never your own screen or asides.
Attach media (`gh pr create --attach ./main.png`, or paste it); do not commit
it. A change with nothing to look at shows a table of what changed (before,
after, source), or a measurement with its conditions.
-->

## Edge cases

<!-- The awkward inputs, and every existing flow that goes through what you changed. -->

| Case | How checked | Result |
|---|---|---|

---

## How

<!-- The decisions that are not obvious, and what you tried and dropped, with why. -->

## Verification

<!--
Results, not claims: "412 passed, 0 failed, 3 ignored", not "tests pass", for
`cargo fmt --all`, clippy as CONTRIBUTING.md runs it, `make test`, and `make e2e`
when the command or the app's behavior changed. Say which systems, and how: CI,
your own computer, a virtual machine. What only a person can check (shortcuts,
the microphone, permissions): the manual QA you ran, from CONTRIBUTING.md. Docs
updated, as AGENTS.md's "Keep the docs in sync" asks.
-->

## Not in this pull request

<!--
The scope you drew, what you could not verify (a system you could not run,
hardware you do not have), and anything you noticed and left.

When they apply, add: Review first (the two to four spots you are least sure of),
Merge notes (conflicts with open pull requests), Tests changed, and why.
-->
