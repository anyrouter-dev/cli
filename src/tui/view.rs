//! Ratatui widgets + ANSI-free plain frames for dump / tests.
//!
//! The launcher renders as a two-pane layout on wide terminals (info panel
//! left, action list right) and stacks vertically when narrow. Pickers stay
//! single-pane. All plain_* dumps remain ANSI-free for CI.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::state::{MenuState, PickerState};
use super::theme;

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
    // Two panes side by side only when there is room; stack otherwise.
    let wide = area.width >= 60 && state.header.len() <= 6;
    let (header_area, body_area) = if wide {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(area);
        (cols[0], cols[1])
    } else {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2 + state.header.len() as u16),
                Constraint::Min(3),
            ])
            .split(area);
        (rows[0], rows[1])
    };

    if wide {
        render_info_panel(frame, header_area, &state.title, &state.header);
    } else {
        render_header(frame, header_area, &state.title, &state.header);
    }

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

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if wide { theme::brand() } else { theme::muted() })
            .title(Span::styled(
                format!(" {} ", state.title),
                theme::title(),
            )),
    );
    let mut list_state = ListState::default().with_selected(Some(state.cursor));
    frame.render_stateful_widget(list, body_area, &mut list_state);

    let footer_area = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, footer_area);
}

/// Bordered panel showing account / model / agent / credits lines.
fn render_info_panel(frame: &mut Frame, area: Rect, title: &str, header: &[String]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::muted())
        .title(Span::styled(format!(" {title} "), theme::brand()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled("▲", theme::brand())));
    lines.push(Line::from(""));
    for h in header {
        let Some((key, rest)) = h.split_once("  ") else {
            lines.push(Line::from(Span::styled(h.clone(), theme::white())));
            continue;
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{key:<9}"), theme::muted()),
            Span::styled(rest.to_string(), theme::white()),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
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
    } else if l.contains("config") {
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
    } else if l.contains("quit") {
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
    } else if l.contains("quit") {
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
    let width = cols.max(40);
    let mut lines = Vec::new();
    lines.push(truncate(&format!("▲ {}", state.title), width));
    for h in &state.header {
        lines.push(truncate(h, width));
    }
    lines.push(truncate(&"─".repeat(width.min(48)), width));
    for (i, label) in state.items.iter().enumerate() {
        let marker = if i == state.cursor { "◆" } else { " " };
        lines.push(truncate(&format!("{marker} {label}"), width));
    }
    lines.push(truncate(state.hint(), width));
    lines
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
            vec!["Launch claude".into(), "Quit".into()],
        );
        let frame = plain_menu_frame(&state, 80);
        assert!(!frame.contains('\u{1b}'));
        assert!(frame.contains("▲ AnyRouter"));
        assert!(frame.contains("◆ Launch claude"));
        assert!(frame.contains("Quit"));
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
}
