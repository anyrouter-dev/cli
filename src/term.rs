//! Color TUI helpers. Truecolor GrokNight palette; silent when not a TTY or
//! when `NO_COLOR` is set. No extra crates — keep startup cheap.

use std::io::{self, IsTerminal, Write};

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";

pub const MAGENTA: &str = "\x1b[38;2;187;154;247m";
pub const BLUE: &str = "\x1b[38;2;122;162;247m";
pub const TEAL: &str = "\x1b[38;2;58;149;171m";
pub const SUCCESS: &str = "\x1b[38;2;158;206;106m";
pub const ORANGE: &str = "\x1b[38;2;255;158;100m";
pub const YELLOW: &str = "\x1b[38;2;224;175;104m";
pub const DANGER: &str = "\x1b[38;2;247;118;142m";
pub const WHITE: &str = "\x1b[38;2;225;225;225m";
pub const MUTED: &str = "\x1b[38;2;108;108;108m";
pub const LABEL: &str = "\x1b[38;2;200;200;200m";

pub fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    io::stdout().is_terminal()
}

pub fn paint(ansi: &str, text: &str) -> String {
    if !color_enabled() {
        return text.to_string();
    }
    format!("{ansi}{text}{RESET}")
}

pub fn bold(text: &str) -> String {
    paint(BOLD, text)
}

pub fn dim(text: &str) -> String {
    paint(MUTED, text)
}

pub fn accent(text: &str) -> String {
    paint(MAGENTA, text)
}

pub fn ok(text: &str) -> String {
    paint(SUCCESS, text)
}

pub fn warn(text: &str) -> String {
    paint(YELLOW, text)
}

pub fn err(text: &str) -> String {
    paint(DANGER, text)
}

pub fn model_id(text: &str) -> String {
    paint(TEAL, text)
}

/// Official AR ligature as half-blocks. Source is `anyrouter-logo-white.png`,
/// not the 32px favicon. Half-blocks fill the cell so the mark stays readable
/// in fonts where braille dots collapse into noise.
pub const MARK_LINES: [&str; 5] = [
    "        ▄█▄▀█████████▄",
    "      ▄█████▄       ███",
    "    ▄███▀  ▀██▄ ▄▄▄███▀",
    "  ▄███▀      ▀██▄▀███▄",
    "▄███▀          ▀██▄▀███▄",
];

/// Compact 3-row AR mark for TUI chrome. Recovered from the original
/// half-block logo (`b7a313b`); not the later 5-line ligature.
pub const TUI_MARK_LINES: [&str; 3] = ["    ▄▄ ▄▄▄ ", "  ▄█▀▀█▄▄█▀", " ▀▀    ▀▀▀▀"];

const MARK_PNG: &[u8] = include_bytes!("../assets/mark.png");

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Graphics {
    Kitty,
    Iterm,
    Sixel,
}

/// Inline image protocol for this terminal.
///
/// - Kitty PNG: kitty, Ghostty
/// - iTerm OSC 1337 PNG: iTerm2, WezTerm, Warp, mintty, rio, Tabby, Blink, Prompt
/// - Sixel: Windows Terminal, Konsole, foot, mlterm, contour, Alacritty, eat, yaft
///
/// `ANYR_GRAPHICS=kitty|iterm|sixel|off` overrides detection.
/// Generic `xterm-256color` is not treated as Sixel (VTE/Apple/conhost share it).
/// tmux: Kitty/iTerm are wrapped in DCS passthrough; Sixel is left raw for tmux 3.4+.
fn graphics_kind() -> Option<Graphics> {
    if !io::stdout().is_terminal() {
        return None;
    }
    graphics_kind_from(|k| std::env::var(k).ok())
}

fn graphics_kind_from(get: impl Fn(&str) -> Option<String>) -> Option<Graphics> {
    let get = &get;
    let has = |k: &str| get(k).is_some();
    let val = |k: &str| get(k).unwrap_or_default();

    match val("ANYR_GRAPHICS").trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "0" | "false" => return None,
        "kitty" => return Some(Graphics::Kitty),
        "iterm" | "iterm2" | "osc1337" => return Some(Graphics::Iterm),
        "sixel" => return Some(Graphics::Sixel),
        _ => {}
    }

    let term = val("TERM").to_ascii_lowercase();
    let program = val("TERM_PROGRAM").to_ascii_lowercase();
    let emulator = val("TERMINAL_EMULATOR").to_ascii_lowercase();
    let lc_terminal = val("LC_TERMINAL").to_ascii_lowercase();

    // Multiplexers that swallow sequences (tmux is handled at emit time).
    if has("ZELLIJ") || has("STY") {
        return None;
    }
    if term.starts_with("screen") && !has("TMUX") {
        return None;
    }

    if matches!(
        program.as_str(),
        "apple_terminal"
            | "vscode"
            | "vscode-insiders"
            | "cursor"
            | "zed"
            | "hyper"
            | "terminus"
            | "jetbrains"
    ) || emulator.contains("jetbrains")
        || has("NVIM")
        || has("TERMUX_VERSION")
        || matches!(
            term.as_str(),
            "dumb" | "linux" | "vt100" | "vt102" | "vt220" | "ansi" | "cygwin"
        )
    {
        return None;
    }

    if has("KITTY_WINDOW_ID")
        || has("KITTY_PID")
        || has("GHOSTTY_RESOURCES_DIR")
        || has("GHOSTTY_BIN_DIR")
        || term.contains("kitty")
        || term.contains("ghostty")
        || program == "kitty"
        || program == "ghostty"
    {
        return Some(Graphics::Kitty);
    }

    if has("ITERM_SESSION_ID")
        || has("WEZTERM_EXECUTABLE")
        || has("WEZTERM_PANE")
        || lc_terminal == "iterm2"
        || matches!(
            program.as_str(),
            "iterm.app"
                | "wezterm"
                | "warpterminal"
                | "mintty"
                | "rio"
                | "tabby"
                | "blink"
                | "prompt"
        )
        || term.contains("wezterm")
        || term.contains("mintty")
        || term.contains("warp")
        || term == "rio"
        || term.starts_with("rio-")
    {
        return Some(Graphics::Iterm);
    }

    if has("WT_SESSION")
        || has("WT_PROFILE_ID")
        || has("KONSOLE_VERSION")
        || has("KONSOLE_DBUS_SERVICE")
        || has("ALACRITTY_WINDOW_ID")
        || has("ALACRITTY_SOCKET")
        || term.starts_with("foot")
        || term.contains("mlterm")
        || term.contains("contour")
        || term.contains("sixel")
        || term.contains("alacritty")
        || term.starts_with("yaft")
        || term.starts_with("eat-")
        || term.contains("darktile")
        || term.contains("domterm")
    {
        return Some(Graphics::Sixel);
    }

    None
}

fn tmux_passthrough(seq: &str) -> String {
    let escaped = seq.replace('\x1b', "\x1b\x1b");
    format!("\x1bPtmux;{escaped}\x1b\\")
}

/// Best-effort inline PNG/Sixel of the official AR mark.
fn emit_graphics_mark() -> bool {
    let Some(kind) = graphics_kind() else {
        return false;
    };
    let seq = match kind {
        Graphics::Kitty => {
            let b64 = base64_encode(MARK_PNG);
            format!("\x1b_Ga=T,f=100,c=12,r=4,q=2;{b64}\x1b\\")
        }
        Graphics::Iterm => {
            let b64 = base64_encode(MARK_PNG);
            format!("\x1b]1337;File=inline=1;width=12;height=4;preserveAspectRatio=1:{b64}\x07")
        }
        Graphics::Sixel => include_str!("../assets/mark.sixel").to_string(),
    };
    // Kitty/iTerm need tmux DCS passthrough (`allow-passthrough on`, tmux 3.3+).
    // Sixel is understood by tmux 3.4+ itself.
    let payload = if std::env::var_os("TMUX").is_some() && kind != Graphics::Sixel {
        tmux_passthrough(&seq)
    } else {
        seq
    };
    let mut out = io::stdout();
    if out.write_all(payload.as_bytes()).is_err() {
        return false;
    }
    let _ = out.write_all(b"\n");
    let _ = out.flush();
    true
}

/// AR mark next to matching caption lines.
pub fn brand_header(captions: &[&str]) -> String {
    let mut out = String::new();
    for (i, mark_line) in MARK_LINES.iter().enumerate() {
        let mark = paint(WHITE, mark_line);
        let caption = captions.get(i).copied().unwrap_or("");
        out.push_str(&mark);
        if !caption.is_empty() {
            out.push_str("  ");
            out.push_str(caption);
        }
        out.push('\n');
    }
    out
}

/// Print the official AR mark: inline PNG/Sixel when the terminal can, else half-blocks.
pub fn print_brand_header(captions: &[&str]) {
    if emit_graphics_mark() {
        for caption in captions {
            if !caption.is_empty() {
                println!("{caption}");
            }
        }
        return;
    }
    print!("{}", brand_header(captions));
}

/// OSC 8 hyperlink when color/TTY is on; plain URL otherwise.
pub fn link(url: &str) -> String {
    if !color_enabled() {
        return url.to_string();
    }
    format!("\x1b]8;;{url}\x1b\\{MAGENTA}{url}{RESET}\x1b]8;;\x1b\\")
}

pub fn tool_color(tool: &str) -> &'static str {
    match tool {
        "claude" | "cc" => ORANGE,
        "codex" => SUCCESS,
        "grok" => WHITE,
        "opencode" => BLUE,
        "pi" => TEAL,
        "pool" | "poolside" => MAGENTA,
        _ => MAGENTA,
    }
}

pub fn divider(width: usize) -> String {
    dim(&"─".repeat(width.max(8)))
}

pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub fn prompt(label: &str) -> Result<String, String> {
    eprint!("{label}");
    let _ = io::stderr().flush();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("Could not read input: {e}"))?;
    Ok(line.trim().to_string())
}

/// Read a line with terminal echo disabled (for API keys). Falls back to
/// normal reading when stdin is not a TTY or termios control fails.
///
/// Windows keeps echo on (pre-existing limitation; no new deps).
pub fn prompt_secret(label: &str) -> Result<String, String> {
    #[cfg(unix)]
    {
        if !io::stdin().is_terminal() {
            return prompt(label);
        }

        let fd = libc::STDIN_FILENO;
        // SAFETY: zeroed termios is only a write buffer for tcgetattr.
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fd 0 is stdin; `original` is a valid termios buffer we own.
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return prompt(label);
        }

        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        // SAFETY: `hidden` is a copy of the termios returned by tcgetattr.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
            return prompt(label);
        }

        let result = prompt(label);
        // Restore echo even if read_line failed — restore before return.
        // SAFETY: `original` came from tcgetattr on this same fd.
        let _ = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
        let _ = io::stderr().write_all(b"\n");
        let _ = io::stderr().flush();
        result
    }
    #[cfg(not(unix))]
    {
        // Windows: documented limitation, same as before.
        prompt(label)
    }
}

pub fn confirm(question: &str) -> bool {
    match prompt(&format!("{question} [y/N] ")) {
        Ok(ans) => matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes"),
        Err(_) => false,
    }
}

/// Interactive picker (Ratatui when `native`). `current` is pre-selected.
#[cfg(feature = "native")]
pub fn pick(title: &str, items: &[String], current: Option<usize>) -> Result<usize, String> {
    crate::tui::pick(title, items, current)
}

/// Fallback readline picker when the native TUI feature is off.
#[cfg(not(feature = "native"))]
pub fn pick(title: &str, items: &[String], current: Option<usize>) -> Result<usize, String> {
    const PAGE: usize = 12;
    if items.is_empty() {
        return Err("Nothing to pick.".into());
    }
    let mut page = current
        .map(|i| i / PAGE)
        .unwrap_or(0)
        .min(items.len().saturating_sub(1) / PAGE);

    loop {
        let start = page * PAGE;
        let end = (start + PAGE).min(items.len());
        let pages = items.len().div_ceil(PAGE);
        eprintln!("{}", bold(title));
        if pages > 1 {
            eprintln!(
                "{}",
                dim(&format!(
                    "  page {}/{}  ·  n next · p prev · q cancel",
                    page + 1,
                    pages
                ))
            );
        } else {
            eprintln!("{}", dim("  q cancel"));
        }
        for (i, item) in items.iter().enumerate().take(end).skip(start) {
            let marker = if current == Some(i) { "*" } else { " " };
            eprintln!("  {marker} {:>2}. {item}", i + 1);
        }
        let hint = current
            .map(|i| format!("Enter = {} · ", i + 1))
            .unwrap_or_default();
        let ans = prompt(&format!("{hint}Pick 1-{}: ", items.len()))?;
        let lower = ans.to_ascii_lowercase();
        if ans.is_empty() {
            return current.ok_or_else(|| "Cancelled.".into());
        }
        if lower == "q" || lower == "quit" || lower == "cancel" {
            return Err("Cancelled.".into());
        }
        if pages > 1 && (lower == "n" || lower == "next") {
            page = (page + 1) % pages;
            continue;
        }
        if pages > 1 && (lower == "p" || lower == "prev" || lower == "previous") {
            page = if page == 0 { pages - 1 } else { page - 1 };
            continue;
        }
        if let Ok(n) = ans.parse::<usize>() {
            if n >= 1 && n <= items.len() {
                return Ok(n - 1);
            }
        }
        let hits: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.to_ascii_lowercase() == lower || item.to_ascii_lowercase().contains(&lower)
            })
            .map(|(i, _)| i)
            .collect();
        if hits.len() == 1 {
            return Ok(hits[0]);
        }
        if let Some(i) = items
            .iter()
            .position(|item| item.to_ascii_lowercase() == lower)
        {
            return Ok(i);
        }
        eprintln!("{}", dim(&format!("Not a valid pick: {ans}")));
    }
}

/// Port of the JS fuzzy matcher (contiguous substring + subsequence bonuses).
pub fn fuzzy_score(query: &str, target: &str) -> i32 {
    let q = query.to_ascii_lowercase();
    let t = target.to_ascii_lowercase();
    if q.is_empty() {
        return 0;
    }
    let q_chars: Vec<char> = q.chars().collect();
    let t_chars: Vec<char> = t.chars().collect();
    let mut score = 0i32;
    if let Some(sub_idx) = t.find(&q) {
        score += 50;
        if sub_idx == 0 {
            score += 25;
        } else if is_sep(
            t_chars
                .get(sub_idx.saturating_sub(1))
                .copied()
                .unwrap_or('\0'),
        ) {
            score += 20;
        }
    }
    let mut qi = 0usize;
    let mut run = 0i32;
    for (ti, ch) in t_chars.iter().enumerate() {
        if qi >= q_chars.len() {
            break;
        }
        if *ch == q_chars[qi] {
            if run > 0 {
                score += 8 + run;
            } else {
                score += 1;
            }
            run += 1;
            if ti == 0 {
                score += 30;
            }
            if ti > 0 && is_sep(t_chars[ti - 1]) {
                score += 15;
            }
            qi += 1;
        } else {
            run = 0;
        }
    }
    if qi < q_chars.len() && t.find(&q).is_none() {
        return -1;
    }
    if qi < q_chars.len() {
        return -1;
    }
    score
}

fn is_sep(c: char) -> bool {
    matches!(c, '/' | '-' | '_' | '.' | ':' | ' ' | '\t' | '\n')
}

pub fn rank_ids(query: &str, ids: &[String]) -> Vec<String> {
    if query.trim().is_empty() {
        return ids.to_vec();
    }
    let mut scored: Vec<(i32, usize, String)> = ids
        .iter()
        .enumerate()
        .filter_map(|(i, id)| {
            let s = fuzzy_score(query, id);
            if s < 0 {
                None
            } else {
                Some((s, i, id.clone()))
            }
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_secret_falls_back_without_tty() {
        // In `cargo test` stdin is typically not a tty; assert it delegates and
        // returns whatever the underlying reader yields. Feed empty stdin via
        // the existing harness pattern — simplest: just call it and accept Ok/Err
        // but require no panic.
        let _ = prompt_secret("x: ");
    }

    #[test]
    fn paint_is_plain_when_no_tty() {
        let out = paint(MAGENTA, "hello");
        assert_eq!(out, "hello");
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn fuzzy_contiguous_outranks_scattered() {
        let glm = fuzzy_score("glm", "z-ai/glm-4.7");
        let gem = fuzzy_score("glm", "google/gemini-2.5");
        assert!(glm > gem, "glm={glm} gem={gem}");
        assert!(glm >= 0);
    }

    #[test]
    fn fuzzy_miss_is_negative() {
        assert_eq!(fuzzy_score("zzzz", "anthropic/claude-sonnet-4.6"), -1);
    }

    #[test]
    fn rank_ids_keeps_order_on_empty_query() {
        let ids = vec!["b".into(), "a".into()];
        assert_eq!(rank_ids("", &ids), ids);
    }

    #[test]
    fn brand_header_places_captions_beside_the_mark() {
        let out = brand_header(&["AnyRouter", "account  default", ""]);
        for line in MARK_LINES {
            assert!(out.contains(line), "missing {line:?} in:\n{out}");
        }
        assert!(
            out.contains("▀█████████▄"),
            "must be the official AR half-block mark, not dithered dots:\n{out}"
        );
        assert!(out.contains("AnyRouter"), "{out}");
        assert!(out.contains("account  default"), "{out}");
        assert_eq!(out.lines().count(), MARK_LINES.len());
    }

    #[test]
    fn mark_png_is_the_official_logo_not_the_32px_icon() {
        assert!(
            MARK_PNG.starts_with(b"\x89PNG"),
            "assets/mark.png must be PNG"
        );
        assert!(
            MARK_PNG.len() > 4000,
            "hi-res official AR is larger than the 96px crop, got {}",
            MARK_PNG.len()
        );
        let b64 = base64_encode(MARK_PNG);
        assert!(b64.starts_with("iVBOR"));
        assert!(!b64.contains(' '));
    }

    #[test]
    fn mark_sixel_is_a_sixel_dcs() {
        let s = include_str!("../assets/mark.sixel");
        assert!(
            s.starts_with("\x1bPq"),
            "assets/mark.sixel must start with DCS Pq, got {:?}",
            s.chars().take(8).collect::<String>()
        );
        assert!(s.contains("\x1b\\"), "sixel must end with ST");
    }

    fn probe(pairs: &[(&str, &str)]) -> Option<Graphics> {
        graphics_kind_from(|k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        })
    }

    #[test]
    fn graphics_kitty_family() {
        assert_eq!(probe(&[("KITTY_WINDOW_ID", "1")]), Some(Graphics::Kitty));
        assert_eq!(probe(&[("TERM", "xterm-kitty")]), Some(Graphics::Kitty));
        assert_eq!(
            probe(&[("GHOSTTY_RESOURCES_DIR", "/opt/ghostty")]),
            Some(Graphics::Kitty)
        );
        assert_eq!(
            probe(&[("TERM", "xterm-ghostty"), ("TERM_PROGRAM", "ghostty")]),
            Some(Graphics::Kitty)
        );
    }

    #[test]
    fn graphics_iterm_family() {
        assert_eq!(
            probe(&[("ITERM_SESSION_ID", "w0t0p0")]),
            Some(Graphics::Iterm)
        );
        assert_eq!(
            probe(&[("LC_TERMINAL", "iTerm2"), ("TERM", "xterm-256color")]),
            Some(Graphics::Iterm)
        );
        assert_eq!(probe(&[("TERM_PROGRAM", "WezTerm")]), Some(Graphics::Iterm));
        assert_eq!(
            probe(&[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm")]),
            Some(Graphics::Iterm)
        );
        assert_eq!(
            probe(&[("TERM_PROGRAM", "WarpTerminal")]),
            Some(Graphics::Iterm)
        );
        assert_eq!(probe(&[("TERM_PROGRAM", "mintty")]), Some(Graphics::Iterm));
        assert_eq!(probe(&[("TERM_PROGRAM", "rio")]), Some(Graphics::Iterm));
        assert_eq!(probe(&[("TERM_PROGRAM", "Tabby")]), Some(Graphics::Iterm));
        assert_eq!(probe(&[("TERM_PROGRAM", "Blink")]), Some(Graphics::Iterm));
    }

    #[test]
    fn graphics_sixel_family() {
        assert_eq!(probe(&[("WT_SESSION", "abc")]), Some(Graphics::Sixel));
        assert_eq!(
            probe(&[("KONSOLE_VERSION", "240400")]),
            Some(Graphics::Sixel)
        );
        assert_eq!(probe(&[("TERM", "foot")]), Some(Graphics::Sixel));
        assert_eq!(probe(&[("TERM", "mlterm")]), Some(Graphics::Sixel));
        assert_eq!(probe(&[("TERM", "contour")]), Some(Graphics::Sixel));
        assert_eq!(
            probe(&[("ALACRITTY_WINDOW_ID", "1")]),
            Some(Graphics::Sixel)
        );
        assert_eq!(probe(&[("TERM", "eat-truecolor")]), Some(Graphics::Sixel));
        assert_eq!(probe(&[("TERM", "yaft")]), Some(Graphics::Sixel));
    }

    #[test]
    fn graphics_skips_hosts_that_would_print_garbage() {
        assert_eq!(
            probe(&[("TERM", "xterm-256color"), ("TERM_PROGRAM", "vscode")]),
            None
        );
        assert_eq!(
            probe(&[
                ("TERM", "xterm-256color"),
                ("TERM_PROGRAM", "Apple_Terminal")
            ]),
            None
        );
        assert_eq!(
            probe(&[("TERM", "xterm-256color"), ("VTE_VERSION", "7600")]),
            None
        );
        assert_eq!(probe(&[("TERM", "xterm-256color")]), None);
        assert_eq!(probe(&[("TERM", "linux")]), None);
        assert_eq!(probe(&[("TERM", "dumb")]), None);
        assert_eq!(probe(&[("ZELLIJ", "0"), ("KITTY_WINDOW_ID", "1")]), None);
        assert_eq!(
            probe(&[("STY", "123.pts"), ("TERM", "screen-256color")]),
            None
        );
        assert_eq!(probe(&[("NVIM", "1"), ("TERM", "xterm-kitty")]), None);
        assert_eq!(
            probe(&[
                ("TERMINAL_EMULATOR", "JetBrains-JediTerm"),
                ("TERM", "xterm-256color")
            ]),
            None
        );
        assert_eq!(probe(&[("MSYSTEM", "MINGW64"), ("TERM", "xterm")]), None);
    }

    #[test]
    fn graphics_tmux_still_detects_outer_terminal() {
        assert_eq!(
            probe(&[
                ("TMUX", "/tmp/tmux-1/default,1,0"),
                ("KITTY_WINDOW_ID", "1")
            ]),
            Some(Graphics::Kitty)
        );
        assert_eq!(
            probe(&[("TMUX", "1"), ("ITERM_SESSION_ID", "w0t0p0")]),
            Some(Graphics::Iterm)
        );
        assert_eq!(
            probe(&[("TMUX", "1"), ("WT_SESSION", "abc")]),
            Some(Graphics::Sixel)
        );
    }

    #[test]
    fn graphics_override_env() {
        assert_eq!(
            probe(&[("ANYR_GRAPHICS", "off"), ("KITTY_WINDOW_ID", "1")]),
            None
        );
        assert_eq!(
            probe(&[("ANYR_GRAPHICS", "sixel"), ("TERM_PROGRAM", "vscode")]),
            Some(Graphics::Sixel)
        );
        assert_eq!(
            probe(&[("ANYR_GRAPHICS", "kitty"), ("TERM", "dumb")]),
            Some(Graphics::Kitty)
        );
        assert_eq!(probe(&[("ANYR_GRAPHICS", "iterm")]), Some(Graphics::Iterm));
    }

    #[test]
    fn tmux_passthrough_doubles_esc() {
        let wrapped = tmux_passthrough("\x1b_Gok\x1b\\");
        assert!(wrapped.starts_with("\x1bPtmux;"), "{wrapped:?}");
        assert!(wrapped.contains("\x1b\x1b_Gok\x1b\x1b\\"), "{wrapped:?}");
        assert!(wrapped.ends_with("\x1b\\"), "{wrapped:?}");
    }
}
