use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const DATA_PACK_DIR_NAME: &str = "ICMarkets_EST7_2020_present";
const H1_SUFFIX: &str = "_H1_2020_present.tsv";
const M1_SUFFIX: &str = "_M1_2020_present.tsv";
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

fn library_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("cannot resolve app config dir: {error}"))?;
    fs::create_dir_all(&dir).map_err(|error| format!("cannot create config dir: {error}"))?;
    Ok(dir.join("asset-library.json"))
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
    serde_json::from_slice(&bytes).map_err(|error| format!("asset library is invalid JSON: {error}"))
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

pub fn resolve_data_pack_root() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(override_path) = env::var("QUANTFORGE_DATA_PACK") {
        candidates.push(PathBuf::from(override_path));
    }
    if let Ok(home) = env::var("HOME") {
        candidates.push(
            PathBuf::from(&home)
                .join("Documents")
                .join("QuantForge")
                .join(DATA_PACK_DIR_NAME),
        );
    }
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join(DATA_PACK_DIR_NAME));
        candidates.push(cwd.join("..").join(DATA_PACK_DIR_NAME));
        candidates.push(
            cwd.join("Documents")
                .join("QuantForge")
                .join(DATA_PACK_DIR_NAME),
        );
    }
    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .ok_or_else(|| {
            format!(
                "data pack not found; set QUANTFORGE_DATA_PACK or place {DATA_PACK_DIR_NAME} under Documents/QuantForge"
            )
        })
}

fn quantforge_runs_root() -> PathBuf {
    if let Ok(home) = env::var("HOME") {
        let preferred = PathBuf::from(home)
            .join("Documents")
            .join("QuantForge")
            .join("runs");
        if preferred.parent().is_some_and(|parent| parent.is_dir()) {
            return preferred;
        }
    }
    PathBuf::from("runs")
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
        let h1 = pack_root.join(format!("{DEMO_PREFIX}{symbol}{H1_SUFFIX}"));
        let h1_meta = h1.with_extension("metadata.csv");
        let m1 = pack_root.join(format!("{DEMO_PREFIX}{symbol}{M1_SUFFIX}"));
        let m1_meta = m1.with_extension("metadata.csv");
        if !(h1.is_file() && h1_meta.is_file() && m1.is_file() && m1_meta.is_file() && broker_path.is_file())
        {
            continue;
        }
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
