use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The two kinds of user-owned lexical personalization supported by V1.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PersonalizationEntryKind {
    #[default]
    Vocabulary,
    Correction,
}

impl PersonalizationEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vocabulary => "vocabulary",
            Self::Correction => "correction",
        }
    }

    pub fn parse_storage_value(value: &str) -> Option<Self> {
        match value {
            "vocabulary" => Some(Self::Vocabulary),
            "correction" => Some(Self::Correction),
            _ => None,
        }
    }
}

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

/// A local-first vocabulary term or an explicit source-to-replacement rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PersonalizationEntry {
    pub id: String,
    pub kind: PersonalizationEntryKind,
    pub source_text: String,
    pub replacement_text: Option<String>,
    pub language_code: Option<String>,
    pub enabled: bool,
    pub hit_count: u64,
    #[serde(default = "now_utc")]
    pub created_at: DateTime<Utc>,
    #[serde(default = "now_utc")]
    pub updated_at: DateTime<Utc>,
}

impl Default for PersonalizationEntry {
    fn default() -> Self {
        let now = now_utc();
        Self {
            id: Uuid::new_v4().to_string(),
            kind: PersonalizationEntryKind::Vocabulary,
            source_text: String::new(),
            replacement_text: None,
            language_code: None,
            enabled: true,
            hit_count: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

impl PersonalizationEntry {
    pub fn new(
        kind: PersonalizationEntryKind,
        source_text: impl Into<String>,
        replacement_text: Option<String>,
        language_code: Option<String>,
    ) -> Self {
        let now = now_utc();
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            source_text: source_text.into(),
            replacement_text,
            language_code,
            enabled: true,
            hit_count: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips_with_wire_vocabulary() {
        assert_eq!(
            serde_json::to_string(&PersonalizationEntryKind::Vocabulary).unwrap(),
            "\"vocabulary\""
        );
        assert_eq!(
            serde_json::from_str::<PersonalizationEntryKind>("\"correction\"").unwrap(),
            PersonalizationEntryKind::Correction
        );
    }

    #[test]
    fn new_entry_defaults_to_enabled_and_zero_hits() {
        let entry = PersonalizationEntry::new(
            PersonalizationEntryKind::Correction,
            "sbobino",
            Some("Sbobino".to_string()),
            Some("it".to_string()),
        );

        assert!(!entry.id.is_empty());
        assert!(entry.enabled);
        assert_eq!(entry.hit_count, 0);
        assert_eq!(entry.kind, PersonalizationEntryKind::Correction);
    }
}
