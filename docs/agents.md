# Agent workflows

## Codex and Claude Code

Select a task and press `A`. Type an optional goal and press Enter; an empty goal uses the task itself. Shepherd selects an installed CLI and the project's known checkout. Tab opens secondary settings for the agent, task mode, and directory; Left/Right changes options, Enter returns to the goal, and Esc cancels. Ctrl+S is an alternative launch key. Alt+Enter or Shift+Enter inserts a goal newline where supported by the terminal. Pasting multiline goals does not submit them.

For shared tasks, the checkout must have a Git remote matching the repository. Local tasks use their registered directory. Git-backed issue tasks default to implementation in a separate checkout; PRs and plain directories default to planning. The last available backend is remembered; native CLI model and login settings are retained.

Before starting, Shepherd loads applicable `AGENTS.md` files from the original directory and its parents, in precedence order, and includes them in the run context. The run records which files were loaded. Implementation instructions require reading project documentation, outlining the approach and checks before editing, implementing, running relevant checks, and reporting the result. Agents must honor deeper instructions where they edit.

- **Plan** uses Codex read-only mode or Claude Code plan mode with read tools. It inspects the selected checkout, including local changes.
- **Implement** creates a detached Git worktree from committed HEAD. Your original checkout remains available. New files appear in worktree status; the diff includes tracked changes from the base commit.

Shepherd invokes the installed native CLI with its existing login and model configuration. It does not provision API keys or run a separate agent framework. Both CLI integrations use structured output; Codex uses native automatic approval review for implementation, and Claude Code uses `acceptEdits` with noninteractive permission handling. Denied tools appear as Needs attention. Authentication, version, and permission failures remain visible in activity. Claude Code must support `--permission-prompts none`.

| Agents key | Action |
| -- | -- |
| A / n | New run for the selected task |
| r | Resume the native session with a follow-up goal |
| s | Stop the selected agent and its child tools, including in another window |
| Enter | Open or resume the native Codex/Claude conversation |
| Ctrl+Z (inside CLI) | Return to Shepherd, keeping the same native process |
| Home / End | First / last run (Home also resets preview scrolling) |
| v | Switch between activity and the last automated result |
| Delete / D | Confirm deletion of finished run history and transcript |
| d | Review implementation status and diff |
| P | Review a result comment; Enter explicitly publishes it to GCD |
| x | Remove a clean isolated checkout |
| f | Toggle current project and all local runs |
| t / O | Return to the task / open its GCD issue |
| a / Esc | Return to tasks |

Enter takes you into the real native CLI in the same terminal. Ctrl+Z detaches from the CLI and returns to Shepherd while the agent keeps working. Enter reattaches to the same process, including its in-memory draft. A tmux terminal in the private user cache preserves the screen and runs independently of the visible conversation; packaged Shepherd includes tmux. Exiting the CLI also returns to Shepherd. Other background agents continue being polled. Opening a still-running automated turn first asks to stop that turn and take over its session. Every Shepherd instance using the same user cache can attach to that terminal, including simultaneously, without restarting the CLI. Background takeover and `s` send a local stop request to the owning window, which signals only that agent's process group. The owning window retains checkout and task locks; closing it still stops its agents. Separate `--data-dir` caches and other machines do not share terminals.

After an interactive conversation, the saved automated result is labeled as historical and cannot be published as a fresh result. An automated follow-up with `r` can produce a new publishable result. Native session history remains available through Enter.

Delete removes only Shepherd's run record and transcript. It requires confirmation, blocks active/detached runs and pending publication, and retains project files, worktrees, native session history, and GCD comments. `x` separately removes a clean worktree.

Up to four runs can execute in one window. Local ownership locks prevent another Shepherd window from using the same task or checkout concurrently. Resume retains the original checkout and native session. A changed HEAD must be preserved before starting a new run. Cleanup refuses dirty worktrees or worktrees containing new commits. Stopping a run keeps its changes and result.

Runs, logs, native session IDs, and checkouts live under the user cache's private `agents/` directory. Activity and result previews are bounded; the raw transcript is capped at 8 MiB per run. Token usage is shown only when the CLI reports it. Claude's reported cost is a cumulative CLI estimate for the session, not a billing total.

Agents run on **this machine**. They are not a distributed job service. Governance and GCD provide the shared hierarchy and issues; explicitly published result comments are visible elsewhere and loaded in expanded task detail. Local run state does not assign or move shared issues. Publication is never automatically retried, including after an uncertain response. Inspect GCD before another attempt.

Ordinary quit waits for active runs; stop them first. Ctrl+C or a handled terminal-close signal stops owned process groups and records them as stopped. Abrupt crashes, SIGKILL, or tools that escape the process group can leave an unattached native session; inspect it in the native CLI before continuing. Repository instructions and each backend's permissions still apply.

Integration references: [Codex noninteractive mode](https://learn.chatgpt.com/docs/non-interactive-mode), [Codex commands](https://learn.chatgpt.com/docs/developer-commands), [Claude Code programmatic use](https://code.claude.com/docs/en/headless), and [Claude Code CLI reference](https://code.claude.com/docs/en/cli-reference).

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
