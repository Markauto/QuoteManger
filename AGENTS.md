# Quote Manager contributor guide

## Project intent

Build and maintain the `quotes` Rust application described by `spec.md`. Treat that file as the source of truth for product behavior and test coverage. Keep the original `Quotes` legacy input file unchanged so it remains a recoverable migration source.

## Technical baseline

- Use stable Rust with the 2024 edition.
- Keep the application as one binary named `quotes`.
- Use Clap for CLI parsing, Rusqlite with bundled SQLite for storage, Ratatui/Crossterm for the TUI, Serde for JSON transfer, and `unicode-width` for displayed-width calculations.
- Keep domain/database behavior independent of terminal rendering so it can be unit tested.
- Use transactions for multi-row writes and deterministic ordering for exported data.
- Preserve stdout for requested data; send diagnostics and no-result explanations to stderr.

## Quality gates

Run these before considering a change complete:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

Add or update tests for changed behavior. In particular, retain coverage of database migrations, CRUD and filtering, legacy/JSON imports, CLI streams and exit statuses, Unicode width, and TUI state transitions/cleanup.

## Repository hygiene

- Never modify or delete `Quotes` during application migration or tests.
- Do not commit generated databases, build output, or local editor files.
- Prefer small modules organized by responsibility over a monolithic `main.rs`.
- Keep public JSON formats explicitly versioned and backward-compatible within a version.
