---
name: issue-lifecycle
description: Use whenever specing, creating, implementing, or wrapping up work in solutionscay/record-maker that touches a GitHub issue — covers create → branch → implement → verify → merge → close, with an end-of-task sweep to make sure no touched issue is left open. Trigger any time an issue number is mentioned, a new feature/bug is being planned, or a task is being wrapped up.
---

# Issue lifecycle (record-maker)

This repo's recurring failure mode: an issue gets created, the work gets done and
committed (sometimes even referencing the issue number in the commit message,
e.g. `(#85)`), and the issue is never closed. The user then has to manually
audit open issues to find work that already shipped. **Closing the issue is not
optional cleanup — it is part of "done."** Do not report a task complete while
any issue it touched is still open, unless you say so explicitly and why.

Plain `(#85)` in a commit message or PR title does **not** auto-close anything
on GitHub. Only `Fixes #85` / `Closes #85` / `Resolves #85` in a commit that
lands on the default branch, or in a merged PR's body, triggers auto-close —
and even then, verify it actually happened. This repo has open issues today
whose work already shipped because of exactly this gap.

## The six steps

### 1. Spec — create the issue before writing code

For any nontrivial change (skip only for trivial one-liners), create the issue
first with `gh issue create`. Use the structure that matches this repo's
existing issues:

**Feature / enhancement:**
```
## Summary
## Context / what exists
## Scope
## Out of scope
## Acceptance
```

**Bug:**
```
## Bug
## Diagnosis
## Expected behavior
## Possible fix direction
```

Apply labels from the existing set — don't invent new ones:
`bug`, `enhancement`, `documentation`, `good first issue`, `help wanted`,
`question`, `wontfix`, `duplicate`, `invalid`, `contract`, `epic`, `engine`,
`mvp`, `editor`, `runtime`, `ai`, `data-model`, `decision`, `shell`.
`contract` = touches the permanent contract (metadata schema, calc semantics,
file format, relationship model) — see the locked-architecture context if
unsure whether something qualifies. If the work is large, split it into an
epic + sub-issues rather than one sprawling issue.

No FileMaker references anywhere in the issue title/body (legal caution —
this project is a clone in spirit, not in name).

### 2. Track it so it survives context compaction

Immediately after creating (or picking up) the issue, add a task via
TaskCreate: **"Close #N after work lands on main."** This is the actual fix
for the root cause — the reason issues get left open isn't malice, it's that
the closing step falls out of context over a long session or after
compaction. A tracked task doesn't fall out.

### 3. Branch before implementing

`git checkout -b feat/<N>-<slug>` or `fix/<N>-<slug>`. Never implement
directly on `main`. (See [[workflow-branch-and-libs]] — also don't self-install
missing system libs; hand the user the commands instead.)

### 4. Implement and verify

Commit with a descriptive message; referencing `(#N)` is good for
traceability but, per above, does not by itself close anything.

Before merging, verify the build — this repo's real gate:
- `npm run check` (svelte/type check, from `ui/`)
- `cargo test -p record-maker-server` (or the relevant crate)
- `npm run build`

Never push/merge a build you haven't run.

### 5. Consolidate to main

This user's preference is direct consolidation over long-lived branches
(see [[git-consolidate-to-main]]): merge the branch into `main`, re-verify the
build post-merge, then delete the branch (local and remote). If a PR was
opened instead, merge it with `gh pr merge`.

### 6. Close the issue — do not skip, do not assume

Right after the work is on `main`, explicitly close it:

```
gh issue close <N> --comment "Shipped in <sha or PR #>: <one-line summary>."
```

If you used a PR with `Closes #N` in the body, don't just assume it worked —
confirm:

```
gh issue view <N> --json state -q .state
```

If it still says `OPEN`, close it manually with the command above.

Then mark the TaskCreate item from step 2 completed.

## End-of-task sweep (always do this before saying "done")

Before reporting any task finished, check every issue number touched or
mentioned in the session:

```
gh issue list --state open --json number,title
```

Cross-reference against issue numbers you created, branched for, or
referenced in commits this session. Anything that shipped but is still open
gets closed now, not left for the user to find. If something is intentionally
left open (deferred scope, blocked on something else), say so explicitly in
your summary rather than silently leaving it.
