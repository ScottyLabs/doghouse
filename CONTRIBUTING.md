# Contributing

Doghouse is a Rust CLI and terminal application. Keep user-facing guidance in `docs/` and repository maintenance instructions in [AGENTS.md](AGENTS.md).

## Development

Use the shared Kennel environment from the repository root:

```sh
collar dev run -- doghouse --offline
collar dev run -- cargo fmt
collar dev run -- cargo test --locked
collar dev run -- cargo clippy --all-targets -- -D warnings
collar check --all
```

Use temporary data directories and local fixtures when testing writes. Check the Nix package with `nix build --no-link` when changing packaging. The Forgejo workflow uses Kennel's reusable checks.

## Commits and review

Work on a feature branch based on `main`. Use atomic, subject-only conventional commits and keep the history linear. Update the relevant guides when behavior changes. Open a pull request for review with a summary of the change and the checks you ran. Governance owns repository provisioning and settings.
