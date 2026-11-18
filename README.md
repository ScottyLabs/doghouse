# Shepherd

A Rust terminal project manager for ScottyLabs. Governance defines workspaces and projects; GCD issues hold shared tasks. Collar supplies discovery, authentication, and forge operations.

```sh
nix run . -- --workspace devops
```

Press `+` to add a directory under Local projects; `n` then creates tasks stored on this machine. Use `u` for All accessible or My work, `w` for a workspace, `P` for a project, and `s` to refresh tasks. The task list has a live preview; `v` switches to kanban. `A` opens a goal-first agent dialog; Enter starts it. `Space` opens task actions; `e`/`d` edit issue or PR text, `p` picks priority, `c` comments, and `z` closes/reopens. `a` opens runs; Enter talks to an agent and Ctrl+Z returns. `v` previews results, `d` shows changes, and Delete removes finished run history. `?` shows actions. [Terminal workflow](docs/tui.md) covers editing and shared state.

The Nix package and development shell pin [Collar](https://git.cmu.dev/ScottyLabs/collar), including canonical team discovery and revalidated API response caching. No separate Collar upgrade is needed for `nix run`. For development, use `collar dev run -- shepherd --workspace devops`; Forge access uses Collar as a Rust library with shared connections and authenticated caching. `SHEPHERD_COLLAR` selects the CLI used for canonical Governance discovery.

```sh
nix run . -- discover
nix run . -- --workspace cmu-housing --project housing sync
nix run . -- --offline --workspace devops
```

The cache defaults to `$XDG_CACHE_HOME/shepherd` or `~/.cache/shepherd`, independently of the current directory. `--data-dir` overrides it. Existing checkout-local `.shepherd/board.json` is copied on first use when the user cache does not exist. Local drafts and old planning notes are retained, clearly separated from shared issues.

```sh
collar dev run -- cargo test --locked
collar dev run -- cargo clippy --all-targets -- -D warnings
collar check --all
nix build --no-link
```

The shared Kennel flake supplies the Rust environment, hooks, and packaging. Forgejo CI uses Kennel's reusable checks. See the [agent guide](docs/agents.md) for session workflows.
