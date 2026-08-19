//! Static assertions that this repo stays on 0.1.x and ships release files.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn quoted_after_key(text: &str, key: &str) -> String {
    let mut search = text;
    while let Some(idx) = search.find(key) {
        let after = search[idx + key.len()..].trim_start();
        let after = after
            .strip_prefix(':')
            .or_else(|| after.strip_prefix('='))
            .unwrap_or(after)
            .trim_start();
        if let Some(rest) = after.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return rest[..end].to_string();
            }
        }
        search = &search[idx + key.len()..];
    }
    panic!("did not find quoted value for {key:?}");
}

fn cargo_package_version(toml: &str) -> String {
    let mut in_package = false;
    for line in toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_package = t == "[package]";
            continue;
        }
        if in_package && t.starts_with("version") {
            return quoted_after_key(t, "version");
        }
    }
    panic!("Cargo.toml [package] version missing");
}

fn assert_0_1(label: &str, version: &str) {
    assert!(
        version.starts_with("0.1."),
        "{label} version {version:?} does not match ^0\\.1\\."
    );
}

fn must_exist(rel: &str) {
    let path = root().join(rel);
    assert!(path.is_file(), "missing required file {}", path.display());
}

fn contains_true(hay: &str, key: &str) {
    let compact = hay.replace([' ', '\n', '\t', '\r'], "");
    let needle = format!("\"{key}\":true");
    assert!(
        compact.contains(&needle),
        "expected {key} = true in config, compact={compact}"
    );
}

#[test]
fn versions_are_0_1_x() {
    let cargo = cargo_package_version(&read("Cargo.toml"));
    let pkg = quoted_after_key(&read("package.json"), "\"version\"");
    let manifest = quoted_after_key(&read(".release-please-manifest.json"), "\".\"");
    let lib = read("src/lib.rs");

    assert_0_1("Cargo.toml", &cargo);
    assert_0_1("package.json", &pkg);
    assert_0_1(".release-please-manifest.json", &manifest);
    assert_eq!(
        cargo, pkg,
        "Cargo.toml and package.json versions must match"
    );
    assert_eq!(
        cargo, manifest,
        "Cargo.toml and release-please manifest versions must match"
    );
    assert!(
        lib.contains("env!(\"CARGO_PKG_VERSION\")"),
        "lib.rs VERSION must follow Cargo.toml"
    );
}

#[test]
fn release_please_manifest_has_no_0_2() {
    let manifest = read(".release-please-manifest.json");
    assert!(
        !manifest.contains("\"version\": \"0.2"),
        "release-please manifest must not contain \"version\": \"0.2\": {manifest}"
    );
    assert!(
        !manifest.contains("\"0.2"),
        "release-please manifest must stay on 0.1.x: {manifest}"
    );
}

#[test]
fn release_please_locks_pre_major_bumps() {
    let config = read("release-please-config.json");
    contains_true(&config, "bump-patch-for-minor-pre-major");
    contains_true(&config, "bump-minor-pre-major");
    contains_true(&config, "include-v-in-tag");
    assert!(
        config.contains("\"@anyr/cli\""),
        "package-name should be @anyr/cli"
    );
    assert!(
        config.contains("Cargo.toml"),
        "extra-files must include Cargo.toml"
    );
    let compact = config.replace([' ', '\n', '\t', '\r'], "");
    assert!(
        compact.contains("\"release-type\":\"node\""),
        "node release-type updates CHANGELOG + package.json"
    );
}

#[test]
fn workflow_files_exist_and_do_not_auto_merge() {
    let workflows = [
        ".github/workflows/ci.yml",
        ".github/workflows/release-please.yml",
        ".github/workflows/release-binaries.yml",
        ".github/workflows/npm-publish.yml",
    ];
    for rel in workflows {
        must_exist(rel);
        let body = read(rel);
        assert!(
            !body.contains("gh pr merge"),
            "{rel} must not auto-merge (found gh pr merge)"
        );
        assert!(
            !body.contains("--auto"),
            "{rel} must not auto-merge (found --auto)"
        );
    }

    let binaries = read(".github/workflows/release-binaries.yml");
    assert!(
        binaries.contains("anyr-linux-x86_64"),
        "linux x86_64 asset is required"
    );

    let npm = read(".github/workflows/npm-publish.yml");
    assert!(
        npm.contains("npm publish --access public --tag next"),
        "npm must publish to the next tag"
    );

    let rp = read(".github/workflows/release-please.yml");
    assert!(
        rp.contains("googleapis/release-please-action"),
        "release-please workflow must use googleapis/release-please-action"
    );
}

#[test]
fn npm_package_is_public_next_tag() {
    let pkg = read("package.json");
    let compact = pkg.replace([' ', '\n', '\t', '\r'], "");
    assert!(compact.contains("\"access\":\"public\""));
    assert!(compact.contains("\"tag\":\"next\""));
    assert!(compact.contains("\"cli\":\"scripts/npx-anyr.js\""));
    assert!(compact.contains("\"anyr\":\"scripts/npx-anyr.js\""));
    assert!(compact.contains("\"anyrouter\":\"scripts/npx-anyr.js\""));
    assert!(compact.contains("\"ar\":\"scripts/npx-anyr.js\""));
    assert!(pkg.contains("git+https://github.com/anyrouter-dev/cli.git"));
}

#[test]
fn wrapper_and_install_scripts_exist() {
    must_exist("scripts/npx-anyr.js");
    must_exist("scripts/install.sh");
    must_exist("scripts/install-hooks.sh");
    must_exist("scripts/check-coauthors.sh");
    must_exist("LICENSE");
    must_exist("CHANGELOG.md");
    must_exist("README.md");
    let wrapper = read("scripts/npx-anyr.js");
    assert!(wrapper.contains("github.com/anyrouter-dev/cli"));
    assert!(wrapper.contains("binaries"));
    let install = read("scripts/install.sh");
    assert!(install.contains("--channel"));
    assert!(install.contains("stable"));
    assert!(install.contains("beta"));
}

#[test]
fn hook_source_contains_both_coauthor_emails() {
    let hook = Path::new(".githooks").join("prepare-commit-msg");
    let body = read(hook.to_str().unwrap());
    assert!(
        body.contains("Co-authored-by: Duyet Le <me@duyet.net>"),
        "missing Duyet Le trailer"
    );
    assert!(
        body.contains("Co-authored-by: duyetbot <bot@duyet.net>"),
        "missing duyetbot trailer"
    );
    let check = read("scripts/check-coauthors.sh");
    assert!(check.contains("me@duyet.net"));
    assert!(check.contains("bot@duyet.net"));
}
