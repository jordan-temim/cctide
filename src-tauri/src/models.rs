//! Per-model data: quota weighting and context window.
//!
//! The table ships **with the app**: `models.json` at the repo root, compiled
//! into the binary via `include_str!`. Edit it when Anthropic changes prices or
//! releases new models, then rebuild. Nothing is written to `~/.claude`.
//!
//! Keys match a substring of the model id; the longest match wins (so
//! `opus-4-8` beats `opus`). Only pricing ratios matter; calibration absorbs
//! the absolute scale.
//!
//! **Cache reads are deliberately excluded from quota weighting.** Anthropic's
//! rate-limit metering counts `input + cache_creation` and does *not* count
//! `cache_read` (documented for the API ITPM limit). Counting them made our
//! metric balloon with conversation length and over-estimate usage.
//!
//! Source: <https://platform.claude.com/docs/en/about-claude/pricing>,
//! <https://platform.claude.com/docs/en/about-claude/models/overview>
//! (captured 2026-05-30).

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

const MODELS_JSON: &str = include_str!("../../models.json");

/// All data for one model variant.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    /// Quota weight ($/MTok) for input tokens.
    pub input: f64,
    /// Quota weight ($/MTok) for output tokens.
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    /// Context-window size in tokens (e.g. 200_000).
    #[serde(default = "default_context_window")]
    pub context_window: u64,
}

fn default_context_window() -> u64 {
    200_000
}

#[derive(Debug, Clone, Deserialize)]
pub struct Models {
    /// Entries keyed by a substring of the model id; longest match wins.
    pub models: BTreeMap<String, ModelEntry>,
    /// Fallback for an unrecognised model.
    pub default: ModelEntry,
}

impl Default for ModelEntry {
    fn default() -> Self {
        ModelEntry {
            input: 3.0,
            output: 15.0,
            cache_write_5m: 3.75,
            cache_write_1h: 6.0,
            context_window: 200_000,
        }
    }
}

impl Default for Models {
    fn default() -> Self {
        let mut models = BTreeMap::new();
        models.insert(
            "opus-4-8".to_string(),
            ModelEntry {
                input: 5.0,
                output: 25.0,
                cache_write_5m: 6.25,
                cache_write_1h: 10.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "opus-4-7".to_string(),
            ModelEntry {
                input: 5.0,
                output: 25.0,
                cache_write_5m: 6.25,
                cache_write_1h: 10.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "opus-4-6".to_string(),
            ModelEntry {
                input: 5.0,
                output: 25.0,
                cache_write_5m: 6.25,
                cache_write_1h: 10.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "opus-4-1".to_string(),
            ModelEntry {
                input: 15.0,
                output: 75.0,
                cache_write_5m: 18.75,
                cache_write_1h: 30.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "opus".to_string(),
            ModelEntry {
                input: 5.0,
                output: 25.0,
                cache_write_5m: 6.25,
                cache_write_1h: 10.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "sonnet-4-6".to_string(),
            ModelEntry {
                input: 3.0,
                output: 15.0,
                cache_write_5m: 3.75,
                cache_write_1h: 6.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "sonnet".to_string(),
            ModelEntry {
                input: 3.0,
                output: 15.0,
                cache_write_5m: 3.75,
                cache_write_1h: 6.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "haiku".to_string(),
            ModelEntry {
                input: 1.0,
                output: 5.0,
                cache_write_5m: 1.25,
                cache_write_1h: 2.0,
                context_window: 200_000,
            },
        );
        models.insert(
            "haiku-3".to_string(),
            ModelEntry {
                input: 0.8,
                output: 4.0,
                cache_write_5m: 1.0,
                cache_write_1h: 1.6,
                context_window: 200_000,
            },
        );
        Models {
            models,
            default: ModelEntry::default(),
        }
    }
}

/// Loads the embedded model table (falls back to hardcoded defaults if parsing fails).
pub fn load() -> Models {
    serde_json::from_str::<Models>(MODELS_JSON).unwrap_or_default()
}

impl Models {
    /// Entry for a model id (longest substring key wins, else the default).
    pub fn entry_for(&self, model: &str) -> &ModelEntry {
        let m = model.to_lowercase();
        let mut best: Option<(usize, &ModelEntry)> = None;
        for (key, entry) in &self.models {
            if m.contains(&key.to_lowercase()) {
                let len = key.len();
                if best.is_none_or(|(bl, _)| len > bl) {
                    best = Some((len, entry));
                }
            }
        }
        best.map(|(_, e)| e).unwrap_or(&self.default)
    }

    /// Quota-weighted units for one assistant turn — input + output + cache
    /// writes, **excluding cache reads** (see module docs).
    pub fn quota_units(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_write_5m: u64,
        cache_write_1h: u64,
    ) -> f64 {
        let e = self.entry_for(model);
        input as f64 * e.input
            + output as f64 * e.output
            + cache_write_5m as f64 * e.cache_write_5m
            + cache_write_1h as f64 * e.cache_write_1h
    }

    /// Context-window limit for a model, in tokens. Checks user overrides first,
    /// then the model table, then falls back to 200k.
    pub fn context_limit_for(&self, model: &str, overrides: &HashMap<String, u64>) -> u64 {
        let m = model.to_lowercase();
        let mut best: Option<(usize, u64)> = None;
        for (key, val) in overrides {
            if m.contains(&key.to_lowercase()) {
                let len = key.len();
                if best.is_none_or(|(bl, _)| len > bl) {
                    best = Some((len, *val));
                }
            }
        }
        if let Some((_, v)) = best {
            return v;
        }
        self.entry_for(model).context_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn m() -> Models {
        Models::default()
    }

    // --- entry_for ---

    #[test]
    fn longest_match_wins_over_generic_key() {
        let models = m();
        let e = models.entry_for("claude-opus-4-1");
        assert!(
            (e.input - 15.0).abs() < 1e-9,
            "expected opus-4-1 pricing, got {}",
            e.input
        );
    }

    #[test]
    fn generic_fallback_for_unknown_model() {
        let models = m();
        let e = models.entry_for("claude-totally-unknown");
        // Unknown model falls back to default pricing (sonnet-class).
        assert!((e.input - 3.0).abs() < 1e-9);
    }

    #[test]
    fn case_insensitive_match() {
        let models = m();
        let e1 = models.entry_for("claude-SONNET-4-6");
        let e2 = models.entry_for("claude-sonnet-4-6");
        assert_eq!(e1.input, e2.input);
    }

    // --- quota_units ---

    #[test]
    fn quota_units_sonnet_basic() {
        // sonnet: input=3, output=15, cache_write_5m=3.75
        let units = m().quota_units("claude-sonnet-4-6", 1_000, 500, 200, 0);
        let expected = 1_000.0 * 3.0 + 500.0 * 15.0 + 200.0 * 3.75;
        assert!((units - expected).abs() < 1e-6);
    }

    #[test]
    fn quota_units_zero_tokens() {
        assert_eq!(m().quota_units("claude-sonnet-4-6", 0, 0, 0, 0), 0.0);
    }

    #[test]
    fn quota_units_opus_more_expensive_than_sonnet() {
        let sonnet = m().quota_units("claude-sonnet-4-6", 1000, 1000, 0, 0);
        let opus = m().quota_units("claude-opus-4-8", 1000, 1000, 0, 0);
        assert!(opus > sonnet);
    }

    // --- context_limit_for ---

    #[test]
    fn context_limit_default_200k() {
        let limit = m().context_limit_for("claude-sonnet-4-6", &HashMap::new());
        assert_eq!(limit, 200_000);
    }

    #[test]
    fn context_limit_user_override_wins() {
        let mut overrides = HashMap::new();
        overrides.insert("sonnet".to_string(), 500_000u64);
        let limit = m().context_limit_for("claude-sonnet-4-6", &overrides);
        assert_eq!(limit, 500_000);
    }

    #[test]
    fn context_limit_longest_override_wins() {
        let mut overrides = HashMap::new();
        overrides.insert("sonnet".to_string(), 100_000u64);
        overrides.insert("sonnet-4-6".to_string(), 999_000u64);
        let limit = m().context_limit_for("claude-sonnet-4-6", &overrides);
        assert_eq!(limit, 999_000);
    }
}
