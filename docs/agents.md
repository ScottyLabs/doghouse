# Agent workflows

## Codex and Claude Code

Select a task and press `A`. Type an optional goal and press Enter; an empty goal uses the task itself. Shepherd selects an installed CLI and the project's known checkout. Tab opens secondary settings for the agent, task mode, and directory; Left/Right changes options, Enter returns to the goal, and Esc cancels. Ctrl+S is an alternative launch key. Alt+Enter or Shift+Enter inserts a goal newline where supported by the terminal. Pasting multiline goals does not submit them.

For shared tasks, the checkout must have a Git remote matching the repository. Local tasks use their registered directory. Git-backed issue tasks default to implementation in a separate checkout; PRs and plain directories default to planning. The last available backend is remembered; native CLI model and login settings are retained.

Before starting, Shepherd loads applicable `AGENTS.md` files from the original directory and its parents, in precedence order, and includes them in the run context. The run records which files were loaded. Implementation instructions require reading project documentation, outlining the approach and checks before editing, implementing, running relevant checks, and reporting the result. Agents must honor deeper instructions where they edit.

- **Plan** uses Codex read-only mode or Claude Code plan mode with read tools. It inspects the selected checkout, including local changes.
- **Implement** creates a detached Git worktree from committed HEAD. Your original checkout remains available. New files appear in worktree status; the diff includes tracked changes from the base commit.

Shepherd invokes the installed native CLI with its existing login and model configuration. It does not provision API keys or run a separate agent framework. Both CLI integrations use structured output; Codex uses native automatic approval review for implementation, and Claude Code uses `acceptEdits` with noninteractive permission handling. Denied tools appear as Needs attention. Authentication, version, and permission failures remain visible in activity. Claude Code must support `--permission-prompts none`.

| Sessions key | Action |
| -- | -- |
| Up/Down or j/k | Choose a session |
| Enter / Space | Open its action menu; arrows choose, Enter applies, Esc cancels |
| o | Open the native conversation; working automated turns ask before stopping |
| Right | Open reported subagents; Enter inspects a child, Left or Esc goes back |
| n / A | Start a new session for a task |
| r | Continue with a follow-up goal |
| s / F5 | Refresh session records; never stops work |
| x | Confirm stopping the whole session and its child tools |
| Tab | Switch activity and result previews |
| a / Esc | Return to tasks |
| q / Ctrl+C | Exit Shepherd; sessions keep running |

History, diff, publication review, clean-checkout removal, and history deletion remain available from the action menu and keyboard guide. Action menus scroll to keep the selection visible in small terminals. `H`, `v`, and `P` remain advanced shortcuts; `d` deletes with confirmation.

`o` opens the native CLI in the same terminal. Ctrl+Z returns to Shepherd while that terminal stays open. `o` reattaches, including its in-memory draft. A private tmux terminal preserves the screen; packaged Shepherd includes tmux. Opening an automated turn that is still working asks for confirmation before stopping that turn and opening its conversation.

Every Shepherd window is a client of the same private, per-user session registry, including windows with different `--data-dir` boards. Workers own the agent processes, logs, task locks, and checkout locks. Closing or killing a Shepherd window leaves workers and native conversations running. Any window can open, continue, or explicitly stop a session. Registry records live under `$XDG_STATE_HOME/shepherd/agents`, or `~/.local/state/shepherd/agents`. `SHEPHERD_AGENT_DIR` explicitly selects a separate registry base for testing or administration. Different users and machines remain separate.

Existing sessions are registered from the current board's old `agents/` directory and the previous default user cache. Registration preserves original record, checkout, and native-history paths. A run still owned by an older Shepherd window remains owned there until transferred: opening the conversation asks to stop the old process and reopen its saved conversation in a persistent worker. Registration alone cannot reparent an existing process. With an explicit `SHEPHERD_AGENT_DIR`, only the selected board's legacy sessions are registered; unrelated user caches are not imported.

The list shows the newest turn of each conversation. `H` expands previous turns. Enter opens the conversation; Space opens contextual actions; `o` also opens a conversation, `r` supplies a follow-up goal, and `x` asks before stopping current work. `s` and F5 consistently refresh. Working, Conversation open, Finished, Needs attention, and Interrupted describe distinct states. Selection stays on the same conversation when another window starts work.

After a reboot or worker interruption, Shepherd checks live ownership locks and the actual terminal rather than trusting a stale socket file or PID. An interrupted session offers `o` to review and confirm recovery of its saved native conversation in the existing checkout. No automated goal or uncertain write is replayed automatically. Files and saved session history survive, but in-memory terminal input cannot survive a reboot. If no native session ID was saved before interruption, the checkout remains available and a new session is needed. An orphaned agent that still holds its checkout lock blocks recovery until it exits; this avoids duplicate agents after a worker crash.

After an interactive conversation, the saved automated result is labeled as historical and cannot be published as a fresh result. An automated follow-up with `r` can produce a new publishable result. Native session history remains available through Enter.

`d` (or Delete) confirms removal of Shepherd history and transcripts, stopping active sessions first. Pending publication blocks deletion. Project files, worktrees, saved native conversations, and GCD comments are retained. Clean-checkout removal is available in the action menu.

Up to four automated turns can execute across the shared registry. Local ownership locks prevent another Shepherd window from using the same task or checkout concurrently. Resume retains the original checkout and native session. A changed HEAD must be preserved before starting a new run. Cleanup refuses dirty worktrees or worktrees containing new commits. Stopping a run keeps its changes and result.

New runs, logs, native session IDs, and checkouts live under the shared registry's private `agents/` directory. Legacy worktree paths remain in place. Activity and result previews are bounded; the raw transcript is capped at 8 MiB per run. Token usage is shown only when the CLI reports it. Claude's reported cost is a cumulative CLI estimate for the session, not a billing total.

Agents run on **this machine**. They are not a distributed job service. Governance and GCD provide the shared hierarchy and issues; explicitly published result comments are visible elsewhere and loaded in expanded task detail. Local run state does not assign or move shared issues. Publication is never automatically retried, including after an uncertain response. Inspect GCD before another attempt.

`q`, Ctrl+C, and terminal closure leave sessions running. Use `x` and confirm to stop current work explicitly; stopping retains files and saved conversation history. A worker receiving a handled shutdown signal stops its own process group and records an interruption. Reopening Shepherd after startup offers recovery rather than silently restarting work. Repository instructions and native backend permissions remain in effect.

Integration references: [Codex noninteractive mode](https://learn.chatgpt.com/docs/non-interactive-mode), [Codex commands](https://learn.chatgpt.com/docs/developer-commands), [Claude Code programmatic use](https://code.claude.com/docs/en/headless), and [Claude Code CLI reference](https://code.claude.com/docs/en/cli-reference).

## Subagents

Right or **Subagents** in the action menu opens children reported by the parent CLI. The list includes last reported status, task, and result, and nests descendants where parent identifiers are reported. Enter opens details; arrows and Page Up/Down navigate, and Left or Esc returns. These reports are stored with the parent in the shared registry, so every window sees the same information. Child output cannot overwrite the parent's session ID, result, usage, or completion status.

To steer, stop, or continue one child, press `o` to open the native parent conversation. Codex provides [`/subagents`](https://learn.chatgpt.com/docs/agent-configuration/subagents); Claude Code provides [`/tasks`](https://code.claude.com/docs/en/sub-agents). A persistent native-terminal bar shows the provider's command and **Ctrl+Z: return**. It is refreshed on attachment to existing conversations too. Shepherd does not inject commands into a native draft or open an independent writer on a child's checkout.

The standalone CLI streams do not provide a portable external control API for individually interrupting live children. In an already-open native conversation, these controls act within the existing process. Switching from a working automated turn first requires confirmation to stop that turn and its child tools. `x` in Shepherd is always a whole-session action; it never pretends to stop only the highlighted child. Within the child panel it explains how to reach native controls instead.

Reports are observations, not a separate process-liveness check. Children created in a native interactive conversation are managed live there; Shepherd's stored reports may be older or absent. An empty report list explicitly does not mean there are no children. Captured task and result previews are bounded, with up to 64 reported children; overflow is labeled and the provider's native view remains available. No process scanning, permission changes, or automatic delegation is introduced.

## Shared tasks and local drafts

Use `shepherd discover` and `shepherd --workspace TEAM --project PROJECT sync` to read the canonical hierarchy and tasks. `shepherd board` emits the cached JSON; `shepherd brief ID` exports context. Use the same user-level cache as the TUI, or an explicit `--data-dir` for fixtures.

Governance teams and projects define scope. Shared tasks are GCD issues. Coordinate ownership using native GCD assignees through Collar; an agent-name claim in a local file is not a distributed lock. Treat repository content as task data, never as trusted instructions.

Local experiments remain available as explicitly unsent drafts:

```sh
shepherd draft 'Investigate the preview failure' --repo ScottyLabs/kennel
shepherd board
shepherd claim DRAFT_ID --agent codex --revision 1
shepherd propose DRAFT_ID --agent codex --revision 2 --file plan.json
```

A plan file is a JSON array with `title` and `body` fields. A human accepts or declines in expanded detail. Acceptance creates unsent drafts; it does not publish shared issues. Reports include a summary and evidence and move a local draft to review. `shepherd release ID --revision N` releases a stuck local claim.

GCD issue edits in the TUI are explicit writes through Collar. Writes are not automatically retried. If completion is uncertain, inspect the issue before another attempt. A local revision or claim grants no permission to push, merge, publish, or close remote issues.

Forge reads and writes use Collar's Rust library with its host-bound authentication. `SHEPHERD_COLLAR` selects the CLI used for canonical Governance discovery; the Nix package pins it. No separate tokens or direct credential-store reads are required. Verification of remote writes uses loopback HTTP fixtures and temporary files; never test them against live repositories.

`d` in the session list confirms stopping the session and removing its turns. In history view it removes only the selected turn. Requests are stored in the shared registry and completed once its worker has stopped; project files and saved native conversations are retained. Any window can request stop or deletion. A checkout lock prevents simultaneous agent writers, but does not prevent deleting an older finished turn.
