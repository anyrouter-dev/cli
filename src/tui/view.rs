//! Ratatui widgets + ANSI-free plain frames for dump / tests.
//!
//! The launcher / config menu renders as a **centered dialog card** (title,
//! status, actions) — not a full-bleed two-pane sprawl. Pickers stay
//! single-pane and fill the terminal. All plain_* dumps remain ANSI-free for CI.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::state::{MenuState, PickerState};
use super::theme;

/// Preferred dialog width; shrinks on narrow terminals.
const DIALOG_PREF_WIDTH: u16 = 52;
/// Minimum usable dialog width before we fill almost the whole terminal.
const DIALOG_MIN_WIDTH: u16 = 28;

pub fn render_picker(frame: &mut Frame, state: &PickerState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1 + state.header.len() as u16),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, chunks[0], &state.title, &state.header);

    let search = Paragraph::new(Line::from(vec![
        Span::styled("⌕ ", theme::accent()),
        Span::styled(state.query.as_str(), theme::white()),
        Span::styled("█", theme::accent()),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::muted())
            .title(Span::styled(" search ", theme::muted())),
    );
    frame.render_widget(search, chunks[1]);

    let filtered = state.filtered();
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let style = if i == state.cursor {
                theme::selected()
            } else {
                theme::white()
            };
            ListItem::new(Line::from(vec![
                Span::styled(if i == state.cursor { "❯ " } else { "  " }, theme::accent()),
                Span::styled(item_icon(label), item_icon_style(label)),
                Span::styled(format!("{label}"), style),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::muted())
            .title(Span::styled(
                format!(" {} ", state.title),
                theme::title(),
            )),
    );
    let mut list_state = ListState::default().with_selected(Some(state.cursor.min(
        filtered.len().saturating_sub(1),
    )));
    frame.render_stateful_widget(list, chunks[2], &mut list_state);

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[3]);
}

pub fn render_menu(frame: &mut Frame, state: &MenuState) {
    let area = frame.area();

    // Dim full-screen backdrop so the card reads as a modal.
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::backdrop_rgb()).fg(Color::Reset)),
        area,
    );

    let dialog = centered_dialog(area, dialog_height(state), DIALOG_PREF_WIDTH);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::brand())
        .title(Span::styled(
            format!(" ▲ {} ", state.title),
            theme::brand(),
        ))
        .style(Style::default().bg(theme::surface_rgb()));
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);

    // status | actions | hint
    let status_h = state.header.len().max(1) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(status_h),
            Constraint::Length(1),
            Constraint::Min(state.items.len().max(1) as u16),
            Constraint::Length(1),
        ])
        .split(inner);

    render_status_lines(frame, chunks[0], &state.header);

    let rule = Paragraph::new(Span::styled(
        "─".repeat(chunks[1].width as usize),
        theme::muted(),
    ));
    frame.render_widget(rule, chunks[1]);

    let items: Vec<ListItem> = state
        .items
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == state.cursor {
                theme::selected()
            } else {
                theme::white()
            };
            ListItem::new(Line::from(vec![
                Span::styled(if i == state.cursor { "❯ " } else { "  " }, theme::accent()),
                Span::styled(item_icon(label), item_icon_style(label)),
                Span::styled(label.as_str(), style),
            ]))
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ListState::default().with_selected(Some(state.cursor));
    frame.render_stateful_widget(list, chunks[2], &mut list_state);

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[3]);
}

fn dialog_height(state: &MenuState) -> u16 {
    // borders(2) + status + rule(1) + items + hint(1)
    let status = state.header.len().max(1) as u16;
    let items = state.items.len().max(1) as u16;
    2 + status + 1 + items + 1
}

/// Center a fixed-size dialog; clamp to terminal so narrow TTYs never clip badly.
fn centered_dialog(area: Rect, content_height: u16, pref_width: u16) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }
    let max_w = area.width.saturating_sub(2).max(1);
    let width = pref_width
        .min(max_w)
        .max(DIALOG_MIN_WIDTH.min(max_w));
    let max_h = area.height.saturating_sub(0).max(1);
    let height = content_height.min(max_h).max(5.min(max_h));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn render_status_lines(frame: &mut Frame, area: Rect, header: &[String]) {
    let mut lines: Vec<Line> = Vec::new();
    if header.is_empty() {
        lines.push(Line::from(Span::styled("—", theme::muted())));
    } else {
        for h in header {
            if let Some((key, rest)) = h.split_once("  ") {
                lines.push(Line::from(vec![
                    Span::styled(format!("{key:<9}"), theme::muted()),
                    Span::styled(rest.to_string(), theme::white()),
                ]));
            } else {
                lines.push(Line::from(Span::styled(h.clone(), theme::white())));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_header(frame: &mut Frame, area: Rect, title: &str, header: &[String]) {
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("▲ {title}"),
        theme::brand(),
    ))];
    for h in header {
        lines.push(Line::from(Span::styled(h.clone(), theme::muted())));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Icon for a launcher/picker row, derived from its label.
pub fn item_icon(label: &str) -> &'static str {
    let l = label.to_ascii_lowercase();
    if label.starts_with("Launch") || l.contains("claude") || l.contains("codex") {
        "⚡ "
    } else if l.contains("config") || l.contains("settings") {
        "⚙  "
    } else if l.contains("switch") || l.contains("account") {
        "⇄  "
    } else if l.contains("credit") {
        "¤  "
    } else if l.contains("login") || l.contains("sign in") {
        "🔑 "
    } else if l.contains("logout") || l.contains("log out") {
        "🚪 "
    } else if l.contains("onboard") {
        "📋 "
    } else if l.contains("quit") || l.contains("done") {
        "✕  "
    } else if l.contains("model") {
        "◆  "
    } else {
        "·  "
    }
}

fn item_icon_style(label: &str) -> Style {
    let l = label.to_ascii_lowercase();
    if label.starts_with("Launch") {
        theme::success()
    } else if l.contains("quit") || l.contains("done") {
        theme::muted()
    } else if l.contains("credit") {
        theme::model()
    } else {
        theme::accent()
    }
}

/// ANSI-free plain frame for `--dump-tui` and unit tests.
pub fn plain_picker_lines(state: &PickerState, cols: usize) -> Vec<String> {
    let width = cols.max(40);
    let mut lines = Vec::new();
    lines.push(truncate(&format!("▲ {}", state.title), width));
    for h in &state.header {
        lines.push(truncate(h, width));
    }
    lines.push(truncate(&format!("search: {}", state.query), width));
    lines.push(truncate(&"─".repeat(width.min(48)), width));
    let filtered = state.filtered();
    if filtered.is_empty() {
        lines.push(truncate("  (no matches)", width));
    } else {
        for (i, (_, label)) in filtered.iter().enumerate() {
            let marker = if i == state.cursor { "◆" } else { " " };
            lines.push(truncate(&format!("{marker} {label}"), width));
        }
    }
    lines.push(truncate(state.hint(), width));
    lines
}

pub fn plain_menu_lines(state: &MenuState, cols: usize) -> Vec<String> {
    // Dialog-shaped dump: fixed inner width, centered with padding when wide.
    let term_w = cols.max(DIALOG_MIN_WIDTH as usize);
    let inner = (DIALOG_PREF_WIDTH as usize)
        .min(term_w.saturating_sub(2))
        .max(DIALOG_MIN_WIDTH as usize)
        .min(term_w);
    let pad = term_w.saturating_sub(inner) / 2;
    let pad_s = " ".repeat(pad);
    let content_w = inner.saturating_sub(2); // inside │…│

    let mut lines = Vec::new();
    let title_raw = format!(" ▲ {} ", state.title);
    let title = truncate(&title_raw, content_w);
    let dash_n = content_w.saturating_sub(title.chars().count());
    lines.push(format!("{pad_s}╭{title}{}╮", "─".repeat(dash_n)));

    if state.header.is_empty() {
        lines.push(format!("{pad_s}│{}│", pad_content("—", content_w)));
    } else {
        for h in &state.header {
            lines.push(format!("{pad_s}│{}│", pad_content(h, content_w)));
        }
    }
    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));

    for (i, label) in state.items.iter().enumerate() {
        let marker = if i == state.cursor { "◆" } else { " " };
        let row = format!("{marker} {label}");
        lines.push(format!("{pad_s}│{}│", pad_content(&row, content_w)));
    }

    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));
    lines.push(format!(
        "{pad_s}│{}│",
        pad_content(state.hint(), content_w)
    ));
    lines.push(format!("{pad_s}╰{}╯", "─".repeat(content_w)));
    lines
}

fn pad_content(s: &str, width: usize) -> String {
    let truncated = truncate(s, width);
    let chars: Vec<char> = truncated.chars().collect();
    if chars.len() >= width {
        return truncated;
    }
    format!("{truncated}{}", " ".repeat(width - chars.len()))
}

pub fn plain_picker_frame(state: &PickerState, cols: usize) -> String {
    let mut lines = plain_picker_lines(state, cols);
    lines.push(String::new());
    lines.join("\n")
}

pub fn plain_menu_frame(state: &MenuState, cols: usize) -> String {
    let mut lines = plain_menu_lines(state, cols);
    lines.push(String::new());
    lines.join("\n")
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_menu_is_ansi_free() {
        let state = MenuState::new(
            "AnyRouter",
            vec!["account  default".into()],
            vec!["Launch claude".into(), "Config".into(), "Quit".into()],
        );
        let frame = plain_menu_frame(&state, 80);
        assert!(!frame.contains('\u{1b}'));
        assert!(frame.contains("▲ AnyRouter"), "{frame}");
        assert!(frame.contains("◆ Launch claude"), "{frame}");
        assert!(frame.contains("Config"), "{frame}");
        assert!(frame.contains("Quit"), "{frame}");
        assert!(frame.contains('╭') && frame.contains('╯'), "dialog box: {frame}");
    }

    #[test]
    fn dump_menu_narrow_terminal() {
        let state = MenuState::new(
            "AnyRouter",
            vec!["model  auto".into()],
            vec!["Login / sign in".into(), "Quit".into()],
        );
        let frame = plain_menu_frame(&state, 32);
        assert!(!frame.contains('\u{1b}'));
        assert!(frame.contains("▲ AnyRouter"), "{frame}");
        assert!(frame.contains("Quit"), "{frame}");
    }

    #[test]
    fn dump_picker_shows_search() {
        let mut state = PickerState::new("Model", vec!["a".into(), "b".into()], Some(0));
        state.query = "a".into();
        let frame = plain_picker_frame(&state, 80);
        assert!(frame.contains("search: a"));
        assert!(!frame.contains('\u{1b}'));
    }

    #[test]
    fn icons_cover_launcher_actions() {
        for (label, icon) in [
            ("Launch claude", "⚡"),
            ("Config", "⚙"),
            ("Settings", "⚙"),
            ("Switch model", "⇄"),
            ("Credits", "¤"),
            ("Login / sign in", "🔑"),
            ("Log out", "🚪"),
            ("Agent onboard prompt…", "📋"),
            ("Quit", "✕"),
        ] {
            assert_eq!(item_icon(label).trim(), icon, "icon for {label}");
        }
    }

    #[test]
    fn centered_dialog_clamps_to_area() {
        let tiny = Rect::new(0, 0, 20, 8);
        let d = centered_dialog(tiny, 20, 52);
        assert!(d.width <= tiny.width);
        assert!(d.height <= tiny.height);
        assert!(d.width >= 1);
    }
}
