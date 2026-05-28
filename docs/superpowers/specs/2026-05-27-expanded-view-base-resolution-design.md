# Expanded view base resolution — design

**Status:** approved, ready for implementation plan
**Date:** 2026-05-27

## Problem

Two related issues prevent Expanded view from working in a fresh worktree:

1. **Expanded over-bails.** When no base branch resolves, Expanded refuses to render anything and shows "Expanded view needs a base branch." But only one of its three groups (Committed) actually needs a base; the other two (Changes, New) are derivable from `git status` alone — exactly what Flat and Tree render successfully in the same scenario.

2. **Base resolution is too strict for worktrees.** Today's resolver has two tiers — explicit override (`--base` / `base_branch` config) and `origin/HEAD`. `git worktree add` doesn't set `origin/HEAD`, so a worktree on a repo where that ref was never set (or was removed) has no resolvable base and Expanded falls into problem #1.

## Solution overview

Two independent fixes. Either alone improves the user experience; together they restore Expanded to a fully useful state in worktrees.

- **Part 1:** Expanded renders the base-independent groups (Changes, New) even when no base resolves. The Committed group is silently omitted when absent — matching the existing "empty groups are hidden" contract.
- **Part 2:** Two new tiers in `resolve_base_branch` between the explicit override and `origin/HEAD`: the current branch's reflog fork point, then the main worktree's HEAD branch.

## Part 1: Expanded view degrades gracefully

### Change

Delete the bail-out block in `render_expanded_file_list` at `src/ui/mod.rs:375-378`:

```rust
if state.merge_base().is_none() && state.initial_seed_done() {
    render_expanded_no_base(frame, theme, area);
    return;
}
```

Also delete the now-unused `render_expanded_no_base` function at `src/ui/mod.rs:451`.

### Why it works without further changes

- `compute_with_base` (`src/git/worker.rs:240`) already falls back to `git.status()` when no base resolves, so `files` contains only Changes + New entries (status-only) and never Committed entries.
- `build_expanded_rows` (`src/ui/tree.rs:248`) already hides empty groups — verified by `EXPANDED_GROUP_ORDER` filtering and existing tests.
- Result: Committed disappears silently; Changes and New render normally; the header strip (which already shows the resolved base, if any) is the only signal that base-scoped data is unavailable.

### Tests

Update `src/ui/mod.rs` tests:

- Remove `test_render_expanded_mode_shows_no_base_message`.
- Add `test_render_expanded_mode_renders_groups_without_base`: state has Changes + New files, `merge_base` is `None`, `initial_seed_done()` is true → assert rendered output contains "Changes" and "New files" group headers and does NOT contain "Committed" or "needs a base branch".

## Part 2: Base resolution adds two tiers

### New priority order

`resolve_base_branch(explicit_override)` returns `Some(branch_name)` from the first tier that yields a result; otherwise `None`.

1. **Explicit override** — `explicit_override` argument (already merged by caller from `--base` flag and `base_branch` config). *Unchanged.*
2. **Worktree fork point** — read the first line of `<common-git-dir>/logs/refs/heads/<current-branch>`. Parse `branch: Created from <X>`. Reject `X` if it equals `HEAD`, is a hex SHA (length 7-40, all hex), or equals the current branch name. Strip `refs/heads/` and `refs/remotes/<remote>/` prefixes to get a short name.
3. **Main worktree HEAD branch** — read `<common-git-dir>/HEAD`. Parse `ref: refs/heads/<name>`. Skip if `name` equals the current branch name. (Detached HEAD in the main worktree returns `None`, falls through.)
4. **`origin/HEAD`** — read `refs/remotes/origin/HEAD` from common git dir, parse `ref: refs/remotes/origin/<target>`. *Unchanged from today.*

### Scope

Applies in both main and linked worktrees. Tier 3 self-skips when the candidate equals the current branch, which is the common case in the main worktree on trunk — so trunk behavior in the main worktree is unchanged.

### Stacked branches

Tier 2 is honored literally. `git worktree add -b stacked feature1` records `feature1` as the fork point; perch diffs `stacked` against `feature1`. Users who want trunk anyway can pass `--base`.

### Implementation

In `src/git/mod.rs`:

- Restore `parse_created_from(line: &str) -> Option<String>` and `is_hex_sha(s: &str) -> bool` helpers from the pre-c2216b2 implementation. These are pure parsing functions with unit tests already established in the deleted code's history.
- Restore `reflog_first_created_from(&self, branch: &str) -> Option<String>` on `GitRepo` — reads `<common>/logs/refs/heads/<branch>` first line and applies `parse_created_from`. Reject if extracted name equals `branch`.
- Add `main_worktree_head_branch(&self) -> Option<String>` on `GitRepo` — reads `<common-git-dir>/HEAD`, parses `ref: refs/heads/<name>`. Returns `Some(name)` for symbolic refs, `None` for detached HEAD or unreadable file.
- Update `resolve_base_branch` to walk the four tiers in order. Each tier returns `Option<String>`; the first `Some` wins. Tier 3 must skip if `name == self.branch_name()` to avoid resolving the current branch as its own base.

Skipped from the pre-c2216b2 code: the `head_reflog_parent` function (HEAD-reflog `checkout: moving from` correlation with timestamp + SHA matching). It existed to recover the parent for `git checkout -b foo` *without* a start-point, where the branch reflog says `Created from HEAD`. Worktrees almost always specify a start-point (`git worktree add -b foo trunk`), so this complexity is not justified for the worktree case. Users hitting this edge case can pass `--base`.

### Tests

In `tests/base_detect.rs` (and `src/git/mod.rs` unit tests for parsing helpers):

- **Parsing helpers** (unit tests in `src/git/mod.rs`):
  - `parse_created_from` accepts `branch: Created from main` → `Some("main")`.
  - Rejects `branch: Created from HEAD`, raw SHAs, and empty target.
  - Strips `refs/heads/` and `refs/remotes/<remote>/` prefixes.
  - `is_hex_sha` accepts 7-40 hex; rejects shorter, longer, and non-hex.

- **Tier 2 — worktree fork point** (integration tests with real git repos via `tempfile`):
  - `git worktree add -b feature main` in a repo with `main` → resolution returns `Some("main")`.
  - `git worktree add -b stacked feature` → resolution returns `Some("feature")` (stacked branches honored literally).
  - `git checkout -b foo` with no start-point (reflog says `Created from HEAD`) → tier 2 returns `None`, falls through.
  - Branch reflog whose extracted name equals current branch → returns `None`.

- **Tier 3 — main worktree HEAD** (integration tests):
  - On a feature branch in a linked worktree, with `refs/remotes/origin/HEAD` deleted → resolution returns the main worktree's trunk branch name.
  - On trunk in the main worktree → tier 3 self-skips, falls through to tier 4.
  - Main worktree in detached HEAD → tier 3 returns `None`, falls through.

- **Priority order** (integration test):
  - Repo with all four tiers populated and distinct values → explicit override wins.
  - With override absent but fork point present → fork point wins over main-worktree HEAD and origin/HEAD.
  - With override + fork point absent → main-worktree HEAD wins over origin/HEAD.

### Docs

Update `CLAUDE.md` and `README.md` to describe the four-tier resolution. Frame the new tiers as *recorded git facts* (reflog entries, HEAD ref contents), distinguishing them from the `main`/`master` name guessing that c2216b2 specifically excluded. The "strict, no guessing" principle still holds — perch never invents a base branch name; it only reads ones git has recorded.

## Out of scope

- Network calls (e.g. `git ls-remote --symref origin HEAD`) to discover `origin/HEAD` when missing locally.
- Auto-setting `refs/remotes/origin/HEAD` via `git remote set-head origin --auto`.
- Restoring the HEAD-reflog `checkout: moving from` parent correlation. See "Implementation" note above.
- Surfacing the resolved base tier in the UI (e.g. "base: main (origin/HEAD)" vs "base: main (fork point)"). The header already shows the base name; the tier is implementation detail.
