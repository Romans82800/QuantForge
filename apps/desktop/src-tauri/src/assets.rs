use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Preferred pack folder names, newest-first. QUANTFORGE_DATA_PACK overrides.
const DATA_PACK_CANDIDATES: &[&str] = &[
    "ICMarkets_EST7_2016_present",
    "ICMarkets_EST7_2020_present",
];
const DEMO_PREFIX: &str = "ICMarketsSC-Demo_";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetProfile {
    id: String,
    name: String,
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    m1_data_path: Option<String>,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    broker_path: String,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolPack {
    symbol: String,
    data_path: String,
    metadata_path: String,
    m1_data_path: String,
    m1_metadata_path: String,
    quote_path: Option<String>,
    broker_path: String,
    default_databank_path: String,
    pack_root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AssetLibrary {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    assets: Vec<AssetProfile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearchRangeProfile {
    pub id: String,
    pub name: String,
    pub ranges: quantforge_discover::SearchRangeProfile,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SearchRangeLibrary {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    profiles: Vec<SavedSearchRangeProfile>,
}

/// Reusable Discover form snapshot. The form remains client-owned because its
/// path fields are machine-local, while this library makes the snapshot durable
/// across application restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedDiscoverProfile {
    pub id: String,
    pub name: String,
    pub settings: Value,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DiscoverProfileLibrary {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    profiles: Vec<SavedDiscoverProfile>,
}

fn library_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("cannot resolve app config dir: {error}"))?;
    fs::create_dir_all(&dir).map_err(|error| format!("cannot create config dir: {error}"))?;
    Ok(dir.join("asset-library.json"))
}

fn search_range_library_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("cannot resolve app config dir: {error}"))?;
    fs::create_dir_all(&dir).map_err(|error| format!("cannot create app config dir: {error}"))?;
    Ok(dir.join("search-range-profiles.json"))
}

fn discover_profile_library_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("cannot resolve app config dir: {error}"))?;
    fs::create_dir_all(&dir).map_err(|error| format!("cannot create app config dir: {error}"))?;
    Ok(dir.join("discover-profiles.json"))
}

fn load_search_range_library(app: &AppHandle) -> Result<SearchRangeLibrary, String> {
    let path = search_range_library_path(app)?;
    if !path.exists() {
        return Ok(SearchRangeLibrary {
            schema_version: 1,
            profiles: Vec::new(),
        });
    }
    serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("cannot read search profiles: {error}"))?,
    )
    .map_err(|error| format!("search profiles are invalid: {error}"))
}

fn save_search_range_library(app: &AppHandle, library: &SearchRangeLibrary) -> Result<(), String> {
    let path = search_range_library_path(app)?;
    let bytes = serde_json::to_vec_pretty(library)
        .map_err(|error| format!("cannot serialize search profiles: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("cannot save search profiles: {error}"))
}

fn load_discover_profile_library(app: &AppHandle) -> Result<DiscoverProfileLibrary, String> {
    let path = discover_profile_library_path(app)?;
    if !path.exists() {
        return Ok(DiscoverProfileLibrary {
            schema_version: 1,
            profiles: Vec::new(),
        });
    }
    serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("cannot read Discover profiles: {error}"))?,
    )
    .map_err(|error| format!("Discover profiles are invalid: {error}"))
}

fn save_discover_profile_library(
    app: &AppHandle,
    library: &DiscoverProfileLibrary,
) -> Result<(), String> {
    let path = discover_profile_library_path(app)?;
    let bytes = serde_json::to_vec_pretty(library)
        .map_err(|error| format!("cannot serialize Discover profiles: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("cannot save Discover profiles: {error}"))
}

fn load_library(app: &AppHandle) -> Result<AssetLibrary, String> {
    let path = library_path(app)?;
    if !path.exists() {
        return Ok(AssetLibrary {
            schema_version: 1,
            assets: Vec::new(),
        });
    }
    let bytes = fs::read(&path).map_err(|error| format!("cannot read asset library: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("asset library is invalid JSON: {error}"))
}

fn save_library(app: &AppHandle, library: &AssetLibrary) -> Result<(), String> {
    let path = library_path(app)?;
    let mut payload = library.clone();
    payload.schema_version = 1;
    payload
        .assets
        .sort_by_key(|asset| asset.name.to_ascii_lowercase());
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("cannot serialize asset library: {error}"))?;
    fs::write(&path, bytes).map_err(|error| format!("cannot write asset library: {error}"))
}

fn validate_asset(asset: &AssetProfile) -> Result<(), String> {
    if asset.name.trim().is_empty() {
        return Err("asset name is required".into());
    }
    if asset.data_path.trim().is_empty() {
        return Err("decision OHLC path is required".into());
    }
    if asset.broker_path.trim().is_empty() {
        return Err("broker profile path is required".into());
    }
    Ok(())
}

pub(crate) fn user_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn resolve_data_pack_root() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(override_path) = env::var("QUANTFORGE_DATA_PACK") {
        candidates.push(PathBuf::from(override_path));
    }
    if let Some(home) = user_home_dir() {
        for name in DATA_PACK_CANDIDATES {
            candidates.push(home.join("Documents").join("QuantForge").join(name));
        }
    }
    if let Ok(cwd) = env::current_dir() {
        for name in DATA_PACK_CANDIDATES {
            candidates.push(cwd.join(name));
            candidates.push(cwd.join("..").join(name));
            candidates.push(cwd.join("Documents").join("QuantForge").join(name));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .ok_or_else(|| {
            format!(
                "data pack not found; set QUANTFORGE_DATA_PACK or place one of [{}] under Documents/QuantForge",
                DATA_PACK_CANDIDATES.join(", ")
            )
        })
}

pub(crate) fn quantforge_runs_root() -> PathBuf {
    if let Some(home) = user_home_dir() {
        let preferred = home.join("Documents").join("QuantForge").join("runs");
        if preferred.parent().is_some_and(|parent| parent.is_dir()) {
            return preferred;
        }
    }
    PathBuf::from("runs")
}

/// Locate `ICMarketsSC-Demo_{symbol}_{H1|M1}_*.tsv`, preferring the lexicographically
/// last match so `2016_present` wins over `2020_present` if both somehow coexist.
fn find_pack_tsv(pack_root: &Path, symbol: &str, timeframe: &str) -> Option<PathBuf> {
    let prefix = format!("{DEMO_PREFIX}{symbol}_{timeframe}_");
    let mut matches: Vec<PathBuf> = fs::read_dir(pack_root)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tsv"))
        })
        .collect();
    matches.sort();
    matches.pop()
}

fn scan_symbols(pack_root: &Path) -> Result<Vec<SymbolPack>, String> {
    let entries = fs::read_dir(pack_root)
        .map_err(|error| format!("cannot read data pack {}: {error}", pack_root.display()))?;
    let mut symbols = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read data pack entry: {error}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".broker.json") {
            continue;
        }
        let symbol = name.trim_end_matches(".broker.json").to_owned();
        if symbol.is_empty() {
            continue;
        }
        let broker_path = entry.path();
        let Some(h1) = find_pack_tsv(pack_root, &symbol, "H1") else {
            continue;
        };
        let Some(m1) = find_pack_tsv(pack_root, &symbol, "M1") else {
            continue;
        };
        let h1_meta = h1.with_extension("metadata.csv");
        let m1_meta = m1.with_extension("metadata.csv");
        if !(h1_meta.is_file() && m1_meta.is_file() && broker_path.is_file()) {
            continue;
        }
        let quote_path = infer_pack_quote_path(&m1);
        let default_databank_path = quantforge_runs_root()
            .join(&symbol)
            .join("Databank")
            .join("quantforge-databank.json");
        symbols.push(SymbolPack {
            symbol: symbol.clone(),
            data_path: canonical_display(&h1),
            metadata_path: canonical_display(&h1_meta),
            m1_data_path: canonical_display(&m1),
            m1_metadata_path: canonical_display(&m1_meta),
            quote_path: quote_path.as_ref().map(|path| canonical_display(path)),
            broker_path: canonical_display(&broker_path),
            default_databank_path: default_databank_path.display().to_string(),
            pack_root: canonical_display(pack_root),
        });
    }
    symbols.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    if symbols.is_empty() {
        return Err(format!(
            "no complete symbols found in {}",
            pack_root.display()
        ));
    }
    Ok(symbols)
}

fn infer_pack_quote_path(m1_path: &Path) -> Option<PathBuf> {
    let stem = m1_path.file_stem()?.to_str()?;
    let candidate = m1_path.with_file_name(format!("{stem}.quotes.csv"));
    candidate.is_file().then_some(candidate)
}

fn canonical_display(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

#[tauri::command]
pub fn list_symbols() -> Result<Vec<SymbolPack>, String> {
    let root = resolve_data_pack_root()?;
    scan_symbols(&root)
}

#[tauri::command]
pub fn list_search_range_profiles(app: AppHandle) -> Result<Vec<SavedSearchRangeProfile>, String> {
    let mut profiles = load_search_range_library(&app)?.profiles;
    profiles.sort_by_key(|profile| profile.name.to_ascii_lowercase());
    Ok(profiles)
}

#[tauri::command]
pub fn save_search_range_profile(
    app: AppHandle,
    profile: SavedSearchRangeProfile,
) -> Result<Vec<SavedSearchRangeProfile>, String> {
    if profile.name.trim().is_empty() {
        return Err("profile name is required".into());
    }
    profile
        .ranges
        .validate()
        .map_err(|error| error.to_string())?;
    let mut library = load_search_range_library(&app)?;
    let id = if profile.id.trim().is_empty() {
        format!("range-{}", chrono::Utc::now().timestamp_millis())
    } else {
        profile.id.clone()
    };
    let next = SavedSearchRangeProfile {
        id: id.clone(),
        name: profile.name.trim().to_owned(),
        ranges: profile.ranges,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Some(existing) = library.profiles.iter_mut().find(|item| item.id == id) {
        *existing = next;
    } else {
        library.profiles.push(next);
    }
    library.schema_version = 1;
    save_search_range_library(&app, &library)?;
    list_search_range_profiles(app)
}

#[tauri::command]
pub fn delete_search_range_profile(
    app: AppHandle,
    id: String,
) -> Result<Vec<SavedSearchRangeProfile>, String> {
    let mut library = load_search_range_library(&app)?;
    let before = library.profiles.len();
    library.profiles.retain(|profile| profile.id != id);
    if library.profiles.len() == before {
        return Err("search profile was not found".into());
    }
    save_search_range_library(&app, &library)?;
    list_search_range_profiles(app)
}

#[tauri::command]
pub fn list_discover_profiles(app: AppHandle) -> Result<Vec<SavedDiscoverProfile>, String> {
    let mut profiles = load_discover_profile_library(&app)?.profiles;
    profiles.sort_by_key(|profile| profile.name.to_ascii_lowercase());
    Ok(profiles)
}

#[tauri::command]
pub fn save_discover_profile(
    app: AppHandle,
    profile: SavedDiscoverProfile,
) -> Result<Vec<SavedDiscoverProfile>, String> {
    if profile.name.trim().is_empty() {
        return Err("Discover profile name is required".into());
    }
    if !profile.settings.is_object() {
        return Err("Discover profile settings must be an object".into());
    }
    let mut library = load_discover_profile_library(&app)?;
    let id = if profile.id.trim().is_empty() {
        format!("discover-{}", chrono::Utc::now().timestamp_millis())
    } else {
        profile.id.clone()
    };
    let next = SavedDiscoverProfile {
        id: id.clone(),
        name: profile.name.trim().to_owned(),
        settings: profile.settings,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Some(existing) = library.profiles.iter_mut().find(|item| item.id == id) {
        *existing = next;
    } else {
        library.profiles.push(next);
    }
    library.schema_version = 1;
    save_discover_profile_library(&app, &library)?;
    list_discover_profiles(app)
}

#[tauri::command]
pub fn delete_discover_profile(
    app: AppHandle,
    id: String,
) -> Result<Vec<SavedDiscoverProfile>, String> {
    let mut library = load_discover_profile_library(&app)?;
    let before = library.profiles.len();
    library.profiles.retain(|profile| profile.id != id);
    if library.profiles.len() == before {
        return Err("Discover profile was not found".into());
    }
    save_discover_profile_library(&app, &library)?;
    list_discover_profiles(app)
}

#[tauri::command]
pub fn list_assets(app: AppHandle) -> Result<Vec<AssetProfile>, String> {
    // Prefer auto-scanned pack symbols surfaced as AssetProfile for older UI.
    if let Ok(symbols) = list_symbols() {
        let mapped = symbols
            .into_iter()
            .map(|symbol| AssetProfile {
                id: symbol.symbol.clone(),
                name: symbol.symbol,
                data_path: symbol.data_path,
                metadata_path: Some(symbol.metadata_path),
                source_timezone: None,
                m1_data_path: Some(symbol.m1_data_path),
                m1_metadata_path: Some(symbol.m1_metadata_path),
                m1_source_timezone: None,
                broker_path: symbol.broker_path,
                updated_at: chrono::Utc::now().to_rfc3339(),
            })
            .collect();
        return Ok(mapped);
    }
    Ok(load_library(&app)?.assets)
}

#[tauri::command]
pub fn upsert_asset(app: AppHandle, asset: AssetProfile) -> Result<Vec<AssetProfile>, String> {
    validate_asset(&asset)?;
    let mut library = load_library(&app)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = asset;
    if next.id.trim().is_empty() {
        next.id = format!(
            "{}-{}",
            slug(&next.name),
            chrono::Utc::now().timestamp_millis()
        );
    }
    next.name = next.name.trim().to_owned();
    next.updated_at = now;
    if let Some(existing) = library
        .assets
        .iter_mut()
        .find(|entry| entry.id == next.id || entry.name.eq_ignore_ascii_case(&next.name))
    {
        next.id = existing.id.clone();
        *existing = next;
    } else {
        library.assets.push(next);
    }
    save_library(&app, &library)?;
    Ok(library.assets)
}

#[tauri::command]
pub fn delete_asset(app: AppHandle, id: String) -> Result<Vec<AssetProfile>, String> {
    let mut library = load_library(&app)?;
    let before = library.assets.len();
    library.assets.retain(|asset| asset.id != id);
    if library.assets.len() == before {
        return Err(format!("asset `{id}` was not found"));
    }
    save_library(&app, &library)?;
    Ok(library.assets)
}

fn slug(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = cleaned
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "asset".into()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_local_icmarkets_pack_when_present() {
        let Ok(root) = resolve_data_pack_root() else {
            return;
        };
        let symbols = scan_symbols(&root).expect("pack should contain complete symbols");
        assert!(symbols.iter().any(|symbol| symbol.symbol == "AUDUSD"));
        let aud = symbols
            .iter()
            .find(|symbol| symbol.symbol == "AUDUSD")
            .unwrap();
        assert!(Path::new(&aud.data_path).is_file());
        assert!(Path::new(&aud.m1_data_path).is_file());
        assert!(Path::new(&aud.broker_path).is_file());
    }
}
