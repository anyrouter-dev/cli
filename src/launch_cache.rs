//! Local copies of what a launch would otherwise ask for every time: the
//! model catalog (`GET /v1/models`) and the opencode major version
//! (`opencode --version`). A warm launch reads both from disk.
//!
//! The catalog is fresh for 5 minutes, the gateway's `max-age`. An older copy,
//! up to a day, is still used and refreshed in the background while the agent
//! runs, the gateway's `stale-while-revalidate`.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::http::CatalogModel;

const CATALOG_FILE: &str = "models-cache.json";
#[cfg(feature = "native")]
const VERSIONS_FILE: &str = "tool-versions.json";
const FRESH_SECS: u64 = 300;
const STALE_SECS: u64 = 86_400;
/// A launch with no copy at all waits at most this long for the catalog.
#[cfg(feature = "native")]
const COLD_FETCH: std::time::Duration = std::time::Duration::from_millis(1500);

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Age {
    Fresh,
    Stale,
}

/// The catalog rows for one gateway and key, on disk next to the config.
pub(crate) struct CatalogCache {
    path: PathBuf,
    row: String,
}

impl CatalogCache {
    pub(crate) fn new(dir: &Path, base: &str, key: Option<&str>) -> Self {
        Self {
            path: dir.join(CATALOG_FILE),
            row: crate::mcp_wiring::cache_key(base, key.unwrap_or("")),
        }
    }

    pub(crate) fn load(&self, now: u64) -> Option<(Vec<CatalogModel>, Age)> {
        let rows = read_json(&self.path)?;
        let row = rows.get(&self.row)?;
        let at = row.get("at")?.as_u64()?;
        if at > now || now - at >= STALE_SECS {
            return None;
        }
        let models =
            crate::http::parse_models_body(&json!({ "data": row.get("data")? }).to_string())
                .ok()?;
        let age = if now - at < FRESH_SECS {
            Age::Fresh
        } else {
            Age::Stale
        };
        Some((models, age))
    }

    pub(crate) fn save(&self, models: &[CatalogModel], now: u64) {
        let mut rows = read_json(&self.path)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if let Some(map) = rows.as_object_mut() {
            map.retain(|_, v| {
                v.get("at")
                    .and_then(Value::as_u64)
                    .is_some_and(|at| now.saturating_sub(at) < STALE_SECS)
            });
            let data: Vec<Value> = models.iter().map(model_row).collect();
            map.insert(self.row.clone(), json!({ "at": now, "data": data }));
        }
        write_json(&self.path, &rows);
    }
}

/// The catalog fields a launch reads, in the API's own shape so the
/// gateway's parser reads them back.
fn model_row(m: &CatalogModel) -> Value {
    let mut row = json!({ "id": m.id, "capabilities": m.capabilities });
    if let Some(name) = &m.name {
        row["name"] = json!(name);
    }
    if let Some(owner) = &m.owned_by {
        row["owned_by"] = json!(owner);
    }
    if let Some(ctx) = m.context_length {
        row["context_length"] = json!(ctx);
    }
    if let Some(max) = m.max_completion_tokens {
        row["top_provider"] = json!({ "max_completion_tokens": max });
    }
    row
}

/// The catalog for a launch, plus a background refresh to keep alive while
/// the agent runs. A dry run reads only what is cached.
pub(crate) struct LaunchCatalog {
    pub(crate) models: Option<Vec<CatalogModel>>,
    pub(crate) refresh: Refresh,
}

/// A running background refresh. Hold it until the agent exits.
#[cfg(feature = "native")]
pub(crate) type Refresh = Option<std::thread::JoinHandle<()>>;
#[cfg(not(feature = "native"))]
pub(crate) type Refresh = Option<()>;

pub(crate) fn catalog_for_launch(
    dir: Option<&Path>,
    base: &str,
    key: Option<&str>,
    dry_run: bool,
) -> LaunchCatalog {
    catalog_with(dir, base, key, dry_run, now_secs(), &fetch)
}

#[cfg(feature = "native")]
fn fetch(base: &str, key: Option<&str>) -> Result<Vec<CatalogModel>, String> {
    crate::http::fetch_models_timeout(base, key, COLD_FETCH)
}

#[cfg(not(feature = "native"))]
fn fetch(base: &str, key: Option<&str>) -> Result<Vec<CatalogModel>, String> {
    crate::http::fetch_models(base, key)
}

type Fetch = dyn Fn(&str, Option<&str>) -> Result<Vec<CatalogModel>, String> + Sync;

fn catalog_with(
    dir: Option<&Path>,
    base: &str,
    key: Option<&str>,
    dry_run: bool,
    now: u64,
    fetch: &'static Fetch,
) -> LaunchCatalog {
    let cache = dir.map(|d| CatalogCache::new(d, base, key));
    let cached = cache.as_ref().and_then(|c| c.load(now));
    let (models, stale) = match cached {
        Some((models, age)) => (Some(models), age == Age::Stale),
        None if dry_run => (None, false),
        None => {
            let fetched = fetch(base, key).ok();
            if let (Some(c), Some(m)) = (&cache, &fetched) {
                c.save(m, now);
            }
            (fetched, false)
        }
    };
    let refresh = match cache {
        Some(cache) if stale && !dry_run => spawn_refresh(cache, base, key, fetch),
        _ => None,
    };
    LaunchCatalog { models, refresh }
}

#[cfg(feature = "native")]
fn spawn_refresh(
    cache: CatalogCache,
    base: &str,
    key: Option<&str>,
    fetch: &'static Fetch,
) -> Refresh {
    let (base, key) = (base.to_string(), key.map(str::to_string));
    std::thread::Builder::new()
        .name("anyr-catalog".into())
        .spawn(move || {
            if let Ok(models) = fetch(&base, key.as_deref()) {
                cache.save(&models, now_secs());
            }
        })
        .ok()
}

#[cfg(not(feature = "native"))]
fn spawn_refresh(_: CatalogCache, _: &str, _: Option<&str>, _: &'static Fetch) -> Refresh {
    None
}

/// `opencode --version` major, cached by binary path, size and mtime so a
/// reinstall or upgrade is probed again.
#[cfg(feature = "native")]
pub(crate) fn opencode_major(dir: Option<&Path>, command: &str) -> Option<u32> {
    let Some(dir) = dir else {
        return crate::spawn::opencode_major_version(command);
    };
    let stamp = binary_stamp(command)?;
    let path = dir.join(VERSIONS_FILE);
    let mut rows = read_json(&path)
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if let Some(row) = rows.get(command) {
        if row.get("stamp").and_then(Value::as_str) == Some(stamp.as_str()) {
            return row.get("major").and_then(Value::as_u64).map(|m| m as u32);
        }
    }
    let major = crate::spawn::opencode_major_version(command);
    rows[command] = json!({ "stamp": stamp, "major": major });
    write_json(&path, &rows);
    major
}

#[cfg(not(feature = "native"))]
pub(crate) fn opencode_major(_dir: Option<&Path>, _command: &str) -> Option<u32> {
    None
}

#[cfg(feature = "native")]
fn binary_stamp(command: &str) -> Option<String> {
    let meta = std::fs::metadata(command).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(format!("{}:{mtime}", meta.len()))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(dir) = path.parent() {
        let _ = crate::config::ensure_private_dir(dir);
    }
    let _ = crate::config::write_private(path, value.to_string().as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://gw.test/api";
    const KEY: &str = "sk-ar-v1-cachetest";

    fn dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("anyr-launch-cache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn model(id: &str, ctx: i64) -> CatalogModel {
        CatalogModel {
            id: id.into(),
            name: Some(format!("{id} name")),
            owned_by: Some("owner".into()),
            context_length: Some(ctx),
            max_completion_tokens: Some(8192),
            capabilities: vec!["tools".into()],
        }
    }

    fn ids(models: &Option<Vec<CatalogModel>>) -> Vec<String> {
        models
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|m| m.id.clone())
            .collect()
    }

    fn counting(_: &str, _: Option<&str>) -> Result<Vec<CatalogModel>, String> {
        Ok(vec![model("fetched/m", 1000)])
    }

    fn offline(_: &str, _: Option<&str>) -> Result<Vec<CatalogModel>, String> {
        panic!("this launch must not touch the network")
    }

    #[test]
    fn a_saved_catalog_reads_back_with_every_launch_field() {
        let d = dir("roundtrip");
        let cache = CatalogCache::new(&d, BASE, Some(KEY));
        cache.save(&[model("a/b", 262_144)], 1_000);
        let (models, age) = cache.load(1_000 + FRESH_SECS - 1).unwrap();
        assert_eq!(age, Age::Fresh);
        let m = &models[0];
        assert_eq!(
            (m.id.as_str(), m.name.as_deref(), m.owned_by.as_deref()),
            ("a/b", Some("a/b name"), Some("owner"))
        );
        assert_eq!(
            (m.context_length, m.max_completion_tokens),
            (Some(262_144), Some(8192))
        );
        assert_eq!(m.capabilities, vec!["tools".to_string()]);
        assert_eq!(cache.load(1_000 + FRESH_SECS).unwrap().1, Age::Stale);
        assert!(
            cache.load(1_000 + STALE_SECS).is_none(),
            "a day-old copy is dropped"
        );
    }

    #[test]
    fn the_cache_never_holds_the_key_and_keys_rows_by_account() {
        let d = dir("keyed");
        CatalogCache::new(&d, BASE, Some(KEY)).save(&[model("a/b", 1)], 1_000);
        let text = std::fs::read_to_string(d.join(CATALOG_FILE)).unwrap();
        assert!(!text.contains(KEY));
        assert!(CatalogCache::new(&d, BASE, Some("sk-ar-v1-other"))
            .load(1_001)
            .is_none());
    }

    #[test]
    fn warm_launch_and_dry_run_never_fetch() {
        let d = dir("warm");
        let now = now_secs();
        CatalogCache::new(&d, BASE, Some(KEY)).save(&[model("cached/m", 1)], now);
        let warm = catalog_with(Some(&d), BASE, Some(KEY), false, now + 1, &offline);
        assert_eq!(ids(&warm.models), vec!["cached/m"]);
        assert!(warm.refresh.is_none(), "a fresh copy is not refreshed");
        let cold_dry = catalog_with(Some(&dir("cold-dry")), BASE, Some(KEY), true, now, &offline);
        assert!(cold_dry.models.is_none());
    }

    #[test]
    fn a_stale_copy_is_used_now_and_refreshed_in_the_background() {
        let d = dir("stale");
        let now = now_secs();
        CatalogCache::new(&d, BASE, Some(KEY)).save(&[model("old/m", 1)], now - FRESH_SECS - 5);
        let launch = catalog_with(Some(&d), BASE, Some(KEY), false, now, &counting);
        assert_eq!(ids(&launch.models), vec!["old/m"], "no wait on the network");
        launch.refresh.expect("refresh started").join().unwrap();
        let (models, age) = CatalogCache::new(&d, BASE, Some(KEY))
            .load(now_secs())
            .unwrap();
        assert_eq!((models[0].id.as_str(), age), ("fetched/m", Age::Fresh));
    }

    #[test]
    fn a_cold_launch_fetches_once_and_saves() {
        let d = dir("cold");
        let launch = catalog_with(Some(&d), BASE, Some(KEY), false, now_secs(), &counting);
        assert_eq!(ids(&launch.models), vec!["fetched/m"]);
        assert!(launch.refresh.is_none());
        assert!(CatalogCache::new(&d, BASE, Some(KEY))
            .load(now_secs())
            .is_some());
    }

    #[cfg(unix)]
    #[test]
    fn opencode_version_is_probed_once_per_binary() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("oc");
        let bin = d.join("opencode");
        let log = d.join("probes");
        let script = format!("#!/bin/sh\necho x >> '{}'\necho 'v2.0.24'\n", log.display());
        std::fs::write(&bin, &script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cmd = bin.to_str().unwrap();
        assert_eq!(opencode_major(Some(&d), cmd), Some(2));
        assert_eq!(opencode_major(Some(&d), cmd), Some(2));
        let probes = || std::fs::read_to_string(&log).unwrap().lines().count();
        assert_eq!(probes(), 1, "second launch reads the cache");
        std::fs::write(&bin, script.replace("v2.0.24", "1.4.0 ")).unwrap();
        assert_eq!(
            opencode_major(Some(&d), cmd),
            Some(1),
            "an upgrade is probed again"
        );
        assert_eq!(probes(), 2);
    }
}
