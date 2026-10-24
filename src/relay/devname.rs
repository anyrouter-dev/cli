//! Device name for `anyr relay`: explicit flag, then the saved name, then the
//! Mac's own name, then a generated friendly one. Pure resolution: the
//! command runner and the random seed are passed in so tests need no Mac.

/// Runs a program and returns its trimmed stdout, or None on any failure.
pub(super) trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub(super) struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(program)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Where the resolved name came from, for the first-start line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    Flag,
    Saved,
    MacName,
    Hostname,
    Random,
}

const MAX_NAME_LEN: usize = 64;

/// Names that identify nobody: factory defaults and placeholders.
pub(super) fn is_generic(name: &str) -> bool {
    let mut norm: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    while norm.ends_with(|c: char| c.is_ascii_digit()) {
        norm.pop();
    }
    // "MacBook-Pro.local" normalizes to "macbookprolocal".
    let norm = norm.strip_suffix("local").unwrap_or(&norm);
    matches!(
        norm,
        "" | "localhost"
            | "mac"
            | "macbook"
            | "macbookpro"
            | "macbookair"
            | "imac"
            | "imacpro"
            | "macmini"
            | "macstudio"
            | "macpro"
            | "mymac"
            | "computer"
            | "unknown"
            | "pc"
            | "desktop"
            | "laptop"
            | "ubuntu"
            | "debian"
            | "linux"
    )
}

const ADJECTIVES: [&str; 16] = [
    "swift", "quiet", "bright", "calm", "bold", "lucky", "gentle", "eager", "brisk", "mellow",
    "nimble", "sunny", "witty", "steady", "cosmic", "amber",
];
const NOUNS: [&str; 16] = [
    "otter", "falcon", "maple", "comet", "heron", "panda", "cedar", "lynx", "orchid", "badger",
    "puffin", "willow", "fox", "marmot", "river", "kestrel",
];

/// "swift-otter-42". The seed picks the words; callers persist the result.
pub(super) fn random_name(seed: u64) -> String {
    let a = ADJECTIVES[(seed % 16) as usize];
    let n = NOUNS[((seed >> 8) % 16) as usize];
    format!("{a}-{n}-{}", (seed >> 16) % 90 + 10)
}

pub(super) fn clean(name: &str) -> String {
    name.trim().chars().take(MAX_NAME_LEN).collect()
}

/// The machine's own name: macOS ComputerName, then LocalHostName, then the
/// hostname; elsewhere the hostname. Generic results are skipped.
fn machine_name(
    runner: &dyn CommandRunner,
    macos: bool,
    env_hostname: Option<&str>,
) -> Option<(String, Source)> {
    let mut candidates: Vec<(Option<String>, Source)> = Vec::new();
    if macos {
        candidates.push((
            runner.run("scutil", &["--get", "ComputerName"]),
            Source::MacName,
        ));
        candidates.push((
            runner.run("scutil", &["--get", "LocalHostName"]),
            Source::Hostname,
        ));
    }
    candidates.push((
        runner
            .run("hostname", &[])
            .or_else(|| env_hostname.map(str::to_string)),
        Source::Hostname,
    ));
    candidates
        .into_iter()
        .filter_map(|(n, s)| Some((clean(&n?), s)))
        .find(|(n, _)| !is_generic(n))
}

pub(super) struct Inputs<'a> {
    pub flag: Option<&'a str>,
    pub saved: Option<&'a str>,
    pub macos: bool,
    pub env_hostname: Option<&'a str>,
    pub seed: u64,
}

/// Flag, then saved, then machine name, then random. The caller saves the
/// result when it differs from `saved`; resolving again with that saved value
/// returns the same name.
pub(super) fn resolve(runner: &dyn CommandRunner, i: &Inputs<'_>) -> (String, Source) {
    let non_empty = |s: Option<&str>| s.map(clean).filter(|s| !s.is_empty());
    if let Some(n) = non_empty(i.flag) {
        return (n, Source::Flag);
    }
    if let Some(n) = non_empty(i.saved) {
        return (n, Source::Saved);
    }
    machine_name(runner, i.macos, i.env_hostname)
        .unwrap_or_else(|| (random_name(i.seed), Source::Random))
}

pub(super) fn seed_now() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    h.finish()
}

/// One line for the first start, saying where the name came from.
pub(super) fn first_start_line(name: &str, source: Source, bin: &str) -> Option<String> {
    let how = match source {
        Source::Flag | Source::Saved => return None,
        Source::MacName => format!("Using this Mac's name \"{name}\""),
        Source::Hostname => format!("Using this machine's host name \"{name}\""),
        Source::Random => format!("Picked the device name \"{name}\""),
    };
    Some(format!("{how} (change with {bin} relay rename <name>)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake(HashMap<String, String>);
    impl CommandRunner for Fake {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            self.0
                .get(format!("{program} {}", args.join(" ")).trim())
                .cloned()
        }
    }
    fn fake(pairs: &[(&str, &str)]) -> Fake {
        Fake(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }
    fn inputs<'a>(flag: Option<&'a str>, saved: Option<&'a str>, macos: bool) -> Inputs<'a> {
        Inputs {
            flag,
            saved,
            macos,
            env_hostname: None,
            seed: 7,
        }
    }

    #[test]
    fn flag_beats_saved_beats_mac_name() {
        let r = fake(&[("scutil --get ComputerName", "Studio")]);
        assert_eq!(
            resolve(&r, &inputs(Some("Desk"), Some("Old"), true)),
            ("Desk".into(), Source::Flag)
        );
        assert_eq!(
            resolve(&r, &inputs(None, Some("Old"), true)),
            ("Old".into(), Source::Saved)
        );
        assert_eq!(
            resolve(&r, &inputs(None, None, true)),
            ("Studio".into(), Source::MacName)
        );
    }

    #[test]
    fn falls_back_through_local_host_name_then_hostname() {
        let r = fake(&[
            ("scutil --get LocalHostName", "studio-lan"),
            ("hostname", "h"),
        ]);
        assert_eq!(resolve(&r, &inputs(None, None, true)).0, "studio-lan");
        let r = fake(&[("hostname", "build-box")]);
        assert_eq!(resolve(&r, &inputs(None, None, true)).0, "build-box");
        // Not macOS: scutil is never asked.
        let r = fake(&[
            ("scutil --get ComputerName", "Studio"),
            ("hostname", "linux-dev"),
        ]);
        assert_eq!(resolve(&r, &inputs(None, None, false)).0, "linux-dev");
    }

    #[test]
    fn generic_names_are_skipped_and_random_is_the_last_resort() {
        let r = fake(&[
            ("scutil --get ComputerName", "MacBook-Pro"),
            ("hostname", "localhost"),
        ]);
        let (n, s) = resolve(&r, &inputs(None, None, true));
        assert_eq!(s, Source::Random);
        assert_eq!(n, random_name(7));
        // A generic ComputerName still yields to a real LocalHostName.
        let r = fake(&[
            ("scutil --get ComputerName", "Mac"),
            ("scutil --get LocalHostName", "Studio"),
        ]);
        assert_eq!(resolve(&r, &inputs(None, None, true)).0, "Studio");
    }

    #[test]
    fn generic_detection() {
        for g in [
            "",
            "  ",
            "localhost",
            "Mac",
            "MacBook-Pro",
            "MacBook-Pro-2",
            "MacBook-Air.local",
            "iMac",
            "Mac mini",
            "Mac-Studio-3",
            "My Mac",
            "Computer",
        ] {
            assert!(is_generic(g), "{g:?} should be generic");
        }
        for ok in [
            "Studio",
            "Jane's MacBook Pro",
            "build-box",
            "Duyet Mac",
            "macbook-duyet",
        ] {
            assert!(!is_generic(ok), "{ok:?} should not be generic");
        }
    }

    #[test]
    fn random_name_shape_and_spread() {
        let n = random_name(12345);
        let parts: Vec<&str> = n.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[2].parse::<u32>().is_ok());
        assert_ne!(random_name(1), random_name(2));
        assert_eq!(random_name(99), random_name(99));
    }

    #[test]
    fn saved_random_name_is_stable_across_runs_and_seeds() {
        let r = fake(&[]);
        let (first, _) = resolve(
            &r,
            &Inputs {
                flag: None,
                saved: None,
                macos: true,
                env_hostname: None,
                seed: 1,
            },
        );
        let (again, s) = resolve(
            &r,
            &Inputs {
                flag: None,
                saved: Some(&first),
                macos: true,
                env_hostname: None,
                seed: 2,
            },
        );
        assert_eq!((again, s), (first, Source::Saved));
    }

    #[test]
    fn saved_name_survives_a_mac_rename() {
        let r = fake(&[("scutil --get ComputerName", "Renamed")]);
        assert_eq!(resolve(&r, &inputs(None, Some("Studio"), true)).0, "Studio");
    }

    #[test]
    fn first_start_line_names_the_source_and_the_way_out() {
        let l = first_start_line("Studio", Source::MacName, "anyr").unwrap();
        assert_eq!(
            l,
            "Using this Mac's name \"Studio\" (change with anyr relay rename <name>)"
        );
        assert!(first_start_line("Desk", Source::Flag, "anyr").is_none());
        assert!(first_start_line("x", Source::Saved, "anyr").is_none());
    }

    #[test]
    fn names_are_trimmed_and_capped_to_the_server_limit() {
        assert_eq!(clean("  Studio \n"), "Studio");
        assert_eq!(clean(&"a".repeat(100)).len(), 64);
    }
}
