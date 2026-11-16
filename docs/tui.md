# Terminal workflow

Shepherd uses the same Governance hierarchy wherever it runs:

- Workspace: a Governance team, such as DevOps or CMU Housing.
- Project: a project declared under that team; it can own several repositories.
- Task: a GCD issue. Pull requests are related changes and a separate source view.

Governance discovery refreshes on startup. Choose a workspace with `w` and a project with `P`. `r` selects a repository within the project; Tab cycles those repositories. Projects without repositories remain visible. `G` opens the canonical Governance declaration; workspace/project definitions are maintained there rather than duplicated locally.

The list keeps task details beside the selection when the terminal is wide enough. Enter expands detail. Narrow terminals preserve readable task titles; `v` switches to kanban. High-priority issues sort first. Selection uses a uniform title-line band; metadata remains outside it. All colors come from the terminal's background, foreground, and ANSI palette. `NO_COLOR` is respected.

| Key | Action |
| -- | -- |
| Arrows or h/j/k/l | Change stage or selected task |
| 1-5 | Jump to a stage |
| Enter / Esc | Expand detail / return |
| Page Up / Page Down | Scroll detail or page tasks |
| u | Switch All accessible / My work |
| w / P / r | Workspace / project / repository picker |
| Tab / Shift+Tab | Next / previous scoped repository |
| / | Live search; Enter keeps it, Esc restores the previous search |
| f | Cycle issues, PRs, all shared sources, unsent drafts |
| v | Task list / kanban |
| n | Create an issue on GCD; Tab chooses its target repository |
| e / d | Edit title / multiline description on GCD |
| o | Set GCD assignees using comma-separated logins |
| p | Set shared priority through existing labels |
| m | Change shared stage; Done closes the issue on GCD |
| c / C | Write a multiline issue or PR comment through Collar; Ctrl+S posts |
| z | Close or reopen an issue or unmerged PR through Collar; Enter confirms |
| O | Open the issue or PR in the browser |
| s | Refresh current scope from GCD |
| g / G | Refresh Governance / open its declaration |
| N | Save an unsent local draft |
| b | Export a task brief |
| a / A | Agents view / launch Codex or Claude Code for the selected task |
| ? / q | Show actions / quit |

Editors support Left, Right, Home, End, Delete, and Backspace. Enter saves titles, assignees, and priorities. In descriptions and comments, Enter inserts a newline; Ctrl+S saves or posts. Comments use `collar issue comment` or `collar pr comment` with literal stdin. Close/reopen uses the corresponding typed Collar command. Closing a PR does not merge it; merged PRs cannot be reopened. The dialog identifies the remote write. Edits compare the inspected remote issue with its current state before sending; stale edits fail. Writes are never retried automatically. If a response is uncertain, inspect the issue on GCD before trying again. Ordinary quit waits for a pending write; Ctrl+C force-exits and its remote write may already have completed.

Stages follow repository labels such as `stage/ready`, `stage/in-progress`, and `stage/in progress`. Closed/merged resources are Done. Without a stage label, assigned issues appear active and issues with a directly closing open PR appear in review. Repositories without the requested shared workflow label show an actionable error instead of saving a misleading local override. Priority honors existing `priority/*` and `urgency/*` labels. Shepherd does not provision labels or change Governance settings.

Agent runs support planning, isolated implementation, live activity, stop, native-session resume, diff review, and explicitly published issue comments. See the [agent workflow](agents.md). Local run history is labeled separately from shared GCD results.

## Cache and local drafts

The user-level cache contains private repository content; keep it private. It is a snapshot, not a second project database. `--offline` reads that snapshot and disables remote writes. A failed refresh leaves previous data intact and reports the error. Missing permissions are not converted to empty projects. Unavailable repositories remain in Governance, their cached tasks are retained, and the refresh status reports how many other repositories succeeded. Press `!` for access guidance and the original Collar error.

`N` creates a local draft, shown only in the explicit draft view. Legacy local stage/owner/priority overlays are retained as `localPlan` notes when a shared issue refreshes, while its displayed fields come from GCD. Local agent claims and proposal notes are not cross-machine coordination. Accepting a cached proposal creates unsent drafts; it does not publish multiple issues implicitly.

The GCD API does not provide an atomic compare-and-swap for these edits. Shepherd checks the inspected fields before writing and rechecks immediately before label replacement, but another client can still race that final request.

## Local directory projects

Press `+` to add an existing directory. Relative paths and `~/` are accepted. It appears under **Local projects** in the workspace picker. No Git repository, Collar login, or Governance declaration is required. Select a project with `P`, then use `n` to create tasks; title, description, owner, priority, and stage edits save locally. These tasks are labeled LOCAL and appear in the ordinary task view.

`A` launches an agent using that project's registered directory. Plan works in plain directories. Implement requires a committed Git root and uses a separate worktree. Local tasks have no GCD issue, so result publication is unavailable.

`X` forgets the selected local project after you type `remove`. It keeps files, tasks, and agent history; adding the same canonical directory restores its tasks. Local registrations are machine-specific and survive Governance refreshes. Shared project definitions remain in Governance.

```sh
shepherd local add ~/dev/shepherd --name Shepherd
shepherd local list
shepherd local task LOCAL_PROJECT_ID 'Review the Terrier trial'
shepherd --workspace @local --project LOCAL_PROJECT_ID
shepherd local remove LOCAL_PROJECT_ID
```

## Personal work and accessible projects

The header exposes **All accessible** and **My work**. Press `u` to switch. All accessible preserves the project browser: canonical Governance teams, registered local projects, and account repositories outside Governance under Other accessible repositories. Account discovery uses Collar's authenticated API; it does not provision shared projects.

My work is a single inbox across projects and stages. It includes open issues assigned to the signed-in account, assigned PRs, authored PRs, and requested reviews. Each item shows why it is in the inbox; items in multiple categories appear once. The account is read through Collar, never inferred from the machine username. `s` refreshes this inbox; `f` filters its source types. Offline mode uses the cached identity and assignments and labels the data as cached.

`shepherd inbox` refreshes the same account data without opening the TUI. API reads paginate to completion; failures preserve the previous account snapshot and remain visible. Expanded task detail reads discussion comments through Collar. Ordinary refreshes preserve cached comments.

The keyboard guide groups navigation, task, Collar, agent, and local actions. It uses two columns in wide terminals and one in narrow terminals; arrows and Page Up/Down scroll, and Esc closes it.

In Agents, Enter opens the native conversation, Ctrl+Z returns to Shepherd, and Enter resumes the same process. `v` shows the last automated result. Delete confirms removal of finished history.
