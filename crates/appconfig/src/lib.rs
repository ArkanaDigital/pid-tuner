//! App settings: LLM provider selection, API keys (encrypted at rest, in
//! memory only after load), language, token budget and the price table.

pub mod crypto;
pub mod prices;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    Openai,
    Gemini,
    Deepseek,
}

impl Provider {
    pub const ALL: [Provider; 4] = [
        Provider::Anthropic,
        Provider::Openai,
        Provider::Gemini,
        Provider::Deepseek,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::Openai => "openai",
            Provider::Gemini => "gemini",
            Provider::Deepseek => "deepseek",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Provider::Anthropic => "Anthropic (Claude)",
            Provider::Openai => "OpenAI",
            Provider::Gemini => "Google Gemini",
            Provider::Deepseek => "DeepSeek",
        }
    }
}

/// An API key. `Debug`/`Serialize` never reveal it; there is no `Deserialize`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_set(&self) -> bool {
        !self.0.is_empty()
    }
    /// `"sk-a…wxyz"` style: first 4 and last 4 characters when long enough.
    pub fn masked(&self) -> String {
        let n = self.0.chars().count();
        if n == 0 {
            return String::new();
        }
        if n <= 8 {
            return "****".into();
        }
        let head: String = self.0.chars().take(4).collect();
        let tail: String = self.0.chars().skip(n - 4).collect();
        format!("{head}…{tail}")
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret(****)")
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("****")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub model: String,
    /// Override of the API host (proxies); must be https.
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input_per_m: f64,
    pub output_per_m: f64,
    #[serde(default)]
    pub cached_input_per_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u32,
    pub provider: Provider,
    pub providers: BTreeMap<Provider, ProviderConfig>,
    /// "id" (default) or "en"
    pub language: String,
    /// 0 = unlimited
    pub token_budget_per_session: u64,
    pub max_tool_rounds: u8,
    pub prices: BTreeMap<String, ModelPrice>,
    /// In memory only; persisted encrypted as `keys_enc`.
    #[serde(skip)]
    pub keys: BTreeMap<Provider, Secret>,
    /// Problems seen while loading (e.g. keys that could not be decrypted on this machine).
    #[serde(skip)]
    pub load_warnings: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            provider: Provider::Anthropic,
            providers: Provider::ALL
                .iter()
                .map(|p| {
                    (
                        *p,
                        ProviderConfig {
                            model: prices::default_model(*p).into(),
                            base_url: None,
                        },
                    )
                })
                .collect(),
            language: "id".into(),
            token_budget_per_session: 300_000,
            max_tool_rounds: 8,
            prices: prices::defaults(),
            keys: BTreeMap::new(),
            load_warnings: Vec::new(),
        }
    }
}

impl Settings {
    pub fn key(&self, p: Provider) -> Option<&Secret> {
        self.keys.get(&p).filter(|k| k.is_set())
    }
    pub fn provider_config(&self, p: Provider) -> ProviderConfig {
        self.providers.get(&p).cloned().unwrap_or(ProviderConfig {
            model: prices::default_model(p).into(),
            base_url: None,
        })
    }
    pub fn base_url(&self, p: Provider) -> String {
        self.provider_config(p)
            .base_url
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| prices::default_base_url(p).into())
    }
    pub fn model(&self, p: Provider) -> String {
        self.provider_config(p).model
    }
    pub fn price_for(&self, model: &str) -> Option<ModelPrice> {
        self.prices.get(model).copied()
    }
    pub fn view(&self) -> SettingsView {
        SettingsView {
            schema_version: self.schema_version,
            provider: self.provider,
            providers: self.providers.clone(),
            language: self.language.clone(),
            token_budget_per_session: self.token_budget_per_session,
            max_tool_rounds: self.max_tool_rounds,
            prices: self.prices.clone(),
            keys: Provider::ALL
                .iter()
                .map(|p| {
                    (
                        *p,
                        KeyView {
                            set: self.key(*p).is_some(),
                            masked: self.keys.get(p).map(|k| k.masked()).unwrap_or_default(),
                        },
                    )
                })
                .collect(),
            known_models: Provider::ALL
                .iter()
                .map(|p| {
                    (
                        *p,
                        prices::known_models(*p)
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                    )
                })
                .collect(),
            load_warnings: self.load_warnings.clone(),
        }
    }
    /// Apply a UI patch. Returns the list of changed field names.
    pub fn apply(&mut self, patch: SettingsPatch) -> Vec<&'static str> {
        let mut changed = Vec::new();
        if let Some(p) = patch.provider {
            self.provider = p;
            changed.push("provider");
        }
        if let Some(pc) = patch.providers {
            for (p, c) in pc {
                let mut c = c;
                if let Some(u) = &c.base_url {
                    let u = u.trim().to_string();
                    c.base_url = if u.is_empty() { None } else { Some(u) };
                }
                if c.model.trim().is_empty() {
                    c.model = prices::default_model(p).into();
                }
                self.providers.insert(p, c);
            }
            changed.push("providers");
        }
        if let Some(l) = patch.language {
            self.language = if l == "en" { "en".into() } else { "id".into() };
            changed.push("language");
        }
        if let Some(b) = patch.token_budget_per_session {
            self.token_budget_per_session = b;
            changed.push("token_budget_per_session");
        }
        if let Some(r) = patch.max_tool_rounds {
            self.max_tool_rounds = r.clamp(1, 20);
            changed.push("max_tool_rounds");
        }
        if let Some(pr) = patch.prices {
            self.prices = pr;
            changed.push("prices");
        }
        if let Some(keys) = patch.keys {
            for (p, k) in keys {
                match k {
                    Some(v) if !v.trim().is_empty() => {
                        self.keys.insert(p, Secret::new(v.trim()));
                    }
                    Some(_) => {
                        self.keys.remove(&p);
                    }
                    None => {}
                }
            }
            changed.push("keys");
        }
        changed
    }
}

/// What the UI receives: everything except plaintext keys.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub schema_version: u32,
    pub provider: Provider,
    pub providers: BTreeMap<Provider, ProviderConfig>,
    pub language: String,
    pub token_budget_per_session: u64,
    pub max_tool_rounds: u8,
    pub prices: BTreeMap<String, ModelPrice>,
    pub keys: BTreeMap<Provider, KeyView>,
    pub known_models: BTreeMap<Provider, Vec<String>>,
    pub load_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyView {
    pub set: bool,
    pub masked: String,
}

/// UI → backend. `keys[p] = Some("")` clears the key; `None` leaves it untouched.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingsPatch {
    pub provider: Option<Provider>,
    pub providers: Option<BTreeMap<Provider, ProviderConfig>>,
    pub language: Option<String>,
    pub token_budget_per_session: Option<u64>,
    pub max_tool_rounds: Option<u8>,
    pub prices: Option<BTreeMap<String, ModelPrice>>,
    pub keys: Option<BTreeMap<Provider, Option<String>>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// On-disk shape (keys encrypted).
#[derive(Serialize, Deserialize)]
struct Disk {
    #[serde(flatten)]
    settings: Settings,
    #[serde(default)]
    keys_enc: BTreeMap<Provider, crypto::EncBlob>,
}

pub struct SettingsStore {
    path: PathBuf,
    key: [u8; 32],
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            key: crypto::machine_key(),
        }
    }
    pub fn with_key(path: impl Into<PathBuf>, key: [u8; 32]) -> Self {
        Self {
            path: path.into(),
            key,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Missing file → defaults. Undecryptable keys are dropped with a warning; never an error.
    pub fn load(&self) -> Result<Settings, ConfigError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(e) => return Err(e.into()),
        };
        let disk: Disk = serde_json::from_slice(&bytes)?;
        let mut s = disk.settings;
        // fill defaults for providers/prices added after the file was written
        let d = Settings::default();
        for (p, c) in d.providers {
            s.providers.entry(p).or_insert(c);
        }
        for (m, pr) in d.prices {
            s.prices.entry(m).or_insert(pr);
        }
        s.keys.clear();
        s.load_warnings.clear();
        for (p, blob) in disk.keys_enc {
            match crypto::open(&self.key, p.name().as_bytes(), &blob) {
                Ok(plain) => {
                    s.keys
                        .insert(p, Secret::new(String::from_utf8_lossy(&plain).to_string()));
                }
                Err(_) => s.load_warnings.push(format!(
                    "kunci API {} tidak bisa dibuka di mesin ini, masukkan ulang di Settings",
                    p.title()
                )),
            }
        }
        Ok(s)
    }

    /// Atomic write (tmp + rename), keys encrypted, file mode 0600 on Unix.
    pub fn save(&self, s: &Settings) -> Result<(), ConfigError> {
        let keys_enc: BTreeMap<Provider, crypto::EncBlob> = s
            .keys
            .iter()
            .filter(|(_, k)| k.is_set())
            .map(|(p, k)| {
                (
                    *p,
                    crypto::seal(&self.key, p.name().as_bytes(), k.expose().as_bytes()),
                )
            })
            .collect();
        let disk = Disk {
            settings: s.clone(),
            keys_enc,
        };
        let json = serde_json::to_vec_pretty(&disk)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Estimated cost in USD for a usage under a price.
pub fn estimate_cost(input: u64, output: u64, cached: u64, price: &ModelPrice) -> f64 {
    let cached = cached.min(input);
    let uncached = input - cached;
    let cached_rate = price.cached_input_per_m.unwrap_or(price.input_per_m);
    (uncached as f64 * price.input_per_m
        + cached as f64 * cached_rate
        + output as f64 * price.output_per_m)
        / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted_everywhere() {
        let s = Secret::new("sk-ant-api03-verysecretvalue");
        assert_eq!(format!("{s:?}"), "Secret(****)");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"****\"");
        assert_eq!(s.masked(), "sk-a…alue");
        assert_eq!(Secret::new("short").masked(), "****");
        assert_eq!(Secret::default().masked(), "");
        let mut st = Settings::default();
        st.keys
            .insert(Provider::Openai, Secret::new("sk-proj-SECRET-1234567890"));
        let view = serde_json::to_string(&st.view()).unwrap();
        assert!(!view.contains("SECRET"));
        assert!(view.contains("\"masked\":\"sk-p…7890\""));
        let dbg = format!("{st:?}");
        assert!(!dbg.contains("SECRET"));
    }

    #[test]
    fn save_then_load_roundtrips_keys_and_file_has_no_plaintext() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("settings.json");
        let store = SettingsStore::with_key(&path, [7u8; 32]);
        let mut s = Settings::default();
        for (p, k) in [
            (Provider::Anthropic, "sk-ant-test-AAAA"),
            (Provider::Openai, "sk-test-BBBB"),
            (Provider::Gemini, "AIza-test-CCCC"),
            (Provider::Deepseek, "sk-ds-test-DDDD"),
        ] {
            s.keys.insert(p, Secret::new(k));
        }
        s.provider = Provider::Deepseek;
        s.language = "en".into();
        s.token_budget_per_session = 12345;
        store.save(&s).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        for k in [
            "sk-ant-test-AAAA",
            "sk-test-BBBB",
            "AIza-test-CCCC",
            "sk-ds-test-DDDD",
        ] {
            assert!(!raw.contains(k), "plaintext key in file");
        }
        assert!(!path.with_extension("json.tmp").exists());
        let back = store.load().unwrap();
        assert_eq!(
            back.key(Provider::Anthropic).unwrap().expose(),
            "sk-ant-test-AAAA"
        );
        assert_eq!(
            back.key(Provider::Deepseek).unwrap().expose(),
            "sk-ds-test-DDDD"
        );
        assert_eq!(back.provider, Provider::Deepseek);
        assert_eq!(back.language, "en");
        assert_eq!(back.token_budget_per_session, 12345);
        assert!(back.load_warnings.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn different_machine_key_drops_keys_but_keeps_settings() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("settings.json");
        let a = SettingsStore::with_key(&path, [1u8; 32]);
        let mut s = Settings::default();
        s.keys
            .insert(Provider::Anthropic, Secret::new("sk-ant-test-AAAA"));
        s.providers.get_mut(&Provider::Anthropic).unwrap().model = "claude-opus-5".into();
        a.save(&s).unwrap();
        let b = SettingsStore::with_key(&path, [2u8; 32]);
        let back = b.load().unwrap();
        assert!(back.key(Provider::Anthropic).is_none());
        assert_eq!(back.load_warnings.len(), 1);
        assert!(back.load_warnings[0].contains("Anthropic"));
        assert_eq!(back.model(Provider::Anthropic), "claude-opus-5");
    }

    #[test]
    fn missing_file_is_defaults_and_corrupt_file_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let store = SettingsStore::with_key(d.path().join("nope.json"), [0u8; 32]);
        let s = store.load().unwrap();
        assert_eq!(s.provider, Provider::Anthropic);
        assert_eq!(s.language, "id");
        assert_eq!(s.model(Provider::Deepseek), "deepseek-v4-flash");
        assert!(s.prices.contains_key("gpt-5.6-luna"));
        let bad = d.path().join("bad.json");
        std::fs::write(&bad, b"{not json").unwrap();
        assert!(matches!(
            SettingsStore::with_key(&bad, [0u8; 32]).load(),
            Err(ConfigError::Json(_))
        ));
    }

    #[test]
    fn patch_semantics() {
        let mut s = Settings::default();
        s.keys.insert(Provider::Gemini, Secret::new("AIza-keep"));
        let mut keys = BTreeMap::new();
        keys.insert(Provider::Openai, Some("  sk-new  ".to_string()));
        keys.insert(Provider::Gemini, None);
        keys.insert(Provider::Anthropic, Some(String::new()));
        let changed = s.apply(SettingsPatch {
            keys: Some(keys),
            language: Some("fr".into()),
            max_tool_rounds: Some(99),
            ..Default::default()
        });
        assert_eq!(s.key(Provider::Openai).unwrap().expose(), "sk-new");
        assert_eq!(s.key(Provider::Gemini).unwrap().expose(), "AIza-keep");
        assert!(s.key(Provider::Anthropic).is_none());
        assert_eq!(s.language, "id");
        assert_eq!(s.max_tool_rounds, 20);
        assert!(changed.contains(&"keys") && changed.contains(&"language"));
        let mut pc = BTreeMap::new();
        pc.insert(
            Provider::Openai,
            ProviderConfig {
                model: "  ".into(),
                base_url: Some("  ".into()),
            },
        );
        s.apply(SettingsPatch {
            providers: Some(pc),
            ..Default::default()
        });
        assert_eq!(s.model(Provider::Openai), "gpt-5.6-terra");
        assert_eq!(s.base_url(Provider::Openai), "https://api.openai.com");
    }

    #[test]
    fn cost_estimate_math() {
        let p = ModelPrice {
            input_per_m: 3.0,
            output_per_m: 15.0,
            cached_input_per_m: Some(0.3),
        };
        let c = estimate_cost(1000, 500, 200, &p);
        // 800×3 + 200×0.3 + 500×15 = 2400 + 60 + 7500 = 9960 per 1e6
        assert!((c - 0.00996).abs() < 1e-9, "{c}");
        let p2 = ModelPrice {
            input_per_m: 3.0,
            output_per_m: 15.0,
            cached_input_per_m: None,
        };
        assert!((estimate_cost(1000, 0, 5000, &p2) - 0.003).abs() < 1e-12);
    }
}
