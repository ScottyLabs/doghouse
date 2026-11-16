# Shepherd

A Rust terminal project manager for ScottyLabs. Governance defines workspaces and projects; GCD issues hold shared tasks. Collar supplies discovery, authentication, and forge operations.

```sh
nix run . -- --workspace devops
```

Press `c` to add a directory under Local projects; `n` then creates tasks stored on this machine. Use `w` for a workspace, `P` for a project, and `s` to refresh tasks. The task list has a live preview; `v` switches to kanban. `A` launches Codex or Claude Code for a task; `a` opens local runs, activity, results, and diff review. `?` shows actions. [Terminal workflow](docs/tui.md) covers editing and shared state.

The Nix package and development shell pin the [Collar integration](https://git.cmu.dev/ScottyLabs/collar/compare/main...fix/governance-schema), including typed team discovery and current Governance schema support. No separate Collar upgrade is needed for `nix run`. For development, use `collar dev run -- shepherd --workspace devops`; `SHEPHERD_COLLAR` can explicitly select a test binary.

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
