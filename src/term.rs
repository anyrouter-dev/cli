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

/// Small AR mark rasterized from the official 32px icon (half-blocks).
/// Three rows × 11 cells — readable without a graphics protocol.
pub const MARK_LINES: [&str; 3] = ["    ▄▄ ▄▄▄ ", "  ▄█▀▀█▄▄█▀", " ▀▀    ▀▀▀▀"];

/// Official 32×32 white PNG (Kitty / iTerm2 inline image).
const MARK_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAACXBIWXMAABYlAAAWJQFJUiTwAAABUElEQVRYw2NgGAWjYBSMgqEI/v//rwHEFkC8EIh3Y8GrgTgCiBlp5YDtQHwJiMP/4webgZiV2paHIllQAsQZQFyOhlcA8V+omk5qWs4FxA+RHPAFiOVxqK2GqvkAxOzUckAL1NCfQPwOFsw41MoiOVSDGparAPEPWLACcSKSBcFY1NsgyUtRwwFboIY9BmIeUAoH4r1QsWdAzA9VxwHE5kB8HSp3mhqW+yL5JhRJXA2Iv0PF+4F4Nlou+AZyDKWWg3x0B2rgLizy9VC5P0DsCMQvkdKJOTV8X4NkoDoWeXYgvgFVcwaIY5FCIIZSy+WB+CvUsFY86pyA+B9UXQEQ74SyQaEhRIkD1kINAuV9bgJqFyGVDdZQGgRmk2u5G1JQ+hGhXhSI30DVbwDiCigbFDK25DhgHrRimUSCngikCskAWiSD2BNGq/BRMApGwaAFAIddWov0Aaf6AAAAAElFTkSuQmCC";

enum Graphics {
    Kitty,
    Iterm,
}

fn graphics_kind() -> Option<Graphics> {
    if !io::stdout().is_terminal() {
        return None;
    }
    if std::env::var_os("TMUX").is_some() {
        return None;
    }
    let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    if std::env::var_os("KITTY_WINDOW_ID").is_some()
        || term.contains("kitty")
        || term.contains("ghostty")
        || program.eq_ignore_ascii_case("ghostty")
    {
        return Some(Graphics::Kitty);
    }
    match program.as_str() {
        "iTerm.app" | "WezTerm" | "WarpTerminal" => Some(Graphics::Iterm),
        _ => None,
    }
}

/// Best-effort inline PNG. Returns true when a graphics sequence was written.
fn emit_graphics_mark() -> bool {
    let Some(kind) = graphics_kind() else {
        return false;
    };
    let seq = match kind {
        Graphics::Kitty => format!("\x1b_Ga=T,f=100,c=4,r=2,q=2;{MARK_PNG_B64}\x1b\\"),
        Graphics::Iterm => format!(
            "\x1b]1337;File=inline=1;width=4;height=2;preserveAspectRatio=1:{MARK_PNG_B64}\x07"
        ),
    };
    let mut out = io::stdout();
    if out.write_all(seq.as_bytes()).is_err() {
        return false;
    }
    let _ = out.write_all(b"\n");
    let _ = out.flush();
    true
}

/// Three-line AR mark next to up to three caption lines.
pub fn brand_header(captions: &[&str]) -> String {
    let mut out = String::new();
    for i in 0..3 {
        let mark = paint(WHITE, MARK_LINES[i]);
        let caption = captions.get(i).copied().unwrap_or("");
        if caption.is_empty() {
            out.push_str(&mark);
        } else {
            out.push_str(&mark);
            out.push_str("  ");
            out.push_str(caption);
        }
        out.push('\n');
    }
    out
}

/// Print the mark: Kitty/iTerm PNG when the terminal can, else half-block art.
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

pub fn confirm(question: &str) -> bool {
    match prompt(&format!("{question} [y/N] ")) {
        Ok(ans) => matches!(ans.to_ascii_lowercase().as_str(), "y" | "yes"),
        Err(_) => false,
    }
}

/// Numbered picker. `current` is pre-selected (1-based display still). Empty
/// input keeps `current` when set, otherwise errors.
pub fn pick(title: &str, items: &[String], current: Option<usize>) -> Result<usize, String> {
    if items.is_empty() {
        return Err("Nothing to pick.".into());
    }
    eprintln!("{}", bold(title));
    for (i, item) in items.iter().enumerate() {
        let marker = if current == Some(i) { "*" } else { " " };
        eprintln!("  {marker} {:>2}. {item}", i + 1);
    }
    let hint = current
        .map(|i| format!("Enter = {} · ", i + 1))
        .unwrap_or_default();
    let ans = prompt(&format!("{hint}Pick 1-{}: ", items.len()))?;
    if ans.is_empty() {
        return current.ok_or_else(|| "No selection.".into());
    }
    if let Ok(n) = ans.parse::<usize>() {
        if n >= 1 && n <= items.len() {
            return Ok(n - 1);
        }
    }
    let lower = ans.to_ascii_lowercase();
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
    Err(format!("Not a valid pick: {ans}"))
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
        assert!(out.contains("AnyRouter"), "{out}");
        assert!(out.contains("account  default"), "{out}");
        assert_eq!(out.lines().count(), 3);
    }
}
