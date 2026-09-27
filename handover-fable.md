# Shep desktop: Fable restart handover

Written 9 September 2026, 20:45 BST, after both Codex sessions stopped on
`You've hit your usage limit` at 16:01. This file is the restart plan for the
**desktop** session: worktree `shep.so`, branch `main`. The mobile session runs
alongside in `../shep-clients` on `feat/mobile-web-clients`, and the website
session in `../shep-website` on `feat/promo-website`; each has its own
`handover-fable.md`. Same repository, three worktrees.

**Decision from Sam, 9 September 21:00:** `feat/mobile-web-clients` merges
into `main`. Once that lands, `main` is the only integration branch for all
three sessions. The sequence and the rules for sharing `main` afterwards are
in "Merging mobile into main" below. Until it lands, do not merge either branch
into the other.

`handover.md` is Codex's last detailed stopping point and stays useful for
context. `TODO.md` is the authoritative request list. `AGENTS.md` owns the
operational rules and is not repeated here.

## Standing instructions from Sam

- Use parallel agents in isolated worktrees, all the way through. This
  overrides "No subagents are authorized" in `handover.md`.
- First consolidate: every existing lane gets merged into `main` and pushed
  before any new lane starts. After that, merge lanes into `main` as they
  complete rather than batching.
- Agents use the same model as the primary session. Never set a model
  override on the Agent tool.
- Pushing to `main` is authorised. Confirm once at the start with a list of
  what will be pushed, then push routinely.
- No Claude co-author trailers. Conventional Commits, hooks on.

## State on 9 September, 22:10 (phase 1 complete)

Verified directly with git by the Fable desktop session.

**`main`** is pushed as `c414227` and equals `origin/main` (Sam confirmed the
push at 22:00). On top of `1c0fc45` it carries `c35e2b5` (backup history,
encrypted export, ranked scratch: the interrupted cherry-pick, resolved),
`42c69b4` (bounded backup journal owner), `61c6dfc` (remote account-removal
reviews), `c0ebf3f` (duplicate-address sidebar labels) and two docs commits.
Every merge ran the full hook suite on the merged tree (932 to 939
executions); 84 Python tests, strict docs and 31 + 18 native scenarios pass.
Locally, `main` additionally has the aggregate folder account chooser merge
(`1abc4b9` via `fd67cae`), **not pushed**: the mobile session started merging
`c414227` into `feat/mobile-web-clients` at 22:03 and all desktop pushes are
held until that branch is merged into `main` (see "Merging mobile into main").

**Lanes.** Every `codex/*` branch except the three shared harness branches was
classified by three read-only agents against `main` by code and tests.
Integrated or superseded and deleted with their worktrees: `profile-links`,
`profile-reviews`, `profile-cache`, `profile-account-reviews`,
`bounded-folder-delete`, `aggregate-folder-choice`, `combined-folder-delete`,
`folder-convergence`, `compact-mail`, `backup-history`, `backup-formats`,
`backup-all`, `ftp-backups`, `sftp-backups`, `multiple-backups`,
`encrypted-cache`, `native-tray`, `s3-before-main-adaptation`,
`native-palette-before-rebase-1854c62`. The `native-tray` rebase was aborted
rather than finished because its output was byte-identical to `c35e2b5`.
Still to delete once their merges are pushed: `codex/backup-journal-owner`,
`codex/profile-account-removals` (worktree `profile-cache`),
`codex/sidebar-account-labels` (worktree `folder-convergence`),
`codex/aggregate-folder-choice-v2`, and the four `worktree-agent-*` branches
under `.claude/worktrees/`. Keep `codex/profile-core-encrypted-open` and
`codex/profile-initialization-harness` (on origin; mobile consumes them).
`codex/profile-catalog-harness` was superseded by the mobile session and its
local branch/worktree are already gone; the origin copy stays until Sam decides.

**Phase 2 has started and is paused by the rate limit (10 September, 00:05).**
Sam's message at 21:35: "The first thing I want you to fix if not already done
is sort out the closing of the app/blocking saving." That is recorded in
`TODO.md` under R90. Three lane agents were spawned and all three were
terminated at about 23:55 by "You've hit your session limit, resets 1:10am";
their UNCOMMITTED work is intact in locked worktrees (do not prune or reset):

| Worktree (`.claude/worktrees/`) | Lane | Base | State when cut off |
|---|---|---|---|
| `agent-a92937f9fa561a16d` | Close blocked by saving (R90/R86) | `c414227` | 8 files edited (`src/lifecycle.rs`, `src/notifications.rs`, `src/ui/closing.rs`, `src/ui/mod.rs`, `src/ui/tray.rs`, `scripts/e2e.py`, `scripts/mcp_harness.py`, `tests/support/backups.rs`); was about to build test-ui and run native close scenarios |
| `agent-a77706227ce2ff3d2` | Encrypted cache guarded publication/recovery (R22) | `64437ee` | new `src/cache_cipher/publication.rs` + `publication/`, edits to `cache_cipher.rs`, `migration.rs`, docs; full test suite was running, completion/audit drafts in progress |
| `agent-adfeba145fe019aee` | Post-enrollment account linking (R02/R49) | `64437ee` | 10 files across `src/profile_sync`, `src/store/profile_sync`, `src/engine/profile_sync.rs`, `src/ui/profile_sync.rs`; was adding observation JSON for native scenarios |

**10 September, 09:00 update.** The mobile session pushed `d8d5c1e`; the
desktop session merged it into `main` as `ee1305c` (only `TODO.md`
conflicted; the client's combined list was taken and desktop receipts
re-applied), ran the gates (1066 hook executions, 96 Python, strict docs, 45
native scenarios) and pushed. `main` is the single integration branch now; the
mobile worktree is reset onto it. Rules from here: `git fetch origin && git
rebase origin/main` before every push, rerun affected gates, never force-push.
The three paused lane agents were resumed at 09:00 with the instruction to
merge `main` first (the workspace now includes the shared crates and the
tracking docs are combined lists), then finish verification and commit.

**10 September, 10:40 update.** Two of the three lanes are merged and pushed:
`origin/main` is `c146e55`, carrying encrypted cache publication with
journalled crash recovery (`eb1a5cb`, R22; activation blockers listed in
completion) and the app-close fix (`c146e55`, R90/R86: explicit Quit leaves
journaled backup/credential work and exits, hidden-tray Quit announces saving,
exit is bounded to five seconds when a blocking task stalls). Gates on the
final tree: 1084 hook executions, 96 Python, strict docs, 41 native scenarios.
**10 September, late update (Opus 5).** Fable credits ran out; Sam switched
the session to Opus 5, so new agents run on Opus 5. Sam reported that
`scripts/install-linux.sh` no longer compiled: the release build uses no
features, and the native test-state snapshot in `src/ui/mod.rs` read four
`observation()` methods and the `inbox_reveal_height` field that only exist
with `test-support` (introduced by `488a9ec`/`93d4289`/`3927053` and
`15a4a3c`; hooks only built with `--all-features`). Fixed in `372fe9b`, which
also adds `cargo clippy --no-default-features -- -D warnings` to the shared
pre-commit hook. The three unfinished lanes were relaunched on Opus 5: the
incremental pull and cache bootstrap lanes finish in their existing worktrees
(`agent-a8d052821715bfeee`, `agent-a9e5c204996d077db`) and large mail (R23)
starts fresh. Sam asked to push everything when done.

**11 September, 00:05.** `origin/main` is `1ec7654`. Pushed since the
compile fix: `9ea8ad1` (merge of the mobile browser-profile commits; its first
commit attempt was refused only by the commit-msg hook because Git's default
"Merge remote-tracking branch" subject is not Conventional, so always pass
`-m` with a Conventional subject), `ca28e69` (incremental enrolled profile
pulls through the catalog change token, with full-listing fallback; the lane's
first commit had never declared its module, fixed in `b531d09`), and
`1ec7654` (guarded encrypted cache bootstrap: one worker holds the exclusive
guard through key lookup, recovery and conversion, then a shared guard that
catalog, store and journal workers keep until admitted writes finish; still
unencrypted unless a folder already has a key). Each lane merged `main` first,
so the merge tree was byte-identical to the hook-verified lane tip. The R23
large-mail lane was relaunched twice by mistake: `stat` without `-L` on the
task output files measures the 151-byte symlink, not the transcript, so every
agent looked frozen. Use `stat -L` (or the `subagents/*.jsonl` target) to judge
liveness. The first agent had in fact gone quiet at 23:40 without reporting;
its worktree was removed and it was stopped. The lane now runs in
`artifacts/worktrees/large-mail` on branch `lane/large-mail` from `1ec7654`. Remaining activation blockers for R22 are
listed in the newest completion entry.

**11 September, 15:30: everything merged.** `origin/main` is `16a52fd`. Sam
asked to merge everything: the three superseded shared profile-core harness
branches are recorded as merged with an `ours` merge (`f6ec923`, content
verified line by line, no file changed), and the mobile session's Flutter group
actions lane was finished, verified (flutter analyze, 153 Flutter tests, 97
Flutter Rust tests, 4 Playwright and 3 Android plus 4 Appium scenarios, 1179
root hooks) and merged (`01569b5`). No local branch, worktree or uncommitted
change is unmerged. At Sam's request the fully merged remote
branches `codex/profile-*`, `feat/mobile-web-clients` and `feat/promo-website`
were deleted from GitHub; `main` is the only remote branch. The local
`feat/mobile-web-clients` and `feat/promo-website` branches stay because the
mobile and website worktrees have them checked out (their upstream now shows
as gone); those sessions push with `git push origin HEAD:main`.

**11 September, 13:30.** `origin/main` is `286b4ba`. Sam decided Google-only
protection for synced passwords and asked for a Sign in with Google button;
both are recorded (TODO R49, R75, the shared handover's credential section)
and shipped on desktop: `a75cc4d` (button, built-in Desktop OAuth client from
`SHEP_GOOGLE_CLIENT_ID`/`SECRET` at build time; disabled until Sam creates the
client, steps in `docs/agents/google-sign-in.md`) and `835cfb6` (opt-in
password vault: two app-data files outside the causal history, AES-256-GCM,
staged keychain import after a connection test; shared-crate `vault` feature
with golden fixtures for Flutter). New requests recorded today: R95 (highlight
text in formatted HTML), R96 (reply bar always visible), R97 (previous-thread
preference), R98 (help ? tooltips), R99 (settings search covers every
setting), plus follow-ups under R35 (newest-first conversation in the reply
pane), R61 (make the README release one-liner work; confirm before enabling
the release workflow), R64 (GNOME colour inversion, tray icon too small, tray
colours to match) and R86 (close-to-tray on by default), and R15/R17/R21
(History button as an icon, light-theme checkbox visibility). The mobile and
website sessions are not running; the next free request number is R100. No
desktop lanes are running.

**11 September, 09:40.** `origin/main` is `00328f0`. Pushed since `1ec7654`:
`10d6877` (R23 reader paging: long plain-text messages load 32,000 characters
at a time with Show more; `Command::Detail` carries a `body_chars` budget and
refreshes keep the loaded length; 1135 hook tests, 49 native reader scenarios,
new `long_mail` fixture) and `00328f0` (merge of the website session's commits
and Sam's Windows CI workflow `6ce29de`). R23 still has the 25 MiB incoming
skip, the 256 MiB restore limit and the outgoing limit audit open, with the
technical reasons in TODO. The website session owns R94; new desktop requests
start at R95. No lanes are running.

**10 September, 12:40 update.** `origin/main` is `1d86831`: Google lifecycle
and token ownership through bounded workers (R91, `f4575b2` via merge; 1101
lane hook executions, full gate set on the merge: 1029 workspace tests, 96
Python, profile-core script, docs, 22 native scenarios). Two lanes were cut
off by the session limit (resets 13:00) and are locked with uncommitted work,
both based on `924141d`: `agent-a8d052821715bfeee` (incremental change-token
profile pulls, R02/R49; 13 files; was updating a test helper) and
`agent-a9e5c204996d077db` (encrypted cache bootstrap routing, R22; 23
files). Resume each by messaging its id: merge `main` first, finish
verification, commit with hooks, report. The mobile session pushed `09104db`
and `3859656` (client trees only) and noted that rebasing a docs-touching
commit can silently drop the other side's tracking hunks; diff after any
rebase.

The post-enrollment account linking lane landed as `924141d` (1091 hook
executions, 96 Python, docs, 17 native profile scenarios) and is pushed; all
three resumed lanes are merged and their worktrees removed. Pitfall found: `git rebase origin/main` flattens local
`--no-ff` merge commits; when local `main` carries merges use `git merge
origin/main` before pushing. Worktrees `agent-a08c…` and `agent-a7bff…` under
`.claude/worktrees/` belong to the mobile session; leave them.

**Coordination.** Peer sessions: `shep-clients-0b` (mobile, all merge handoffs
go there), `shep-website-7b` (website, gets the commit after the mobile merge),
`shep-so-52` (coordinator, status only). The mobile session reported that the
repo-level `core.hooksPath` points at this worktree's `.githooks`, that its
branch moved `src/providers/mail/` into `shared/mail-core`, and that it will
reconcile the pre-commit hook as the union of both during its merge; verify the
merged hook runs both sets when rerunning the gates, otherwise send it back.

## State on 9 September, 20:45 (historical, superseded above)

Verified directly with git; do not trust it blindly after the first turn.

**`main`** is `1c0fc45` and equals `origin/main`. The working tree holds an
interrupted cherry-pick of `895fe2d` (tip of `codex/backup-history`, "feat:
retain backup activity and recovery across restarts"): 36 files are staged
cleanly and there are three conflict hunks, two in `TODO.md` (around lines 13
and 40) and one in `src/store.rs` (around line 210). `CHERRY_PICK_HEAD` is
gone but `.git/MERGE_MSG` and `AUTO_MERGE` remain, so `--continue` may refuse.
Resolve the hunks, `git add`, and commit with the subject from `MERGE_MSG`.

**Lane worktrees** under ignored `artifacts/worktrees/`:

| Worktree | Branch | State |
|---|---|---|
| `native-tray` | detached at `1c0fc45` | Interactive rebase of `codex/encrypted-cache` onto main. `f0ff98a` applied with 13 files staged and a conflict in `scripts/release.py`; `8d45a9e` still to pick. This is the encrypted-cache lane despite the directory name. |
| `native-tray/artifacts/profile-core-cache` | `codex/profile-core-encrypted-open` | Clean. Published shared initialiser API `8588c21`; desktop consumes it as an immutable revision. |
| `profile-cache` | `codex/profile-account-removals` | 15 uncommitted files (profile docs, skill, `AGENTS.md`, `TODO.md`, tracking docs). Commit them on the lane first. |
| `folder-convergence` | `codex/sidebar-account-labels` | 4 uncommitted files: `TODO.md`, `scripts/e2e.py`, `src/ui/folder_tests.rs`, `src/ui/sidebar.rs`. Branch itself adds nothing to main. |
| `multiple-backups` | `codex/backup-journal-owner` | Clean, 8 commits ahead of main, latest lane (16:58). |
| `compact-mail` | `codex/compact-mail` | Clean. |
| `profile-catalog` | `codex/profile-catalog-harness` | Clean. Rooted on the **mobile** branch, not main. Leave it to the mobile session. |
| `profile-initialization` | `codex/profile-initialization-harness` | Clean. Merges into main without conflict (27 files, all additions). |

`../shep-website` on `feat/promo-website` has three modified tracking files and
an untracked `website/`. It is nobody's active lane; leave it alone.

**Local `codex/*` branches**, all rooted on main unless noted. Every one of
them conflicts with main in a trial merge, almost always on the same tracking
files (`TODO.md`, `docs/COMPLETION.md`, `docs/REQUEST_AUDIT.md`, `AGENTS.md`,
`.agents/skills/shep-e2e/SKILL.md`, `README.md`, `Cargo.lock`). Codex
integrated lanes by adapting and cherry-picking, then committing
`feat: integrate ...` on main, so ancestry says nothing about whether a lane's
behaviour is already on main. Judge by code and tests, not by `git branch
--merged`.

| Branch | Tip | Last commit | Likely status |
|---|---|---|---|
| `codex/backup-journal-owner` | `d36dca8` 16:58 | refactor: own backup journal state through bounded channels | Open, newest backup work |
| `codex/backup-history` | `895fe2d` 16:48 | feat: retain backup activity and recovery across restarts | Being cherry-picked into main now |
| `codex/backup-formats` | `7ef6c56` 16:12 | feat: choose backup compression and encryption per destination | Open |
| `codex/encrypted-cache` | `8d45a9e` 16:53 | perf: rank conversations in encrypted indexed scratch | Being rebased in `native-tray` |
| `codex/aggregate-folder-choice-v2` | `1abc4b9` 16:54 | feat: choose accounts for common folder actions | Open |
| `codex/bounded-folder-delete` | `9b300af` 16:21 | fix: bound folder deletion count snapshots | Open |
| `codex/aggregate-folder-choice` | `6d7c392` 15:20 | fix: reconcile combined folder deletion immediately | Same tip as `combined-folder-delete`; probably superseded by `-v2` |
| `codex/combined-folder-delete` | `6d7c392` 15:20 | same as above | Same tip as `aggregate-folder-choice` |
| `codex/profile-account-removals` | `18413e7` 16:13 | feat: review shared account connections without replacing local mail | Same tip as `profile-account-reviews`; `1c0fc45` on main says "account reviews" were integrated |
| `codex/profile-account-reviews` | `18413e7` 16:13 | same as above | Duplicate of the line above |
| `codex/profile-links` | `59c6cf0` 13:50 | feat: link matching local accounts during shared profile import | `handover.md` says integrated in `4e75005` |
| `codex/profile-reviews` | `713f96e` 12:37 | feat: review shared preference conflicts with durable choices | `handover.md` says integrated |
| `codex/profile-cache` | `654a90e` 10:30 | perf: reuse verified shared profile records across syncs | Check |
| `codex/backup-all` | `c515b80` 15:31 | feat: back up included destinations with independent recovery | Integrated as `cf76976` |
| `codex/ftp-backups` | `17de158` 14:59 | feat: add recoverable FTP and FTPS backup destinations | Integrated as `13f9c36` |
| `codex/sftp-backups` | `f60dac0` 14:19 | feat: add verified SFTP backup destinations | Integrated as `ff03b02` |
| `codex/multiple-backups` | `878e9f9` 13:34 | test: preserve shared settings during S3 setup saves | Integrated as `4e75005` |
| `codex/s3-before-main-adaptation` | `b78a70f` 13:24 | feat: add S3-compatible backup destinations | Snapshot before adaptation; superseded |
| `codex/folder-convergence` | `66e6cb8` 14:44 | fix: reconcile combined folder choices and filtered unread counts | Integrated as `97c9a9a` |
| `codex/compact-mail` | `279a72a` 13:55 | fix: retain conversation anchor after moving an expanded reply | R89 integrated as `91ed9a9`/`15a4a3c`; check this later fix |
| `codex/native-tray` | `a6dc0c7` 13:40 | fix: include palette edits in the global settings save | Tray/badges/installers integrated per `handover.md`; check this fix |
| `codex/native-palette-before-rebase-1854c62` | `1854c62` 13:07 | feat: add editable light and dark color palettes | Snapshot; superseded |
| `codex/profile-core-encrypted-open` | `e3e69a4` 14:53 | fix(sync): release owned file locks after database teardown | Published shared API; on origin |
| `codex/profile-initialization-harness` | `43cdcf0` 07:19 | test(sync): package shared initialization protocol with native harness | Clean merge; on origin |
| `codex/profile-catalog-harness` | `33d222d` 05:43 | test(sync): expose owned loopback catalog transport | Mobile lane; not yours |
| `codex/sidebar-account-labels` | `1c0fc45` | equals main | Nothing to merge; delete |

Only `origin/main` and the three `codex/profile-*` harness branches exist on
the remote; every other lane is local only. Losing this machine loses them.

## Phase 1: consolidate and push

The primary session does the serial work on `main`. Agents do the parallel
analysis and conflict resolution in their own worktrees. Nothing starts in
phase 2 until `main` is pushed and every lane is either merged or deliberately
deleted.

1. **Finish the two interrupted operations before anything else.** Resolve the
   cherry-pick in this worktree and commit it. In `artifacts/worktrees/native-tray`
   resolve `scripts/release.py`, `git rebase --continue`, and pick `8d45a9e`,
   leaving `codex/encrypted-cache` rebased onto main. Run the mandatory hooks
   on both. If either resolution is unclear, an agent in a scratch worktree can
   compare both sides against `docs/agents/MULTIPLE_BACKUPS.md` and
   `docs/agents/CACHE_ENCRYPTION.md`, which describe the intended behaviour.
2. **Commit every dirty lane on its own branch.** `profile-cache` and
   `folder-convergence` carry uncommitted work; a `wip:` commit that passes
   hooks is fine. Never discard uncommitted work in any worktree.
3. **Classify the lanes in parallel.** One read-only agent per branch (or per
   group of duplicates) answers: is this behaviour already on `main`, is it
   superseded by another lane, or does it carry unique work? For unique work,
   list the code and test files involved. Duplicates with identical tips need
   one answer. Record the verdicts in this file's table.
4. **Bring open lanes onto main, one agent per lane, in its own worktree.**
   Rebase or merge `main` into the lane and resolve conflicts. For the tracking
   files take `main`'s version and re-add the lane's own entries; never take a
   lane's `TODO.md`, `docs/COMPLETION.md`, `AGENTS.md` or `SKILL.md`
   wholesale, because each one is a snapshot of a different moment. Run the
   pre-commit hooks and the native flows the lane touched. Report the commit.
5. **Integrate serially.** Merge each ready lane into `main` with
   `git merge --no-ff`, rerun `cargo fmt --check`, Clippy, `cargo test
   --all-features`, the Python tests and the relevant `scripts/e2e.py`
   selection, then push. One lane at a time; never queue two merges. Suggested
   order: `encrypted-cache`, `backup-history` (already in flight),
   `backup-journal-owner`, `backup-formats`, `aggregate-folder-choice-v2`,
   `bounded-folder-delete`, `profile-account-removals`, then whatever the
   classification says is still unique from the older lanes.
6. **Delete what is integrated or superseded.** `git branch -D` the branch,
   `git worktree remove` its worktree, `git worktree prune`. Keep the
   `codex/profile-*` harness branches that the mobile side consumes.
7. **Update `TODO.md`, `docs/COMPLETION.md` and `docs/REQUEST_AUDIT.md`** for
   everything that landed, commit, push, and replace the state section above
   with the new truth.

Keep each worktree's Cargo target separate (the default `target/` inside the
worktree is fine) and cap builds with `CARGO_BUILD_JOBS=4`. The mobile session
is building on the same machine; expect contention and do not relax any
timeout or budget because of it.

## Merging mobile into main

`feat/mobile-web-clients` is 108 commits ahead of `main` and 89 behind, from
merge-base `10d8569`. It carries `flutter/`, `web/`, `backend/`, `shared/` and
`website/`, plus its own ports of some desktop changes. The mobile session
does the hard half on its branch; you do the easy half on `main`. Find the
peer sessions with ListAgents and use SendMessage at each handoff.

1. You finish phase 1 and push `main`. Tell the mobile session the commit.
2. The mobile session finishes its own phase 1, then merges `main` into
   `feat/mobile-web-clients` in a scratch worktree, resolving conflicts by area
   with agents, runs both gate sets, pushes the branch, and tells you the
   commit. **From the moment it starts that merge until you have merged the
   branch, do not push anything new to `main`.** Keep integrating lanes locally
   if you like, but hold the push; every commit you add to `main` in that
   window is another conflict round for the mobile session.
3. You `git merge --no-ff feat/mobile-web-clients` into `main`. It should be
   conflict-free because the branch already contains `main`; if it is not, the
   mobile session pushed a stale merge, so send it back rather than resolving
   client-side conflicts yourself. Rerun the desktop gates; the root workspace
   may now include client crates, so expect `cargo test --all-features` to
   take longer. Push. Tell the mobile and website sessions the commit.

   Layout change to expect: the branch moved `src/providers/mail/` into
   `shared/mail-core`. Your `a81d767` and `5eabb52` folder-journal and
   move-recovery changes arrive ported into that crate, behaviour unchanged.
   After the merge, `src/providers/mail` no longer exists; lanes that would
   have touched it target `shared/mail-core`. Until your phase 1 push, avoid
   widening changes under `src/providers/mail`, since each one is more porting
   for the mobile session.
4. Delete `origin/feat/mobile-web-clients` only after the mobile session
   confirms it has reset its worktree onto `main`.

**After the merge, three sessions push to `main`.** Rules for you:

- Before every push: `git fetch origin && git rebase origin/main` (or merge if
  a rebase would rewrite a published merge commit), rerun the gates that the
  incoming changes could affect, then push. A rejected push means fetch,
  merge, rerun, retry. Never force-push `main`.
- Do not sit on finished work; the longer `main` diverges between the three
  worktrees, the worse every rebase gets.
- The other two worktrees keep their branch names (`feat/mobile-web-clients`,
  `feat/promo-website`) only because git cannot check out `main` twice. They
  push with `git push origin HEAD:main`. Treat those branch names as "main
  plus one lane", never as long-lived feature branches again.

## Phase 2: work the TODO with agents, merging as you go

Priorities are in `TODO.md`; the ordering Codex left is in `handover.md`
under "Next work". In short: the continuous OAuth/Drive profile sync loop is
first (local capture and admission ahead of remote pulls, per-field
application, dirty-UI preference merges, account linking, conflict and removal
reviews, incremental pulls), then the open desktop items: native tray platform
verification (R86), slow close dependencies (R90), channel ownership (R91),
encrypted cache, large mail, backup formats and history, combined folder
actions, installers and release assets, and the final idle-host performance
pass.

How the loop runs:

- Pick independent lanes that touch different modules. Two agents editing
  `src/store.rs` at once is a merge you will pay for later.
- Spawn each lane with the Agent tool and `isolation: "worktree"`. Give the
  agent the exact TODO entry, the relevant `docs/agents/` page, the tests it
  must add, and the rule that it commits on its branch and never pushes.
- Run at most three lanes at once. Cargo, Flutter and the native harness all
  share this host with the mobile session.
- When a lane reports done, verify its evidence yourself (commit, hook output,
  native screenshots where it touched UI), merge it into `main` the same way
  as phase 1, rerun the gates, push, delete the branch and worktree, and update
  the tracking docs. Then spawn the next lane. Do not let finished lanes pile
  up unmerged.
- Isolated correctness-only native flows can run alongside builds. Latency and
  renderer pixel measurements need a quiet window; defer them to the end.
- Every turn ends with `main` clean, pushed, and the tracking docs current.
  If a merge cannot be finished in the turn, abort it rather than leave it.

## Gates before every push

From `AGENTS.md` "Build and quality gates": `cargo fmt --all -- --check`,
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo test
--all-features`, `python3 scripts/test_profile_core.py`, `python3 -m unittest
discover -s tests -p 'test_*.py'`, the relevant `python3 scripts/e2e.py`
selection through the `shep-e2e` MCP skill, and
`artifacts/docs-venv/bin/zensical build --clean --strict` when docs changed.
Windows GNU cross-compilation where the lane touched platform code. Quality
and release CI stay disabled until Sam's runners are ready; documentation CI
is on.

## Do not

- Merge `main` and `feat/mobile-web-clients` in any order other than the one
  in "Merging mobile into main", or push to `main` during the merge window.
- Force-push `main`, ever.
- Touch the installed personal app, personal mail, credentials, Google data or
  Sam's phone. The native harness uses its own fixtures and display.
- Share a Cargo target directory between worktrees with different vendored
  renderer sources.
- Skip or weaken a hook, timeout or budget.
- Use a sibling worktree path as a shipped dependency; consume immutable
  published revisions of the shared crates.
- Claim live Google, IMAP, SMTP, Windows or macOS verification from fixtures.
