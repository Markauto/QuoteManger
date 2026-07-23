# Rust Quotes Manager

  ## Summary

  Build a Rust application named quotes that replaces the current flat file with a portable SQLite database. Running quotes opens the TUI; subcommands provide a stable CLI suitable for Fastfetch and shell scripts.

  The existing Quotes file will be imported without modification: 35 quotes total, with the final - parsed as attribution where present.

  ## Implementation Changes

  - Create a Rust 2024 binary project using stable Rust, Ratatui/Crossterm, Clap, Rusqlite with bundled SQLite, Serde, and unicode-width.
  - Store data in a single SQLite file:
      - Default Linux location: $XDG_DATA_HOME/quotes/quotes.db, falling back to ~/.local/share/quotes/quotes.db.
      - Override precedence: --database PATH, then QUOTES_DATABASE, then the default.
      - Use SQLite transactions, schema versioning, DELETE journaling, and a busy timeout so the closed database remains a single safely copyable file.
      - Store local integer ID, quote text, optional attribution, displayed width, and creation/update timestamps.
      - Reject empty or multiline values, trim surrounding whitespace, and skip exact text-plus-attribution duplicates.

  - Render attributed entries as Text - Attribution, without added quotation marks. Measure the complete rendered value in terminal columns, including attribution.
  - Provide these public commands:
      - quotes or quotes tui: launch the TUI.
      - quotes add TEXT [--attribution VALUE]
      - quotes edit ID [--text VALUE] [--attribution VALUE | --clear-attribution]
      - quotes remove ID --yes
      - quotes list [--search QUERY] [--min-width N] [--max-width N] [--json]
      - quotes get [--min-width N] [--max-width N] [--json]: randomly select one matching entry and print only its rendered value by default.
      - quotes import PATH [--format auto|legacy|json]
      - quotes export PATH: write versioned, portable JSON; - represents standard output.
      - quotes path: print the active database path for copying or backup.

  - Validate that widths are non-negative and minimum does not exceed maximum. An empty result prints an explanation to stderr, emits nothing to stdout, and returns a nonzero exit status.
  - Make default list output human-readable; treat --json as the stable machine-readable interface.
  - Implement versioned JSON transfer containing quote text and optional attribution. Imports run in one transaction, fail without partial changes on malformed data, and report added/skipped counts.
  - Legacy import splits only the final - separator, correctly preserving entries such as the Wei Wu Wei quote that contains an earlier separator.
  - Build a two-pane TUI with searchable quote list and selected-quote details. Support arrows/j/k, / search, f width filtering, r random selection, a add, e edit, d delete with confirmation, Esc, and q. Show IDs and displayed widths, validate modal forms, surface errors in the status area, and always restore terminal state on exit or failure.
  - Add documentation for installation, command examples, Fastfetch usage such as quotes get --max-width 80, initial migration, JSON transfer, and safe database copying while the application is closed.

  ## Test Plan

  - Unit-test rendering and terminal-width calculation for unattributed text, attribution, ASCII, combining characters, and wide Unicode glyphs.
  - Test CRUD operations, timestamp/width updates, duplicate skipping, schema initialization/migration, search, and inclusive minimum/maximum selection.
  - Test legacy parsing, including the current file’s expected 35 entries: 30 attributed and 5 unattributed.
  - Test JSON round trips, repeated imports, malformed-file rollback, and deterministic exports.
  - Add CLI integration tests using temporary databases for stdout/stderr separation, JSON output, exit statuses, database overrides, and no-match behavior.
  - Test TUI state transitions and form validation independently from the terminal renderer, plus terminal cleanup on normal and error exits.
  - Run formatting, Clippy with warnings denied, the complete test suite, and a release build.

  ## Assumptions

  - Version one targets local, single-user use and portable desktop platforms; it does not provide cloud or live multi-PC synchronization.
  - Copying the SQLite database or exporting/importing JSON is the supported sharing workflow.
  - Imports merge into the destination and skip exact duplicates; they do not delete destination-only quotes.
  - The original Quotes file remains untouched after migration as a recoverable source copy, but the application does not use it at runtime.
  - Tags, favorites, collections, clipboard integration, and usage-history-based quote rotation are deferred.
