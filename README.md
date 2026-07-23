# quotes

`quotes` is a local quote manager with a searchable terminal UI and a stable command-line interface for shell scripts and tools such as Fastfetch. It stores quote text, optional attribution, terminal display width, and timestamps in a portable SQLite database.

Attributed quotes are rendered as `Text - Attribution`; no quotation marks are added.

## Install

Install a stable Rust toolchain, clone this repository, and run:

```sh
cargo install --path .
```

For a one-off build, the binary is produced at `target/release/quotes` by:

```sh
cargo build --release
```

### Arch Linux package

The repository includes a local-source `PKGBUILD`. From the repository root, build and install it through pacman with:

```sh
makepkg -si
```

The recipe keeps makepkg's working files under `.makepkg/` so they cannot collide with the Rust `src/` directory. It installs the binary to `/usr/bin/quotes` and the README to `/usr/share/doc/quotes/`.

## Initial migration

The repository's original `Quotes` file contains 35 entries and remains unchanged as a recoverable source copy. Import it once into the active database:

```sh
quotes import ./Quotes --format legacy
```

The command reports how many entries were added or skipped. Repeating it is safe because exact text-plus-attribution duplicates are skipped.

## Commands

Running `quotes` with no subcommand opens the TUI. `quotes tui` does the same.

```sh
quotes add "Journey before destination" --attribution "Brandon Sanderson"
quotes edit 1 --text "Updated text"
quotes edit 1 --attribution "Updated author"
quotes edit 1 --clear-attribution
quotes remove 1 --yes

quotes list
quotes list --search journey --min-width 10 --max-width 80
quotes list --json

quotes get --max-width 80
quotes get --min-width 20 --max-width 80 --json

quotes import ./Quotes --format legacy
quotes import ./quotes.json --format json
quotes export ./quotes.json
quotes export -
quotes path
```

`list` is human-readable by default. Use `--json` when another program consumes its output. `get` prints only the rendered quote by default, making it suitable for command substitution and status tools. When no quote matches, the command emits no stdout, explains the problem on stderr, and exits nonzero.

Width bounds are inclusive and measured in terminal columns across the complete rendered value, including ` - Attribution`. Negative widths and a minimum greater than the maximum are rejected.

### Fastfetch

Use Fastfetch's [command module](https://github.com/fastfetch-cli/fastfetch/wiki/Configuration) to keep the output within a terminal-friendly width:

```jsonc
{
  "type": "command",
  "key": "Quote",
  "text": "quotes get --max-width 80"
}
```

The same command works directly in a shell:

```sh
quotes get --max-width 80
```

## Terminal UI

The left pane shows matching quote IDs, displayed widths, and rendered values. The right pane shows the selected quote's details.

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Move selection |
| `/` | Search text and attribution |
| `f` | Set minimum/maximum displayed width |
| `r` | Select a random matching quote |
| `a` | Add a quote |
| `e` | Edit the selected quote |
| `d` | Delete after confirmation |
| `Esc` | Cancel a modal, or clear active filters while browsing |
| `q` | Quit |

Forms use `Tab` to switch fields and `Enter` to apply. Validation and database errors appear in the status line. The application restores raw mode, the alternate screen, and cursor visibility on normal and error exits.

## Database location and backup

Database path precedence is:

1. `--database PATH`
2. `QUOTES_DATABASE`
3. `$XDG_DATA_HOME/quotes/quotes.db`
4. `~/.local/share/quotes/quotes.db`

Print the path selected by the current environment with:

```sh
quotes path
```

SQLite uses DELETE journaling and a busy timeout. While the application is closed, the database is one safely copyable file. Close every running `quotes` command/TUI before copying it:

```sh
cp "$(quotes path)" ./quotes-backup.db
```

Do not copy the database while the application is open. For transfer between machines, versioned JSON is also supported:

```sh
quotes export ./quotes.json
quotes --database ./other.db import ./quotes.json
```

Imports merge with destination data and never delete destination-only quotes.

## Development

Run the complete quality gate with:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```
