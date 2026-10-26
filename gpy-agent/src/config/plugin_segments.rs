//! `[plugin.<segment>]` in `config.toml`: user overrides for script segments.
//!
//! Each table overrides a script segment's `ttl_secs`, its `timeout_ms` (only
//! lower than the manifest's) and any declared setting. The table is typed
//! at load; whether a key is declared, and its value allowed, needs the
//! segment manifest, so [`resolve`] checks that against plugin discovery.
//! `gpy config get/set/list` address the entries as `plugin.<segment>.<key>`.

use crate::config::Config;
use crate::plugin::script::detail;
use crate::plugin::{
    EntryType, PluginDiscovery, RESERVED_SETTING_KEYS, ScriptSegmentManifest, SegmentName,
    SettingKey, SettingValue, TimeoutMs, TtlSecs,
};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Prefix of the dotted `gpy config` keys for plugin segment overrides.
pub const PLUGIN_KEY_PREFIX: &str = "plugin.";

/// One `[plugin.<segment>]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PluginSegmentConfig {
    /// Override of the manifest's `ttl_secs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<TtlSecs>,
    /// Override of the manifest's `timeout_ms`; may only lower it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<TimeoutMs>,
    /// Values for the segment's declared settings.
    #[serde(flatten)]
    pub settings: BTreeMap<SettingKey, SettingValue>,
}

/// Read the table key by key so every error names the key it concerns
/// (`#[serde(flatten)]` would report only "did not match any variant").
impl<'de> Deserialize<'de> for PluginSegmentConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let table = toml::Table::deserialize(deserializer)?;
        let mut config = Self::default();
        for (key, value) in table {
            let fail = |message: String| D::Error::custom(format!("'{key}': {message}"));
            let whole = |raw: &toml::Value| {
                raw.as_integer()
                    .and_then(|number| u32::try_from(number).ok())
                    .ok_or_else(|| fail(format!("expected a whole number, got {raw}")))
            };
            match key.as_str() {
                "ttl_secs" => {
                    config.ttl_secs =
                        Some(TtlSecs::new(whole(&value)?).map_err(|e| fail(detail(e)))?);
                }
                "timeout_ms" => {
                    config.timeout_ms =
                        Some(TimeoutMs::new(whole(&value)?).map_err(|e| fail(detail(e)))?);
                }
                _ => {
                    let setting = SettingKey::new(&key).map_err(|e| fail(detail(e)))?;
                    let parsed = match value {
                        toml::Value::Boolean(flag) => SettingValue::Bool(flag),
                        toml::Value::Integer(number) => SettingValue::Int(number),
                        toml::Value::String(text) => SettingValue::String(text),
                        other => {
                            return Err(fail(format!(
                                "expected a boolean, integer or string, got {}",
                                other.type_str()
                            )));
                        }
                    };
                    config.settings.insert(setting, parsed);
                }
            }
        }
        Ok(config)
    }
}

/// A script segment's effective values: manifest defaults with the user's
/// overrides applied, every declared setting present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedScriptSegment {
    /// Effective `ttl_secs`.
    pub ttl_secs: TtlSecs,
    /// Effective `timeout_ms`.
    pub timeout_ms: TimeoutMs,
    /// Effective value of every declared setting.
    pub settings: BTreeMap<SettingKey, SettingValue>,
}

fn key_error(segment: &SegmentName, key: &str, message: impl std::fmt::Display) -> Error {
    Error::config(format!("{PLUGIN_KEY_PREFIX}{segment}.{key}: {message}"))
}

fn check_timeout(
    segment: &SegmentName,
    manifest: &ScriptSegmentManifest,
    timeout: TimeoutMs,
) -> Result<()> {
    if timeout > manifest.timeout_ms {
        return Err(key_error(
            segment,
            "timeout_ms",
            format!(
                "{timeout} is above the segment's timeout_ms {}; an override may only lower it",
                manifest.timeout_ms
            ),
        ));
    }
    Ok(())
}

/// Apply `overrides` to `manifest`, checking each against its declaration.
///
/// # Errors
/// Returns an error naming `plugin.<segment>.<key>` for an undeclared key, a
/// value of the wrong type or out of range, or a raised `timeout_ms`.
pub fn resolve(
    segment: &SegmentName,
    manifest: &ScriptSegmentManifest,
    overrides: Option<&PluginSegmentConfig>,
) -> Result<ResolvedScriptSegment> {
    let mut resolved = ResolvedScriptSegment {
        ttl_secs: manifest.ttl_secs,
        timeout_ms: manifest.timeout_ms,
        settings: manifest
            .settings
            .iter()
            .map(|(key, decl)| (key.clone(), decl.default_value()))
            .collect(),
    };
    let Some(table) = overrides else {
        return Ok(resolved);
    };
    if let Some(ttl) = table.ttl_secs {
        resolved.ttl_secs = ttl;
    }
    if let Some(timeout) = table.timeout_ms {
        check_timeout(segment, manifest, timeout)?;
        resolved.timeout_ms = timeout;
    }
    for (key, value) in &table.settings {
        let decl = manifest
            .settings
            .get(key)
            .ok_or_else(|| key_error(segment, key.as_str(), "not a declared setting"))?;
        decl.check(value)
            .map_err(|e| key_error(segment, key.as_str(), detail(e)))?;
        resolved.settings.insert(key.clone(), value.clone());
    }
    Ok(resolved)
}

/// The manifest of the discovered script segment `segment`, if any.
#[must_use]
pub fn find_script_segment<'a>(
    discovery: &'a PluginDiscovery,
    segment: &SegmentName,
) -> Option<&'a ScriptSegmentManifest> {
    discovery
        .plugins
        .iter()
        .filter(|plugin| matches!(plugin.manifest.entry_type, EntryType::Script))
        .find_map(|plugin| plugin.script_segments.get(segment))
}

/// Split `plugin.<segment>.<key>` into its parts.
fn parse_key(key: &str) -> Result<(SegmentName, &str)> {
    let malformed = || {
        Error::config(format!(
            "{key}: plugin keys have the form {PLUGIN_KEY_PREFIX}<segment>.<setting>"
        ))
    };
    let rest = key.strip_prefix(PLUGIN_KEY_PREFIX).ok_or_else(malformed)?;
    let (raw_segment, field) = rest.split_once('.').ok_or_else(malformed)?;
    if field.is_empty() || field.contains('.') {
        return Err(malformed());
    }
    let segment = SegmentName::new(raw_segment)
        .map_err(|e| Error::config(format!("{key}: {}", detail(e))))?;
    Ok((segment, field))
}

fn manifest_for<'a>(
    discovery: &'a PluginDiscovery,
    segment: &SegmentName,
) -> Result<&'a ScriptSegmentManifest> {
    find_script_segment(discovery, segment).ok_or_else(|| {
        Error::config(format!(
            "{PLUGIN_KEY_PREFIX}{segment}: no installed script plugin provides segment '{segment}'"
        ))
    })
}

/// Canonical spelling of a plugin key (segment name normalized), as saved.
///
/// # Errors
/// Returns an error if the key is malformed.
pub fn canonical_key(key: &str) -> Result<String> {
    let (segment, field) = parse_key(key)?;
    Ok(format!("{PLUGIN_KEY_PREFIX}{segment}.{field}"))
}

/// Effective value of `plugin.<segment>.<key>` (override or manifest default).
///
/// # Errors
/// Returns an error if the segment is not an installed script segment, the
/// key is not overridable, or the stored override is invalid.
pub fn get(config: &Config, discovery: &PluginDiscovery, key: &str) -> Result<String> {
    let (segment, field) = parse_key(key)?;
    let manifest = manifest_for(discovery, &segment)?;
    let resolved = resolve(&segment, manifest, config.plugin.get(&segment))?;
    match field {
        "ttl_secs" => Ok(resolved.ttl_secs.to_string()),
        "timeout_ms" => Ok(resolved.timeout_ms.to_string()),
        other => SettingKey::new(other)
            .ok()
            .and_then(|setting| resolved.settings.get(&setting))
            .map(ToString::to_string)
            .ok_or_else(|| key_error(&segment, other, "not a declared setting")),
    }
}

/// Set `plugin.<segment>.<key>` from CLI text, validated against the manifest.
///
/// # Errors
/// Returns an error naming the key if it is not overridable or the value is invalid.
pub fn set(config: &mut Config, discovery: &PluginDiscovery, key: &str, raw: &str) -> Result<()> {
    let (segment, field) = parse_key(key)?;
    let manifest = manifest_for(discovery, &segment)?;
    let parse_u32 = |text: &str| -> Result<u32> {
        text.trim().parse().map_err(|e| {
            key_error(
                &segment,
                field,
                format!("'{text}' is not a whole number: {e}"),
            )
        })
    };
    let entry = config.plugin.entry(segment.clone()).or_default();
    match field {
        "ttl_secs" => {
            entry.ttl_secs = Some(
                TtlSecs::new(parse_u32(raw)?).map_err(|e| key_error(&segment, field, detail(e)))?,
            );
        }
        "timeout_ms" => {
            let timeout = TimeoutMs::new(parse_u32(raw)?)
                .map_err(|e| key_error(&segment, field, detail(e)))?;
            check_timeout(&segment, manifest, timeout)?;
            entry.timeout_ms = Some(timeout);
        }
        other => {
            let (setting, decl) = SettingKey::new(other)
                .ok()
                .and_then(|setting| manifest.settings.get_key_value(&setting))
                .ok_or_else(|| key_error(&segment, other, "not a declared setting"))?;
            let value = decl
                .parse_str(raw)
                .map_err(|e| key_error(&segment, other, detail(e)))?;
            entry.settings.insert(setting.clone(), value);
        }
    }
    Ok(())
}

/// Every `plugin.<segment>.<key>` the installed script segments accept, with
/// a description, sorted by key. Manifest descriptions are stripped of
/// control characters, because `gpy config list` prints them.
#[must_use]
pub fn list_keys(discovery: &PluginDiscovery) -> Vec<(String, String)> {
    let mut keys = Vec::new();
    for plugin in &discovery.plugins {
        for (segment, manifest) in &plugin.script_segments {
            let prefix = format!("{PLUGIN_KEY_PREFIX}{segment}.");
            keys.push((
                format!("{prefix}{}", RESERVED_SETTING_KEYS[0]),
                format!(
                    "Seconds a result is reused, 0 disables (plugin {}, default {})",
                    plugin.manifest.id, manifest.ttl_secs
                ),
            ));
            keys.push((
                format!("{prefix}{}", RESERVED_SETTING_KEYS[1]),
                format!(
                    "Script time limit in ms, at most {} (plugin {})",
                    manifest.timeout_ms, plugin.manifest.id
                ),
            ));
            for (key, decl) in &manifest.settings {
                keys.push((
                    format!("{prefix}{key}"),
                    format!(
                        "{} [{}, default {}] (plugin {})",
                        crate::template::sanitize_control_chars(
                            decl.description().unwrap_or("Plugin setting")
                        ),
                        decl.summary(),
                        decl.default_value(),
                        plugin.manifest.id
                    ),
                ));
            }
        }
    }
    keys.sort();
    keys
}

#[cfg(test)]
#[path = "plugin_segments_tests.rs"]
mod tests;
