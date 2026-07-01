//! Per-model data: quota weighting and context window.
//!
//! The table ships **with the app**: `models.json` at the repo root, compiled
//! into the binary via `include_str!`. Edit it when Anthropic changes the quota
//! mechanics or releases new models, then rebuild. Nothing is written to
//! `~/.claude`.
//!
//! Keys match a substring of the model id; the longest match wins (so
//! `opus-4-8` beats `opus`). Only ratios matter; calibration absorbs the
//! absolute scale.
//!
//! **Two independent weight sets per model:**
//! - `input`/`output`/`cache_write_*` are the **$/MTok prices**, kept for
//!   reference only.
//! - `quota` holds the **quota weights** actually used by [`Models::quota_units`]
//!   to estimate the 5h/weekly %. These are **empirical**.
//!
//! Price source: <https://platform.claude.com/docs/en/about-claude/pricing>.

use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

const MODELS_JSON: &str = include_str!("../../models.json");

/// Empirical quota weights (per token) for one model. Output is the reference.
#[derive(Debug, Clone, Deserialize)]
pub struct QuotaWeights {
    #[serde(default)]
    pub input: f64,
    #[serde(default = "one")]
    pub output: f64,
    #[serde(default = "default_cache_quota")]
    pub cache_write_5m: f64,
    #[serde(default = "default_cache_quota")]
    pub cache_write_1h: f64,
}

fn one() -> f64 {
    1.0
}

fn default_cache_quota() -> f64 {
    0.11
}

impl Default for QuotaWeights {
    /// Opus/Sonnet-class default: output is the reference (1.0), both cache writes
    /// (5m and 1h) count ~0.11×, everything else ~0.
    fn default() -> Self {
        QuotaWeights {
            input: 0.0,
            output: 1.0,
            cache_write_5m: 0.11,
            cache_write_1h: 0.11,
        }
    }
}

impl QuotaWeights {
    /// Haiku-class: output counts ~0.1× the opus/sonnet reference.
    fn haiku() -> Self {
        QuotaWeights {
            output: 0.1,
            ..QuotaWeights::default()
        }
    }

    /// Fable-class: output counts ~3.3× the opus/sonnet reference (measured on
    /// Max 5×, matches the fable/sonnet output price ratio). Cache keeps the
    /// cross-family invariant cache writes ≈ 0.11 × output (both 5m and 1h).
    fn fable() -> Self {
        QuotaWeights {
            output: 3.3,
            cache_write_5m: 0.36,
            cache_write_1h: 0.36,
            ..QuotaWeights::default()
        }
    }
}

/// All data for one model variant.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    // Prices ($/MTok) — kept for reference/info only, NOT used by quota_units.
    #[allow(dead_code)]
    pub input: f64,
    #[allow(dead_code)]
    pub output: f64,
    #[allow(dead_code)]
    pub cache_write_5m: f64,
    #[allow(dead_code)]
    pub cache_write_1h: f64,
    /// Context-window size in tokens (e.g. 200_000).
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    /// Empirical quota weights used by `quota_units`.
    #[serde(default)]
    pub quota: QuotaWeights,
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
            quota: QuotaWeights::default(),
        }
    }
}

impl Default for Models {
    fn default() -> Self {
        // Prices vary per model, but opus/sonnet share QuotaWeights::default
        // (output=1, cc1h=0.1); haiku uses QuotaWeights::haiku (output=0.1).
        let entry = |input, output, cw5m, cw1h, quota| ModelEntry {
            input,
            output,
            cache_write_5m: cw5m,
            cache_write_1h: cw1h,
            context_window: 200_000,
            quota,
        };
        let std = QuotaWeights::default;
        let mut models = BTreeMap::new();
        models.insert(
            "fable-5".into(),
            entry(10.0, 50.0, 12.5, 20.0, QuotaWeights::fable()),
        );
        models.insert(
            "fable".into(),
            entry(10.0, 50.0, 12.5, 20.0, QuotaWeights::fable()),
        );
        // Mythos 5 (limited availability): same $10/$50 tier as fable; quota is an
        // unverified estimate mirroring fable (no lab data yet).
        models.insert(
            "mythos-5".into(),
            entry(10.0, 50.0, 12.5, 20.0, QuotaWeights::fable()),
        );
        models.insert(
            "mythos".into(),
            entry(10.0, 50.0, 12.5, 20.0, QuotaWeights::fable()),
        );
        models.insert("opus-4-8".into(), entry(5.0, 25.0, 6.25, 10.0, std()));
        models.insert("opus-4-7".into(), entry(5.0, 25.0, 6.25, 10.0, std()));
        models.insert("opus-4-6".into(), entry(5.0, 25.0, 6.25, 10.0, std()));
        models.insert("opus-4-1".into(), entry(15.0, 75.0, 18.75, 30.0, std()));
        models.insert("opus".into(), entry(5.0, 25.0, 6.25, 10.0, std()));
        // Sonnet 5 prices = standard rates ($3/$15) effective 2026-09-01;
        // introductory $2/$10 runs through 2026-08-31 (reference-only).
        models.insert("sonnet-5".into(), entry(3.0, 15.0, 3.75, 6.0, std()));
        models.insert("sonnet-4-6".into(), entry(3.0, 15.0, 3.75, 6.0, std()));
        models.insert("sonnet".into(), entry(3.0, 15.0, 3.75, 6.0, std()));
        models.insert(
            "haiku".into(),
            entry(1.0, 5.0, 1.25, 2.0, QuotaWeights::haiku()),
        );
        models.insert(
            "haiku-3".into(),
            entry(0.8, 4.0, 1.0, 1.6, QuotaWeights::haiku()),
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

    /// Quota-weighted units for one assistant turn, using the empirical `quota`
    /// weights — **excluding cache reads** (never passed in; see module docs).
    pub fn quota_units(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_write_5m: u64,
        cache_write_1h: u64,
    ) -> f64 {
        let q = &self.entry_for(model).quota;
        input as f64 * q.input
            + output as f64 * q.output
            + cache_write_5m as f64 * q.cache_write_5m
            + cache_write_1h as f64 * q.cache_write_1h
    }

    /// Dollar cost for one assistant turn (prices in $/MTok from models.json).
    pub fn cost_usd(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_5m: u64,
        cache_1h: u64,
    ) -> f64 {
        let e = self.entry_for(model);
        (input as f64 * e.input
            + output as f64 * e.output
            + cache_5m as f64 * e.cache_write_5m
            + cache_1h as f64 * e.cache_write_1h)
            / 1_000_000.0
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

    // --- quota_units (empirical quota weights, not prices) ---

    #[test]
    fn quota_units_sonnet_basic() {
        // sonnet quota: input=0, output=1.0, cw5m=0.11, cw1h=0.11
        let units = m().quota_units("claude-sonnet-4-6", 1_000, 500, 200, 100);
        let expected = 1_000.0 * 0.0 + 500.0 * 1.0 + 200.0 * 0.11 + 100.0 * 0.11;
        assert!((units - expected).abs() < 1e-6, "got {units}");
    }

    #[test]
    fn quota_units_zero_tokens() {
        assert_eq!(m().quota_units("claude-sonnet-4-6", 0, 0, 0, 0), 0.0);
    }

    #[test]
    fn quota_units_opus_equals_sonnet() {
        // Empirically opus == sonnet on quota (no 1.67x premium).
        let opus = m().quota_units("claude-opus-4-8", 1000, 1000, 0, 500);
        let sonnet = m().quota_units("claude-sonnet-4-6", 1000, 1000, 0, 500);
        assert!((opus - sonnet).abs() < 1e-9);
    }

    #[test]
    fn quota_units_haiku_output_cheaper_than_sonnet() {
        // Haiku output quota weight is ~0.1x sonnet's.
        let haiku = m().quota_units("claude-haiku-4-5", 0, 1000, 0, 0);
        let sonnet = m().quota_units("claude-sonnet-4-6", 0, 1000, 0, 0);
        assert!(haiku < sonnet, "haiku {haiku} should be < sonnet {sonnet}");
        assert!((haiku - 100.0).abs() < 1e-6); // 1000 * 0.1
    }

    #[test]
    fn quota_units_input_has_no_weight() {
        // input quota weight is 0 → input alone must not contribute.
        assert_eq!(m().quota_units("claude-sonnet-4-6", 10_000, 0, 0, 0), 0.0);
    }

    #[test]
    fn quota_units_unknown_model_uses_default_quota() {
        // An unrecognised model falls back to the default quota weights.
        let d = m().default.quota.clone();
        let u = m().quota_units("some-future-model", 0, 1000, 0, 100);
        let expected = 1000.0 * d.output + 100.0 * d.cache_write_1h;
        assert!((u - expected).abs() < 1e-6, "got {u}");
    }

    #[test]
    fn quota_units_cache_write_1h_contributes() {
        // cw1h quota weight = 0.11.
        let no_1h = m().quota_units("claude-sonnet-4-6", 0, 0, 0, 0);
        let with_1h = m().quota_units("claude-sonnet-4-6", 0, 0, 0, 1_000);
        assert!((with_1h - no_1h - 1_000.0 * 0.11).abs() < 1e-6);
    }

    #[test]
    fn quota_units_both_cache_writes_weigh_the_same() {
        // cw5m aligned to cw1h (0.11): equal token counts must contribute equally.
        let cw5m = m().quota_units("claude-sonnet-4-6", 0, 0, 1_000, 0);
        let cw1h = m().quota_units("claude-sonnet-4-6", 0, 0, 0, 1_000);
        assert!((cw5m - 1_000.0 * 0.11).abs() < 1e-6);
        assert!((cw5m - cw1h).abs() < 1e-9);
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

    #[test]
    fn haiku3_longer_match_beats_generic_haiku() {
        // "haiku-3" key (len 7) must beat "haiku" (len 5) for a haiku-3 model id.
        let models = m();
        let haiku3 = models.entry_for("claude-haiku-3-20251001");
        let haiku = models.entry_for("claude-haiku-4-5");
        // haiku-3 has input price=0.8; generic haiku has input price=1.0
        assert!(
            (haiku3.input - 0.8).abs() < 1e-9,
            "expected haiku-3 pricing"
        );
        assert!((haiku.input - 1.0).abs() < 1e-9, "expected haiku pricing");
    }

    // --- defaults and load ---

    #[test]
    fn model_entry_default_is_sonnet_class() {
        let e = ModelEntry::default();
        assert!((e.input - 3.0).abs() < 1e-9);
        assert!((e.output - 15.0).abs() < 1e-9);
        assert_eq!(e.context_window, 200_000);
        assert!((e.quota.output - 1.0).abs() < 1e-9);
    }

    #[test]
    fn models_default_has_expected_keys() {
        let m = Models::default();
        assert!(m.models.contains_key("opus-4-8"));
        assert!(m.models.contains_key("sonnet-4-6"));
        assert!(m.models.contains_key("haiku"));
        assert!(m.models.contains_key("haiku-3"));
    }

    #[test]
    fn opus_is_more_expensive_than_sonnet() {
        // Prices (reference) still carry the opus premium.
        let m = Models::default();
        let opus = m.entry_for("opus-4-8");
        let sonnet = m.entry_for("sonnet-4-6");
        assert!(opus.input > sonnet.input);
        assert!(opus.output > sonnet.output);
    }

    #[test]
    fn sonnet_is_more_expensive_than_haiku() {
        let m = Models::default();
        let sonnet = m.entry_for("sonnet-4-6");
        let haiku = m.entry_for("haiku");
        assert!(sonnet.input > haiku.input);
        assert!(sonnet.output > haiku.output);
    }

    #[test]
    fn cache_write_5m_is_cheaper_than_1h() {
        let m = Models::default();
        let e = m.entry_for("sonnet");
        assert!(e.cache_write_5m < e.cache_write_1h);

        let e = m.entry_for("opus");
        assert!(e.cache_write_5m < e.cache_write_1h);
    }

    #[test]
    fn context_limit_override_longest_match_wins() {
        let mut overrides = HashMap::new();
        overrides.insert("claude".to_string(), 100_000u64);
        overrides.insert("claude-sonnet".to_string(), 300_000u64);
        overrides.insert("claude-sonnet-4-6".to_string(), 500_000u64);

        let m = Models::default();
        let limit = m.context_limit_for("claude-sonnet-4-6", &overrides);
        assert_eq!(limit, 500_000);
    }

    #[test]
    fn all_models_have_200k_context_by_default() {
        let m = Models::default();
        for entry in m.models.values() {
            assert_eq!(
                entry.context_window, 200_000,
                "all models should have 200k context"
            );
        }
        assert_eq!(m.default.context_window, 200_000);
    }

    #[test]
    fn fable_weighs_3_3x_sonnet_on_quota() {
        let models = m();
        let e = models.entry_for("claude-fable-5");
        assert!((e.input - 10.0).abs() < 1e-9, "expected fable pricing");
        // Measured on Max 5×: fable output ≈ 3.3× sonnet; cache keeps the
        // 0.11×output invariant (0.36 for fable).
        let fable_out = models.quota_units("claude-fable-5", 0, 1000, 0, 0);
        let sonnet_out = models.quota_units("claude-sonnet-4-6", 0, 1000, 0, 0);
        assert!((fable_out / sonnet_out - 3.3).abs() < 1e-9);
        let fable_cc = models.quota_units("claude-fable-5", 0, 0, 0, 1000);
        assert!((fable_cc - 360.0).abs() < 1e-6);
    }

    // --- cost_usd ---

    #[test]
    fn cost_usd_sonnet_input_and_output() {
        // sonnet: input=$3/MTok, output=$15/MTok
        let m = Models::default();
        let input_cost = m.cost_usd("claude-sonnet-4-6", 1_000_000, 0, 0, 0);
        assert!((input_cost - 3.0).abs() < 1e-9, "1M input tokens = $3");
        let output_cost = m.cost_usd("claude-sonnet-4-6", 0, 1_000_000, 0, 0);
        assert!((output_cost - 15.0).abs() < 1e-9, "1M output tokens = $15");
    }

    #[test]
    fn cost_usd_unknown_model_uses_default_pricing() {
        let m = Models::default();
        // default ModelEntry: output=$15/MTok
        let cost = m.cost_usd("some-unknown-model", 0, 1_000_000, 0, 0);
        assert!((cost - 15.0).abs() < 1e-9);
    }

    #[test]
    fn cost_usd_cache_write_prices_applied() {
        // sonnet: cw5m=$3.75/MTok, cw1h=$6/MTok
        let m = Models::default();
        let c5m = m.cost_usd("claude-sonnet-4-6", 0, 0, 1_000_000, 0);
        assert!((c5m - 3.75).abs() < 1e-9, "1M 5m-cache write = $3.75");
        let c1h = m.cost_usd("claude-sonnet-4-6", 0, 0, 0, 1_000_000);
        assert!((c1h - 6.0).abs() < 1e-9, "1M 1h-cache write = $6");
    }

    // --- embedded models.json carries quota weights ---

    #[test]
    fn embedded_json_parses_with_quota() {
        let models = load();
        let sonnet = models.entry_for("claude-sonnet-4-6");
        assert!((sonnet.quota.output - 1.0).abs() < 1e-9);
        assert!((sonnet.quota.cache_write_1h - 0.11).abs() < 1e-9);
        assert!((sonnet.quota.cache_write_5m - 0.11).abs() < 1e-9);
        assert_eq!(sonnet.quota.input, 0.0);
        let haiku = models.entry_for("claude-haiku-4-5");
        assert!((haiku.quota.output - 0.1).abs() < 1e-9);
    }
}
