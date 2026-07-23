use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::db::{Database, ImportReport};
use crate::model::TransferQuote;

pub const TRANSFER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    Auto,
    Legacy,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferDocument {
    pub version: u32,
    pub quotes: Vec<TransferQuote>,
}

pub fn parse_legacy(input: &str) -> Result<Vec<TransferQuote>> {
    let mut quotes = Vec::new();
    for (index, raw_line) in input.lines().enumerate() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.trim().is_empty() {
            bail!("legacy import line {} is empty", index + 1);
        }
        let (text, attribution) = match line.rsplit_once(" - ") {
            Some((text, attribution)) => (text, Some(attribution.to_owned())),
            None => (line, None),
        };
        quotes.push(
            TransferQuote {
                text: text.to_owned(),
                attribution,
            }
            .normalized()
            .with_context(|| format!("invalid legacy import line {}", index + 1))?,
        );
    }
    if quotes.is_empty() {
        bail!("legacy import contains no quotes");
    }
    Ok(quotes)
}

pub fn parse_json(input: &str) -> Result<Vec<TransferQuote>> {
    let document: TransferDocument =
        serde_json::from_str(input).context("malformed quotes JSON")?;
    if document.version != TRANSFER_VERSION {
        bail!(
            "unsupported JSON transfer version {}; expected {}",
            document.version,
            TRANSFER_VERSION
        );
    }
    document
        .quotes
        .iter()
        .enumerate()
        .map(|(index, quote)| {
            quote
                .normalized()
                .with_context(|| format!("invalid JSON quote at index {index}"))
        })
        .collect()
}

pub fn parse_import(path: &Path, input: &str, format: ImportFormat) -> Result<Vec<TransferQuote>> {
    let format = match format {
        ImportFormat::Auto => {
            let json_extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
            if json_extension || input.trim_start().starts_with('{') {
                ImportFormat::Json
            } else {
                ImportFormat::Legacy
            }
        }
        explicit => explicit,
    };
    match format {
        ImportFormat::Legacy => parse_legacy(input),
        ImportFormat::Json => parse_json(input),
        ImportFormat::Auto => unreachable!("auto format was resolved above"),
    }
}

pub fn import_file(
    database: &mut Database,
    path: &Path,
    format: ImportFormat,
) -> Result<ImportReport> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("could not read import file {}", path.display()))?;
    let quotes = parse_import(path, &input, format)?;
    database.import_quotes(&quotes)
}

pub fn export_json(database: &Database) -> Result<String> {
    let document = TransferDocument {
        version: TRANSFER_VERSION,
        quotes: database.transfer_quotes()?,
    };
    let mut output = serde_json::to_string_pretty(&document)?;
    output.push('\n');
    Ok(output)
}

pub fn export_file(database: &Database, path: &Path) -> Result<()> {
    let output = export_json(database)?;
    fs::write(path, output)
        .with_context(|| format!("could not write export file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_original_legacy_file_with_expected_attributions() {
        let source = include_str!("../Quotes");
        let quotes = parse_legacy(source).unwrap();
        assert_eq!(quotes.len(), 35);
        assert_eq!(
            quotes
                .iter()
                .filter(|quote| quote.attribution.is_some())
                .count(),
            30
        );
        assert_eq!(
            quotes
                .iter()
                .filter(|quote| quote.attribution.is_none())
                .count(),
            5
        );
        for (quote, original) in quotes.iter().zip(source.lines()) {
            assert_eq!(
                crate::model::render_quote(&quote.text, quote.attribution.as_deref()),
                original
            );
        }
    }

    #[test]
    fn splits_only_the_final_legacy_separator() {
        let input =
            "What are you unhappy? Because it is for yourself - and there isn't one - Wei Wu Wei";
        let quotes = parse_legacy(input).unwrap();
        assert_eq!(
            quotes[0].text,
            "What are you unhappy? Because it is for yourself - and there isn't one"
        );
        assert_eq!(quotes[0].attribution.as_deref(), Some("Wei Wu Wei"));
    }

    #[test]
    fn json_round_trip_is_deterministic_and_reimports_skip_duplicates() {
        let first = Database::open_in_memory().unwrap();
        first.add("zebra", None).unwrap();
        first.add("alpha", Some("author")).unwrap();
        let exported = export_json(&first).unwrap();
        assert!(exported.find("alpha").unwrap() < exported.find("zebra").unwrap());

        let parsed = parse_json(&exported).unwrap();
        let mut second = Database::open_in_memory().unwrap();
        let initial = second.import_quotes(&parsed).unwrap();
        assert_eq!(
            initial,
            ImportReport {
                added: 2,
                skipped: 0
            }
        );
        let repeated = second.import_quotes(&parsed).unwrap();
        assert_eq!(
            repeated,
            ImportReport {
                added: 0,
                skipped: 2
            }
        );
        assert_eq!(export_json(&second).unwrap(), exported);
    }

    #[test]
    fn rejects_unknown_json_versions_and_malformed_documents() {
        assert!(parse_json(r#"{"version":99,"quotes":[]}"#).is_err());
        assert!(parse_json(r#"{"version":1,"quotes":[}"#).is_err());
    }
}
