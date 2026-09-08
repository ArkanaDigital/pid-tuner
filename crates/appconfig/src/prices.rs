//! Default price table (USD per 1M tokens) and known model ids per provider.
//! Values were read from the vendors' public pricing on 2026-09-08 and are
//! editable estimates in the UI.

use crate::{ModelPrice, Provider};
use std::collections::BTreeMap;

pub fn defaults() -> BTreeMap<String, ModelPrice> {
    let p = |i: f64, o: f64| ModelPrice {
        input_per_m: i,
        output_per_m: o,
        cached_input_per_m: Some(i * 0.1),
    };
    [
        ("claude-haiku-4-5", p(1.0, 5.0)),
        ("claude-sonnet-5", p(2.0, 10.0)),
        ("claude-opus-5", p(5.0, 25.0)),
        ("claude-fable-5-1", p(10.0, 50.0)),
        ("gpt-5.6-luna", p(0.20, 1.20)),
        ("gpt-5.6-terra", p(2.0, 12.0)),
        ("gpt-5.6-sol", p(4.0, 20.0)),
        ("gpt-6-astra", p(10.0, 50.0)),
        ("gemini-3.5-flash-lite", p(0.25, 1.50)),
        ("gemini-3.8-flash", p(0.50, 3.00)),
        ("gemini-2.5-pro", p(1.25, 10.0)),
        ("deepseek-v4-flash", p(0.28, 0.84)),
        ("deepseek-v4-pro", p(0.99, 2.97)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

pub fn known_models(p: Provider) -> &'static [&'static str] {
    match p {
        Provider::Anthropic => &[
            "claude-sonnet-5",
            "claude-haiku-4-5",
            "claude-opus-5",
            "claude-fable-5-1",
        ],
        Provider::Openai => &[
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.6-sol",
            "gpt-6-astra",
        ],
        Provider::Gemini => &[
            "gemini-3.8-flash",
            "gemini-3.5-flash-lite",
            "gemini-2.5-pro",
        ],
        Provider::Deepseek => &["deepseek-v4-flash", "deepseek-v4-pro"],
    }
}

pub fn default_model(p: Provider) -> &'static str {
    known_models(p)[0]
}

pub fn default_base_url(p: Provider) -> &'static str {
    match p {
        Provider::Anthropic => "https://api.anthropic.com",
        Provider::Openai => "https://api.openai.com",
        Provider::Gemini => "https://generativelanguage.googleapis.com",
        Provider::Deepseek => "https://api.deepseek.com",
    }
}
