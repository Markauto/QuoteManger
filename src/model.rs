use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Quote {
    pub id: i64,
    pub text: String,
    pub attribution: Option<String>,
    pub display_width: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Quote {
    #[must_use]
    pub fn rendered(&self) -> String {
        render_quote(&self.text, self.attribution.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferQuote {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

impl TransferQuote {
    pub fn normalized(&self) -> Result<Self> {
        Ok(Self {
            text: normalize_required(&self.text, "quote text")?,
            attribution: normalize_optional(self.attribution.as_deref(), "attribution")?,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuoteFilter {
    pub search: Option<String>,
    pub min_width: Option<u32>,
    pub max_width: Option<u32>,
}

impl QuoteFilter {
    pub fn validate(&self) -> Result<()> {
        if let (Some(minimum), Some(maximum)) = (self.min_width, self.max_width)
            && minimum > maximum
        {
            bail!("minimum width ({minimum}) cannot exceed maximum width ({maximum})");
        }
        Ok(())
    }

    #[must_use]
    pub fn normalized_search(&self) -> Option<&str> {
        self.search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionUpdate {
    Keep,
    Set(String),
    Clear,
}

#[must_use]
pub fn render_quote(text: &str, attribution: Option<&str>) -> String {
    match attribution {
        Some(attribution) => format!("{text} - {attribution}"),
        None => text.to_owned(),
    }
}

#[must_use]
pub fn rendered_width(text: &str, attribution: Option<&str>) -> u32 {
    let width = UnicodeWidthStr::width(render_quote(text, attribution).as_str());
    u32::try_from(width).unwrap_or(u32::MAX)
}

pub fn normalize_required(value: &str, field: &str) -> Result<String> {
    if value.contains(['\n', '\r']) {
        bail!("{field} must be a single line");
    }
    let value = value.trim();
    if value.is_empty() {
        bail!("{field} cannot be empty");
    }
    Ok(value.to_owned())
}

pub fn normalize_optional(value: Option<&str>, field: &str) -> Result<Option<String>> {
    value
        .map(|value| normalize_required(value, field))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_without_added_quotes() {
        assert_eq!(render_quote("Be here", None), "Be here");
        assert_eq!(
            render_quote("Be here", Some("Someone")),
            "Be here - Someone"
        );
    }

    #[test]
    fn measures_ascii_and_attribution() {
        assert_eq!(rendered_width("abc", None), 3);
        assert_eq!(rendered_width("abc", Some("xy")), 8);
    }

    #[test]
    fn measures_combining_characters_and_wide_glyphs() {
        assert_eq!(rendered_width("e\u{301}", None), 1);
        assert_eq!(rendered_width("界", None), 2);
        assert_eq!(rendered_width("界", Some("禅")), 7);
    }

    #[test]
    fn trims_but_rejects_empty_and_multiline_values() {
        assert_eq!(normalize_required("  hello  ", "text").unwrap(), "hello");
        assert!(normalize_required("   ", "text").is_err());
        assert!(normalize_required("first\nsecond", "text").is_err());
        assert!(normalize_required("first\rsecond", "text").is_err());
    }

    #[test]
    fn validates_filter_bounds() {
        assert!(
            QuoteFilter {
                min_width: Some(4),
                max_width: Some(3),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            QuoteFilter {
                min_width: Some(3),
                max_width: Some(3),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
}
