use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Parser, Subcommand, ValueEnum};

use crate::db::{Database, InsertOutcome};
use crate::model::{AttributionUpdate, QuoteFilter};
use crate::transfer::{self, ImportFormat};
use crate::tui;

#[derive(Debug, Parser)]
#[command(
    name = "quotes",
    version,
    about = "A portable SQLite-backed quote manager"
)]
pub struct Cli {
    /// Use this SQLite database instead of QUOTES_DATABASE or the platform default.
    #[arg(long, global = true, value_name = "PATH")]
    pub database: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive terminal interface.
    Tui,
    /// Add a quote.
    Add {
        text: String,
        #[arg(long)]
        attribution: Option<String>,
    },
    /// Edit an existing quote.
    #[command(group(
        ArgGroup::new("change")
            .required(true)
            .multiple(true)
            .args(["text", "attribution", "clear_attribution"])
    ))]
    Edit {
        id: i64,
        #[arg(long)]
        text: Option<String>,
        #[arg(long, conflicts_with = "clear_attribution")]
        attribution: Option<String>,
        #[arg(long)]
        clear_attribution: bool,
    },
    /// Permanently remove a quote.
    Remove {
        id: i64,
        /// Confirm deletion (required for non-interactive safety).
        #[arg(long, required = true)]
        yes: bool,
    },
    /// List quotes in insertion order.
    List {
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        min_width: Option<u32>,
        #[arg(long)]
        max_width: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// Select one random matching quote.
    Get {
        #[arg(long)]
        min_width: Option<u32>,
        #[arg(long)]
        max_width: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// Merge quotes from a legacy text or versioned JSON file.
    Import {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = FormatArg::Auto)]
        format: FormatArg,
    },
    /// Write versioned portable JSON; use - for standard output.
    Export { path: PathBuf },
    /// Print the active SQLite database path.
    Path,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum FormatArg {
    #[default]
    Auto,
    Legacy,
    Json,
}

impl From<FormatArg> for ImportFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Auto => Self::Auto,
            FormatArg::Legacy => Self::Legacy,
            FormatArg::Json => Self::Json,
        }
    }
}

pub fn main_entry() -> ExitCode {
    let cli = Cli::parse();
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("quotes: {error:#}");
            ExitCode::from(1)
        }
    }
}

pub fn execute(cli: Cli) -> Result<()> {
    let database_path = resolve_database_path(cli.database)?;
    if matches!(cli.command, Some(Command::Path)) {
        println!("{}", database_path.display());
        return Ok(());
    }

    let mut database = Database::open(&database_path)?;
    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => tui::run(&mut database),
        Command::Add { text, attribution } => {
            match database.add(&text, attribution.as_deref())? {
                InsertOutcome::Added(quote) => println!("Added quote {}.", quote.id),
                InsertOutcome::Duplicate(id) => println!("Skipped duplicate quote {id}."),
            }
            Ok(())
        }
        Command::Edit {
            id,
            text,
            attribution,
            clear_attribution,
        } => {
            let attribution = if clear_attribution {
                AttributionUpdate::Clear
            } else if let Some(attribution) = attribution {
                AttributionUpdate::Set(attribution)
            } else {
                AttributionUpdate::Keep
            };
            let quote = database.edit(id, text.as_deref(), attribution)?;
            println!("Updated quote {}.", quote.id);
            Ok(())
        }
        Command::Remove { id, yes } => {
            if !yes {
                bail!("refusing to remove quote {id} without --yes");
            }
            if !database.remove(id)? {
                bail!("quote {id} does not exist");
            }
            println!("Removed quote {id}.");
            Ok(())
        }
        Command::List {
            search,
            min_width,
            max_width,
            json,
        } => {
            let filter = QuoteFilter {
                search,
                min_width,
                max_width,
            };
            let quotes = database.list(&filter)?;
            if quotes.is_empty() {
                bail!("no quotes matched the requested filters");
            }
            if json {
                let stdout = io::stdout();
                let mut output = stdout.lock();
                serde_json::to_writer_pretty(&mut output, &quotes)?;
                writeln!(output)?;
            } else {
                for quote in quotes {
                    println!(
                        "{:>4} [{:>3}] {}",
                        quote.id,
                        quote.display_width,
                        quote.rendered()
                    );
                }
            }
            Ok(())
        }
        Command::Get {
            min_width,
            max_width,
            json,
        } => {
            let filter = QuoteFilter {
                search: None,
                min_width,
                max_width,
            };
            let quote = database
                .random(&filter)?
                .context("no quotes matched the requested width filters")?;
            if json {
                let stdout = io::stdout();
                let mut output = stdout.lock();
                serde_json::to_writer_pretty(&mut output, &quote)?;
                writeln!(output)?;
            } else {
                println!("{}", quote.rendered());
            }
            Ok(())
        }
        Command::Import { path, format } => {
            let report = transfer::import_file(&mut database, &path, format.into())?;
            println!(
                "Imported {} quote(s); skipped {} duplicate(s).",
                report.added, report.skipped
            );
            Ok(())
        }
        Command::Export { path } => {
            if path.as_os_str() == "-" {
                print!("{}", transfer::export_json(&database)?);
                io::stdout().flush()?;
            } else {
                transfer::export_file(&database, &path)?;
                println!("Exported quotes to {}.", path.display());
            }
            Ok(())
        }
        Command::Path => unreachable!("path is handled before opening the database"),
    }
}

pub fn resolve_database_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        if path.as_os_str().is_empty() {
            bail!("database path cannot be empty");
        }
        return Ok(path);
    }
    if let Some(path) = env::var_os("QUOTES_DATABASE").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(data_home) = env::var_os("XDG_DATA_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(data_home).join("quotes").join("quotes.db"));
    }
    let home = env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .context(
            "cannot determine the default database path because HOME is not set; use --database",
        )?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("quotes")
        .join("quotes.db"))
}
