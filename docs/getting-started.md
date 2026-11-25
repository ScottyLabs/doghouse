# Getting started

## Run Doghouse

From a checkout with Nix available:

```sh
nix develop --command collar auth login
nix run .
```

Doghouse uses your Collar login for authenticated Forgejo access. The Nix package includes the Collar CLI used for Governance discovery. To install the Doghouse command in your user profile, run `nix profile add .`.

## Find your work

Press `w` to choose a Governance team and `P` to choose a project. Use `u` to switch to **My work**, or `+` to register a local directory. Press `?` for the keyboard guide. See [Terminal workflow](tui.md) for task actions and [Agent workflows](agents.md) for starting a session from a task.

The CLI can also list the shared hierarchy or open a specific workspace:

```sh
doghouse discover
doghouse --workspace devops
doghouse --help
```

## Local data

The board cache defaults to `$XDG_CACHE_HOME/doghouse` or `~/.cache/doghouse`. Use `--data-dir` or `DOGHOUSE_DATA_DIR` to select another directory. With no board in the default cache, Doghouse first checks the previous `shepherd` user cache, then checkout-local `.shepherd/board.json`.

Use `doghouse --offline` to open the cached board without remote writes. The cache can contain private repository data.
