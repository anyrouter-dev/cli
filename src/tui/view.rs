//! Ratatui widgets + ANSI-free plain frames for dump / tests.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
        Span::styled("search: ", theme::muted()),
        Span::styled(state.query.as_str(), theme::white()),
        Span::styled("█", theme::accent()),
    ]))
    .block(Block::default().borders(Borders::ALL).border_style(theme::muted()));
    frame.render_widget(search, chunks[1]);

    let filtered = state.filtered();
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let marker = if i == state.cursor { "◆ " } else { "  " };
            let style = if i == state.cursor {
                theme::selected()
            } else {
                theme::white()
            };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label}"), style)))
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1 + state.header.len() as u16),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, chunks[0], &state.title, &state.header);

    let items: Vec<ListItem> = state
        .items
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let marker = if i == state.cursor { "◆ " } else { "  " };
            let style = if i == state.cursor {
                theme::selected()
            } else {
                theme::white()
            };
            ListItem::new(Line::from(Span::styled(format!("{marker}{label}"), style)))
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
    let mut list_state = ListState::default().with_selected(Some(state.cursor));
    frame.render_stateful_widget(list, chunks[1], &mut list_state);

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[2]);
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
}
