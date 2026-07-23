use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params, params_from_iter};

use crate::model::{
    AttributionUpdate, Quote, QuoteFilter, TransferQuote, normalize_optional, normalize_required,
    rendered_width,
};

pub const SCHEMA_VERSION: i64 = 2;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    Added(Quote),
    Duplicate(i64),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub added: usize,
    pub skipped: usize,
}

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("could not create database directory {}", parent.display())
            })?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("could not open database {}", path.display()))?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn from_connection(mut connection: Connection) -> Result<Self> {
        configure(&connection)?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?)
    }

    pub fn add(&self, text: &str, attribution: Option<&str>) -> Result<InsertOutcome> {
        let quote = TransferQuote {
            text: text.to_owned(),
            attribution: attribution.map(str::to_owned),
        }
        .normalized()?;
        insert_quote(&self.connection, &quote)
    }

    pub fn get(&self, id: i64) -> Result<Option<Quote>> {
        self.connection
            .query_row(
                "SELECT id, text, attribution, display_width, created_at, updated_at
                 FROM quotes WHERE id = ?1",
                [id],
                quote_from_row,
            )
            .optional()
            .context("could not read quote")
    }

    pub fn list(&self, filter: &QuoteFilter) -> Result<Vec<Quote>> {
        filter.validate()?;
        let (conditions, values) = filter_conditions(filter);
        let mut sql = String::from(
            "SELECT id, text, attribution, display_width, created_at, updated_at FROM quotes",
        );
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY id");

        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), quote_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not list quotes")
    }

    pub fn random(&self, filter: &QuoteFilter) -> Result<Option<Quote>> {
        filter.validate()?;
        let (conditions, values) = filter_conditions(filter);
        let mut sql = String::from(
            "SELECT id, text, attribution, display_width, created_at, updated_at FROM quotes",
        );
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY RANDOM() LIMIT 1");
        self.connection
            .query_row(&sql, params_from_iter(values), quote_from_row)
            .optional()
            .context("could not select a random quote")
    }

    pub fn edit(
        &self,
        id: i64,
        text: Option<&str>,
        attribution: AttributionUpdate,
    ) -> Result<Quote> {
        if text.is_none() && attribution == AttributionUpdate::Keep {
            bail!("edit requires --text, --attribution, or --clear-attribution");
        }

        let current = self
            .get(id)?
            .with_context(|| format!("quote {id} does not exist"))?;
        let new_text = match text {
            Some(text) => normalize_required(text, "quote text")?,
            None => current.text.clone(),
        };
        let new_attribution = match attribution {
            AttributionUpdate::Keep => current.attribution.clone(),
            AttributionUpdate::Set(value) => normalize_optional(Some(&value), "attribution")?,
            AttributionUpdate::Clear => None,
        };

        if new_text == current.text && new_attribution == current.attribution {
            return Ok(current);
        }

        let duplicate: bool = self.connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM quotes
                WHERE text = ?1 AND attribution IS ?2 AND id <> ?3
             )",
            params![new_text, new_attribution, id],
            |row| row.get(0),
        )?;
        if duplicate {
            bail!("an identical quote already exists");
        }

        let width = rendered_width(&new_text, new_attribution.as_deref());
        let updated_at = timestamp().max(current.updated_at.saturating_add(1));
        self.connection.execute(
            "UPDATE quotes
             SET text = ?1, attribution = ?2, display_width = ?3, updated_at = ?4
             WHERE id = ?5",
            params![new_text, new_attribution, width, updated_at, id],
        )?;
        self.get(id)?
            .with_context(|| format!("quote {id} disappeared while being edited"))
    }

    pub fn remove(&self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM quotes WHERE id = ?1", [id])?
            > 0)
    }

    pub fn import_quotes(&mut self, quotes: &[TransferQuote]) -> Result<ImportReport> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut report = ImportReport::default();

        for quote in quotes {
            let quote = quote.normalized()?;
            match insert_quote(&transaction, &quote)? {
                InsertOutcome::Added(_) => report.added += 1,
                InsertOutcome::Duplicate(_) => report.skipped += 1,
            }
        }

        transaction.commit()?;
        Ok(report)
    }

    pub fn transfer_quotes(&self) -> Result<Vec<TransferQuote>> {
        let mut statement = self.connection.prepare(
            "SELECT text, attribution FROM quotes
             ORDER BY text COLLATE BINARY, COALESCE(attribution, '') COLLATE BINARY",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(TransferQuote {
                text: row.get(0)?,
                attribution: row.get(1)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("could not prepare quotes for export")
    }

    pub fn count(&self) -> Result<usize> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM quotes", [], |row| row.get(0))?;
        usize::try_from(count).context("database returned an invalid quote count")
    }
}

fn configure(connection: &Connection) -> Result<()> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "DELETE")?;
    Ok(())
}

fn migrate(connection: &mut Connection) -> Result<()> {
    let mut version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        bail!(
            "database schema version {version} is newer than this application supports ({SCHEMA_VERSION})"
        );
    }

    while version < SCHEMA_VERSION {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match version {
            0 => transaction.execute_batch(
                "CREATE TABLE quotes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    text TEXT NOT NULL,
                    attribution TEXT,
                    display_width INTEGER NOT NULL CHECK(display_width >= 0),
                    created_at INTEGER NOT NULL
                 );
                 PRAGMA user_version = 1;",
            )?,
            1 => transaction.execute_batch(
                "ALTER TABLE quotes
                    ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
                 UPDATE quotes SET updated_at = created_at WHERE updated_at = 0;
                 CREATE UNIQUE INDEX quotes_unique_content
                    ON quotes(text, IFNULL(attribution, ''));
                 PRAGMA user_version = 2;",
            )?,
            _ => unreachable!("all supported schema versions have a migration"),
        }
        transaction.commit()?;
        version += 1;
    }
    Ok(())
}

fn insert_quote(connection: &Connection, quote: &TransferQuote) -> Result<InsertOutcome> {
    let now = timestamp();
    let width = rendered_width(&quote.text, quote.attribution.as_deref());
    let changed = connection.execute(
        "INSERT INTO quotes(text, attribution, display_width, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT DO NOTHING",
        params![quote.text, quote.attribution, width, now],
    )?;

    if changed == 0 {
        let id = connection.query_row(
            "SELECT id FROM quotes WHERE text = ?1 AND attribution IS ?2",
            params![quote.text, quote.attribution],
            |row| row.get(0),
        )?;
        return Ok(InsertOutcome::Duplicate(id));
    }

    let id = connection.last_insert_rowid();
    let added = connection.query_row(
        "SELECT id, text, attribution, display_width, created_at, updated_at
         FROM quotes WHERE id = ?1",
        [id],
        quote_from_row,
    )?;
    Ok(InsertOutcome::Added(added))
}

fn quote_from_row(row: &Row<'_>) -> rusqlite::Result<Quote> {
    Ok(Quote {
        id: row.get(0)?,
        text: row.get(1)?,
        attribution: row.get(2)?,
        display_width: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn filter_conditions(filter: &QuoteFilter) -> (Vec<&'static str>, Vec<Value>) {
    let mut conditions = Vec::new();
    let mut values = Vec::new();
    if let Some(search) = filter.normalized_search() {
        conditions.push(
            "(INSTR(LOWER(text), LOWER(?)) > 0 OR INSTR(LOWER(COALESCE(attribution, '')), LOWER(?)) > 0)",
        );
        values.push(Value::Text(search.to_owned()));
        values.push(Value::Text(search.to_owned()));
    }
    if let Some(minimum) = filter.min_width {
        conditions.push("display_width >= ?");
        values.push(Value::Integer(i64::from(minimum)));
    }
    if let Some(maximum) = filter.max_width {
        conditions.push("display_width <= ?");
        values.push(Value::Integer(i64::from(maximum)));
    }
    (conditions, values)
}

fn timestamp() -> i64 {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    i64::try_from(micros).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn initializes_schema_and_uses_delete_journaling() {
        let directory = tempdir().unwrap();
        let database = Database::open(&directory.path().join("quotes.db")).unwrap();
        assert_eq!(database.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(database.journal_mode().unwrap().to_lowercase(), "delete");
    }

    #[test]
    fn migrates_version_one_and_preserves_rows() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE quotes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    text TEXT NOT NULL,
                    attribution TEXT,
                    display_width INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                 );
                 INSERT INTO quotes(text, attribution, display_width, created_at)
                    VALUES ('old', NULL, 3, 42);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        let database = Database::from_connection(connection).unwrap();
        assert_eq!(database.schema_version().unwrap(), SCHEMA_VERSION);
        let quote = database.get(1).unwrap().unwrap();
        assert_eq!(quote.updated_at, 42);
    }

    #[test]
    fn supports_crud_and_updates_width_and_timestamp() {
        let database = Database::open_in_memory().unwrap();
        let InsertOutcome::Added(added) = database.add(" hello ", Some(" world ")).unwrap() else {
            panic!("quote was unexpectedly a duplicate");
        };
        assert_eq!(added.text, "hello");
        assert_eq!(added.attribution.as_deref(), Some("world"));
        assert_eq!(added.display_width, 13);
        assert_eq!(added.created_at, added.updated_at);

        let edited = database
            .edit(added.id, Some("界"), AttributionUpdate::Clear)
            .unwrap();
        assert_eq!(edited.rendered(), "界");
        assert_eq!(edited.display_width, 2);
        assert!(edited.updated_at > edited.created_at);
        assert!(database.remove(added.id).unwrap());
        assert!(!database.remove(added.id).unwrap());
    }

    #[test]
    fn skips_exact_duplicates_but_not_different_attributions() {
        let database = Database::open_in_memory().unwrap();
        assert!(matches!(
            database.add("same", None).unwrap(),
            InsertOutcome::Added(_)
        ));
        assert!(matches!(
            database.add(" same ", None).unwrap(),
            InsertOutcome::Duplicate(_)
        ));
        assert!(matches!(
            database.add("same", Some("author")).unwrap(),
            InsertOutcome::Added(_)
        ));
        assert_eq!(database.count().unwrap(), 2);
    }

    #[test]
    fn searches_and_applies_inclusive_width_bounds() {
        let database = Database::open_in_memory().unwrap();
        database.add("cat", None).unwrap();
        database.add("wide", Some("Dog")).unwrap();
        database.add("something", Some("CATALOGUE")).unwrap();

        let matches = database
            .list(&QuoteFilter {
                search: Some("cat".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(matches.len(), 2);

        let exact = database
            .list(&QuoteFilter {
                min_width: Some(3),
                max_width: Some(3),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].text, "cat");
    }

    #[test]
    fn malformed_batch_rolls_back_every_insert() {
        let mut database = Database::open_in_memory().unwrap();
        let input = vec![
            TransferQuote {
                text: "valid".into(),
                attribution: None,
            },
            TransferQuote {
                text: "bad\nline".into(),
                attribution: None,
            },
        ];
        assert!(database.import_quotes(&input).is_err());
        assert_eq!(database.count().unwrap(), 0);
    }
}
