# Doghouse

Doghouse is the ScottyLabs terminal project manager. Browse Governance projects, manage Forgejo issues and pull requests, and start Codex or Claude Code sessions from a task.

## Get started

From a Doghouse checkout, sign in to Forgejo with Collar and open the board:

```sh
nix develop --command collar auth login
nix run .
```

Use `w` to choose a workspace, `P` to choose a project, and `?` to see the available actions.

## Documentation

- [Getting started](docs/getting-started.md)
- [Terminal workflow](docs/tui.md)
- [Agent workflows](docs/agents.md)
- [Contributing](CONTRIBUTING.md)

## License

[AGPL-3.0-or-later](LICENSE).
