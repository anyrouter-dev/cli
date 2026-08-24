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

use super::state::{MenuState, PaletteEntry, PaletteState, PickerState, SettingRow, SettingsState, Tone};
use super::theme;

/// Preferred dialog width; shrinks on narrow terminals.
const DIALOG_PREF_WIDTH: u16 = 56;
/// Minimum usable dialog width before we fill almost the whole terminal.
const DIALOG_MIN_WIDTH: u16 = 28;
/// Settings screen is wider — model ids need room next to their labels.
const SETTINGS_PREF_WIDTH: u16 = 68;
/// Inner inset (cols / rows) so content isn't flush against the border.
const INSET_X: u16 = 1;
const INSET_Y: u16 = 1;

pub fn render_picker(frame: &mut Frame, state: &PickerState) {
    let area = frame.area();
    let area = inset(area, 1, 0);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1 + state.header.len() as u16),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
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
    frame.render_widget(search, chunks[2]);

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
            .title(Span::styled(format!(" {} ", state.title), theme::title())),
    );
    let mut list_state = ListState::default()
        .with_selected(Some(state.cursor.min(filtered.len().saturating_sub(1))));
    frame.render_stateful_widget(list, chunks[4], &mut list_state);

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[6]);
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
        .title(Span::styled(format!(" ▲ {} ", state.title), theme::brand()))
        .style(Style::default().bg(theme::surface_rgb()));
    let inner = inset(block.inner(dialog), INSET_X, INSET_Y);
    frame.render_widget(block, dialog);

    // status | pad | rule | pad | actions | pad | hint
    let status_h = state.header.len().max(1) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(status_h),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(state.items.len().max(1) as u16),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    render_status_lines(frame, chunks[0], &state.header);

    let rule = Paragraph::new(Span::styled(
        "─".repeat(chunks[2].width as usize),
        theme::muted(),
    ));
    frame.render_widget(rule, chunks[2]);

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
    frame.render_stateful_widget(list, chunks[4], &mut list_state);

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[6]);
}

fn dialog_height(state: &MenuState) -> u16 {
    // borders(2) + inset(2) + status + pads(3) + rule(1) + items + hint(1)
    let status = state.header.len().max(1) as u16;
    let items = state.items.len().max(1) as u16;
    2 + 2 + status + 3 + 1 + items + 1
}

/// Palette preferred width — detail columns need a little more room.
const PALETTE_PREF_WIDTH: u16 = 64;

/// Command palette: floating input on top, fuzzy results below, groups
/// rendered only where they change. Same centered-card language as the menu.
pub fn render_palette(frame: &mut Frame, state: &PaletteState) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::backdrop_rgb()).fg(Color::Reset)),
        area,
    );

    let filtered = state.filtered();
    let visible = filtered.len().min(12);
    // Group headers share the result area, so the dialog must grow by the
    // number of distinct groups among the visible rows.
    let groups = palette_groups(state, &filtered, visible);
    // borders(2) + inset(2) + input + pads + rule + rows + group headers
    // + blank between groups + hint
    let between = groups.saturating_sub(1);
    let height = 2 + 2 + 1 + 3 + 1 + visible.max(1) as u16 + groups as u16 + between as u16 + 1;
    let dialog = centered_dialog(area, height, PALETTE_PREF_WIDTH);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::brand())
        .title(Span::styled(" ▲ anyr ", theme::brand()))
        .style(Style::default().bg(theme::surface_rgb()));
    let inner = inset(block.inner(dialog), INSET_X, INSET_Y);
    frame.render_widget(block, dialog);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),               // input line
            Constraint::Length(1),               // pad
            Constraint::Length(1),               // rule
            Constraint::Length(1),               // pad
            Constraint::Min(visible.max(1) as u16), // results
            Constraint::Length(1),               // pad
            Constraint::Length(1),               // hint
        ])
        .split(inner);

    let input = Paragraph::new(Line::from(vec![
        Span::styled("❯ ", theme::accent()),
        Span::styled(state.query.clone(), theme::white()),
        Span::styled("█", theme::accent()),
    ]));
    frame.render_widget(input, chunks[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(chunks[2].width as usize),
            theme::muted(),
        )),
        chunks[2],
    );

    if filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no matches", theme::muted())),
            chunks[4],
        );
    } else {
        let rows = palette_rows(state, &filtered, visible, chunks[4].width as usize);
        frame.render_widget(Paragraph::new(rows), chunks[4]);
    }

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[6]);
}

/// Number of distinct groups among the first `visible` filtered entries —
/// each renders one header line inside the result area.
fn palette_groups(state: &PaletteState, filtered: &[usize], visible: usize) -> usize {
    let mut groups: Vec<&str> = Vec::new();
    for &entry_i in filtered.iter().take(visible) {
        let g = state.entries[entry_i].group.as_str();
        if g.is_empty() {
            continue;
        }
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    groups.len()
}

/// Build the result lines, inserting a group header whenever it changes and
/// right-aligning each row's detail column.
fn palette_rows(
    state: &PaletteState,
    filtered: &[usize],
    visible: usize,
    inner_w: usize,
) -> Vec<Line<'static>> {
    let cursor_row = state.cursor.min(visible.saturating_sub(1));
    let mut rows: Vec<Line> = Vec::new();
    let mut last_group: Option<&str> = None;
    for (row_i, &entry_i) in filtered.iter().take(visible).enumerate() {
        let entry: &PaletteEntry = &state.entries[entry_i];
        if last_group != Some(entry.group.as_str()) {
            if last_group.is_some() {
                rows.push(Line::from(""));
            }
            if !entry.group.is_empty() {
                rows.push(Line::from(Span::styled(
                    format!("  {}", entry.group.to_ascii_uppercase()),
                    theme::muted(),
                )));
            }
            last_group = Some(&entry.group);
        }
        let selected = row_i == cursor_row;
        let marker_style = if selected { theme::accent() } else { theme::muted() };
        let label = entry.label.clone();
        // Right-align the detail: pad between label and detail like the
        // settings screen pads its value column.
        let used = 2 + label.chars().count() + entry.detail.chars().count();
        let gap = inner_w.saturating_sub(used).max(1);
        let marker: String = if selected { "❯ ".into() } else { "  ".into() };
        rows.push(Line::from(vec![
            Span::styled(marker, marker_style),
            Span::styled(
                format!("{label}{}", " ".repeat(gap)),
                if selected { theme::selected() } else { theme::white() },
            ),
            Span::styled(entry.detail.clone(), theme::muted()),
        ]));
    }
    rows
}

/// Settings screen: same centered-dialog shape, grouped rows with
/// right-aligned colored values.
pub fn render_settings(frame: &mut Frame, state: &SettingsState) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme::backdrop_rgb()).fg(Color::Reset)),
        area,
    );

    let dialog = centered_dialog(area, settings_dialog_height(state), SETTINGS_PREF_WIDTH);
    frame.render_widget(Clear, dialog);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::brand())
        .title(Span::styled(format!(" ▲ {} ", state.title), theme::brand()))
        .style(Style::default().bg(theme::surface_rgb()));
    let inner = inset(block.inner(dialog), INSET_X, INSET_Y);
    frame.render_widget(block, dialog);

    let status_h = state.header.len().max(1) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(status_h),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(state.rows.len().max(1) as u16),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    render_status_lines(frame, chunks[0], &state.header);

    let rule = Paragraph::new(Span::styled(
        "─".repeat(chunks[2].width as usize),
        theme::muted(),
    ));
    frame.render_widget(rule, chunks[2]);

    let inner_w = chunks[4].width as usize;
    let lines: Vec<Line> = state
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| settings_row_line(row, i == state.cursor, inner_w))
        .collect();
    frame.render_widget(Paragraph::new(lines), chunks[4]);

    let footer = Paragraph::new(Span::styled(state.hint(), theme::muted()));
    frame.render_widget(footer, chunks[6]);
}

fn settings_dialog_height(state: &SettingsState) -> u16 {
    // borders(2) + inset(2) + status + pads(3) + rule(1) + rows + hint(1)
    let status = state.header.len().max(1) as u16;
    let rows = state.rows.len().max(1) as u16;
    2 + 2 + status + 3 + 1 + rows + 1
}

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Normal => theme::white(),
        Tone::Good => theme::success(),
        Tone::Model => theme::model(),
        Tone::Warn => Style::default().fg(theme::rgb(230, 160, 60)),
        Tone::Muted => theme::muted(),
    }
}

/// One settings row as styled spans: `❯ label      value` (value right-aligned).
fn settings_row_line(row: &SettingRow, selected: bool, inner_w: usize) -> Line<'static> {
    match row {
        SettingRow::Section(name) => Line::from(Span::styled(
            format!("  {}", name.to_ascii_uppercase()),
            theme::muted(),
        )),
        SettingRow::Gap => Line::from(""),
        SettingRow::Entry { label, value, tone } => {
            let marker = if selected { "❯ " } else { "  " };
            let marker_style = if selected {
                theme::accent()
            } else {
                theme::muted()
            };
            let label_style = if selected {
                theme::selected()
            } else {
                theme::white()
            };
            // Right-align the value: pad the label column to fill the gap.
            let used = 2 + label.chars().count() + value.chars().count();
            let gap = inner_w.saturating_sub(used).max(1);
            let padded_label = format!("{label}{}", " ".repeat(gap));
            Line::from(vec![
                Span::styled(marker.to_string(), marker_style),
                Span::styled(padded_label, label_style),
                Span::styled(value.clone(), tone_style(*tone)),
            ])
        }
    }
}

/// Shrink a rect by `x` columns and `y` rows on each side.
fn inset(area: Rect, x: u16, y: u16) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }
    let x = x.min(area.width.saturating_sub(1) / 2);
    let y = y.min(area.height.saturating_sub(1) / 2);
    Rect::new(
        area.x + x,
        area.y + y,
        area.width.saturating_sub(x.saturating_mul(2)).max(1),
        area.height.saturating_sub(y.saturating_mul(2)).max(1),
    )
}

/// Center a fixed-size dialog; clamp to terminal so narrow TTYs never clip badly.
fn centered_dialog(area: Rect, content_height: u16, pref_width: u16) -> Rect {
    if area.width == 0 || area.height == 0 {
        return area;
    }
    let max_w = area.width.saturating_sub(2).max(1);
    let width = pref_width.min(max_w).max(DIALOG_MIN_WIDTH.min(max_w));
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
    lines.push(format!("{pad_s}│{}│", pad_content("", content_w)));
    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));
    lines.push(format!("{pad_s}│{}│", pad_content("", content_w)));

    for (i, label) in state.items.iter().enumerate() {
        let marker = if i == state.cursor { "◆" } else { " " };
        let row = format!("{marker} {label}");
        lines.push(format!("{pad_s}│{}│", pad_content(&row, content_w)));
    }

    lines.push(format!("{pad_s}│{}│", pad_content("", content_w)));
    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));
    lines.push(format!("{pad_s}│{}│", pad_content(state.hint(), content_w)));
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

/// ANSI-free palette frame for `--dump-tui` and unit tests.
pub fn plain_palette_lines(state: &PaletteState, cols: usize) -> Vec<String> {
    let term_w = cols.max(DIALOG_MIN_WIDTH as usize);
    let inner = (PALETTE_PREF_WIDTH as usize)
        .min(term_w.saturating_sub(2))
        .max(DIALOG_MIN_WIDTH as usize)
        .min(term_w);
    let pad = term_w.saturating_sub(inner) / 2;
    let pad_s = " ".repeat(pad);
    let content_w = inner.saturating_sub(2);

    let mut lines = Vec::new();
    let title_raw = " ▲ anyr ".to_string();
    let dash_n = content_w.saturating_sub(title_raw.chars().count());
    lines.push(format!("{pad_s}╭{title_raw}{}╮", "─".repeat(dash_n)));

    // Input line with a block cursor.
    lines.push(format!(
        "{pad_s}│{}│",
        pad_content(&format!("❯ {}█", state.query), content_w)
    ));
    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));

    let filtered = state.filtered();
    if filtered.is_empty() {
        lines.push(format!("{pad_s}│{}│", pad_content("no matches", content_w)));
    } else {
        let visible = filtered.len().min(12);
        let cursor_row = state.cursor.min(visible.saturating_sub(1));
        let mut last_group: Option<&str> = None;
        for (row_i, &entry_i) in filtered.iter().take(visible).enumerate() {
            let entry = &state.entries[entry_i];
            if last_group != Some(entry.group.as_str()) {
                if last_group.is_some() {
                    lines.push(format!("{pad_s}│{}│", pad_content("", content_w)));
                }
                if !entry.group.is_empty() {
                    lines.push(format!(
                        "{pad_s}│{}│",
                        pad_content(
                            &format!("  {}", entry.group.to_ascii_uppercase()),
                            content_w
                        )
                    ));
                }
                last_group = Some(&entry.group);
            }
            let selected = row_i == cursor_row;
            let marker = if selected { "◆" } else { " " };
            let used = 3 + entry.label.chars().count() + entry.detail.chars().count();
            let gap = content_w
                .saturating_sub(used)
                .max(1);
            let row = format!(
                "{marker} {label}{}{detail}",
                " ".repeat(gap),
                label = entry.label,
                detail = entry.detail
            );
            lines.push(format!("{pad_s}│{}│", pad_content(&row, content_w)));
        }
    }

    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));
    lines.push(format!("{pad_s}│{}│", pad_content(state.hint(), content_w)));
    lines.push(format!("{pad_s}╰{}╯", "─".repeat(content_w)));
    lines
}

pub fn plain_palette_frame(state: &PaletteState, cols: usize) -> String {
    let mut lines = plain_palette_lines(state, cols);
    lines.push(String::new());
    lines.join("\n")
}

/// ANSI-free settings frame for `--dump-tui` and unit tests.
pub fn plain_settings_lines(state: &SettingsState, cols: usize) -> Vec<String> {
    let term_w = cols.max(DIALOG_MIN_WIDTH as usize);
    let inner = (SETTINGS_PREF_WIDTH as usize)
        .min(term_w.saturating_sub(2))
        .max(DIALOG_MIN_WIDTH as usize)
        .min(term_w);
    let pad = term_w.saturating_sub(inner) / 2;
    let pad_s = " ".repeat(pad);
    let content_w = inner.saturating_sub(2);

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
    lines.push(format!("{pad_s}│{}│", pad_content("", content_w)));
    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));
    lines.push(format!("{pad_s}│{}│", pad_content("", content_w)));

    for (i, row) in state.rows.iter().enumerate() {
        let line = match row {
            SettingRow::Section(name) => format!("  {}", name.to_ascii_uppercase()),
            SettingRow::Gap => String::new(),
            SettingRow::Entry { label, value, .. } => {
                let marker = if i == state.cursor { "◆" } else { " " };
                let used = 4 + label.chars().count() + value.chars().count();
                let gap = content_w.saturating_sub(used).max(1);
                format!("{marker} {label}{}{value}", " ".repeat(gap))
            }
        };
        lines.push(format!("{pad_s}│{}│", pad_content(&line, content_w)));
    }

    lines.push(format!("{pad_s}├{}┤", "─".repeat(content_w)));
    lines.push(format!("{pad_s}│{}│", pad_content(state.hint(), content_w)));
    lines.push(format!("{pad_s}╰{}╯", "─".repeat(content_w)));
    lines
}

pub fn plain_settings_frame(state: &SettingsState, cols: usize) -> String {
    let mut lines = plain_settings_lines(state, cols);
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
        assert!(
            frame.contains('╭') && frame.contains('╯'),
            "dialog box: {frame}"
        );
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
    fn inset_shrinks_and_clamps() {
        let r = Rect::new(0, 0, 20, 10);
        let i = inset(r, 1, 1);
        assert_eq!(i, Rect::new(1, 1, 18, 8));
        let tiny = Rect::new(0, 0, 2, 2);
        let i = inset(tiny, 4, 4);
        assert!(i.width >= 1 && i.height >= 1);
    }

    #[test]
    fn centered_dialog_clamps_to_area() {
        let tiny = Rect::new(0, 0, 20, 8);
        let d = centered_dialog(tiny, 20, 52);
        assert!(d.width <= tiny.width);
        assert!(d.height <= tiny.height);
        assert!(d.width >= 1);
    }

    #[test]
    fn dump_settings_is_ansi_free_and_grouped() {
        use super::super::state::{SettingRow, SettingsState, Tone};
        let state = SettingsState::new(
            "Config",
            vec!["account  duyet · me@example.co".into()],
            vec![
                SettingRow::Section("Account".into()),
                SettingRow::Entry {
                    label: "account".into(),
                    value: "duyet".into(),
                    tone: Tone::Normal,
                },
                SettingRow::Gap,
                SettingRow::Section("Model".into()),
                SettingRow::Entry {
                    label: "default".into(),
                    value: "auto".into(),
                    tone: Tone::Model,
                },
            ],
        );
        let frame = plain_settings_frame(&state, 80);
        assert!(!frame.contains('\u{1b}'), "must be ANSI-free: {frame}");
        assert!(frame.contains("▲ Config"), "{frame}");
        assert!(frame.contains("ACCOUNT"), "{frame}");
        assert!(frame.contains("MODEL"), "{frame}");
        assert!(frame.contains("◆ account"), "{frame}");
        assert!(frame.contains('╭') && frame.contains('╯'), "{frame}");
        // Blank line between ACCOUNT and MODEL sections.
        let dumped: Vec<&str> = frame.lines().collect();
        let acct = dumped.iter().position(|l| l.contains("ACCOUNT")).expect("ACCOUNT");
        let model = dumped.iter().position(|l| l.contains("MODEL")).expect("MODEL");
        assert!(model > acct + 1, "expected padding between sections:\n{frame}");
        // Values right-aligned inside the card (before the right border).
        let row_line = frame
            .lines()
            .find(|l| l.contains("default") && l.contains("auto"))
            .expect("default row");
        assert!(
            row_line
                .trim_end()
                .trim_end_matches('│')
                .trim_end()
                .ends_with("auto"),
            "{row_line}"
        );
        // Narrow terminals still render.
        let narrow = plain_settings_frame(&state, 30);
        assert!(!narrow.contains('\u{1b}'));
        assert!(narrow.contains("▲ Config"), "{narrow}");
    }
}
