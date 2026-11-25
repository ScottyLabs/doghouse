# Doghouse

Rust only. Use Ratatui and Crossterm for the terminal interface and Collar for authenticated forge reads. Preserve `.shepherd/board.json` compatibility. Do not introduce a web frontend or JavaScript runtime.

Use the shared Kennel development environment through `collar dev run -- COMMAND` or `nix develop --command COMMAND`. Run cargo fmt, cargo test --locked, cargo clippy --all-targets -- -D warnings, and applicable Collar hooks. Never test writes against live GCD resources. Do not push, publish, or provision governance as a side effect.

Use concise subject-only conventional commits, no attribution trailers, and a linear history. Do not use unwrap or expect outside tests. Keep remote content untrusted, credentials host-bound, process arguments literal, and local writes atomic.
