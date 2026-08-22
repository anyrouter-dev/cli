//! Pure picker / launcher state. Drive with `apply()` in tests — no terminal I/O.

use super::keys::{hint_line, Action, Surface};
use crate::term::rank_ids;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    Selected(usize),
    Cancelled,
    Quit,
}

#[derive(Debug, Clone)]
pub struct PickerState {
    pub title: String,
    pub header: Vec<String>,
    pub items: Vec<String>,
    pub query: String,
    /// Index into the filtered list.
    pub cursor: usize,
    /// Original index pre-selected when query is empty.
    pub initial: Option<usize>,
}

impl PickerState {
    pub fn new(title: impl Into<String>, items: Vec<String>, current: Option<usize>) -> Self {
        let cursor = current.unwrap_or(0).min(items.len().saturating_sub(1));
        Self {
            title: title.into(),
            header: Vec::new(),
            items,
            query: String::new(),
            cursor,
            initial: current,
        }
    }

    pub fn with_header(mut self, header: Vec<String>) -> Self {
        self.header = header;
        self
    }

    pub fn surface(&self) -> Surface {
        Surface::Picker
    }

    pub fn filtered(&self) -> Vec<(usize, &str)> {
        if self.query.trim().is_empty() {
            return self
                .items
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.as_str()))
                .collect();
        }
        let ranked = rank_ids(&self.query, &self.items);
        ranked
            .into_iter()
            .filter_map(|id| {
                self.items
                    .iter()
                    .position(|s| s == &id)
                    .map(|i| (i, self.items[i].as_str()))
            })
            .collect()
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        match action {
            Action::Quit => Outcome::Quit,
            Action::Esc => Outcome::Cancelled,
            Action::Resize => Outcome::Continue,
            Action::Enter => {
                let filtered = self.filtered();
                if filtered.is_empty() {
                    return Outcome::Continue;
                }
                let idx = self.cursor.min(filtered.len() - 1);
                Outcome::Selected(filtered[idx].0)
            }
            Action::Up => {
                let n = self.filtered().len();
                if n > 0 {
                    self.cursor = if self.cursor == 0 {
                        n - 1
                    } else {
                        self.cursor - 1
                    };
                }
                Outcome::Continue
            }
            Action::Down => {
                let n = self.filtered().len();
                if n > 0 {
                    self.cursor = (self.cursor + 1) % n;
                }
                Outcome::Continue
            }
            Action::Backspace => {
                self.query.pop();
                self.cursor = 0;
                Outcome::Continue
            }
            Action::Char(c) => {
                if c.is_control() {
                    return Outcome::Continue;
                }
                self.query.push(c);
                self.cursor = 0;
                Outcome::Continue
            }
            Action::Unset => Outcome::Continue,
        }
    }

    pub fn hint(&self) -> &'static str {
        hint_line(Surface::Picker)
    }
}

#[derive(Debug, Clone)]
pub struct MenuState {
    pub title: String,
    pub header: Vec<String>,
    pub items: Vec<String>,
    pub cursor: usize,
}

impl MenuState {
    pub fn new(title: impl Into<String>, header: Vec<String>, items: Vec<String>) -> Self {
        Self {
            title: title.into(),
            header,
            items,
            cursor: 0,
        }
    }

    pub fn surface(&self) -> Surface {
        Surface::Launcher
    }

    pub fn apply(&mut self, action: Action) -> Outcome {
        match action {
            Action::Quit | Action::Esc => Outcome::Quit,
            Action::Resize => Outcome::Continue,
            Action::Enter => {
                if self.items.is_empty() {
                    return Outcome::Quit;
                }
                Outcome::Selected(self.cursor.min(self.items.len() - 1))
            }
            Action::Up => {
                let n = self.items.len();
                if n > 0 {
                    self.cursor = if self.cursor == 0 {
                        n - 1
                    } else {
                        self.cursor - 1
                    };
                }
                Outcome::Continue
            }
            Action::Down => {
                let n = self.items.len();
                if n > 0 {
                    self.cursor = (self.cursor + 1) % n;
                }
                Outcome::Continue
            }
            Action::Backspace => Outcome::Continue,
            Action::Char(_) | Action::Unset => Outcome::Continue,
        }
    }

    pub fn hint(&self) -> &'static str {
        hint_line(Surface::Launcher)
    }
}

/// Color tone for a settings value — drives TUI color and dump annotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Plain white value.
    Normal,
    /// Green — enabled / healthy.
    Good,
    /// Teal — a model id.
    Model,
    /// Orange — needs attention.
    Warn,
    /// Dim — unset / default.
    Muted,
}

/// One row of the settings screen: a section header or an editable entry.
#[derive(Debug, Clone)]
pub enum SettingRow {
    Section(String),
    Entry {
        label: String,
        value: String,
        tone: Tone,
    },
}

impl SettingRow {
    pub fn selectable(&self) -> bool {
        matches!(self, SettingRow::Entry { .. })
    }
}

/// What the settings screen wants the caller to do after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsOutcome {
    Stay,
    /// Edit the entry at the given row index.
    Edit(usize),
    /// Reset the entry at the given row index to its default (`x`).
    Reset(usize),
    Close,
}

/// Cursor-style settings screen: grouped rows, right-aligned current values.
pub struct SettingsState {
    pub title: String,
    pub header: Vec<String>,
    pub rows: Vec<SettingRow>,
    /// Cursor index into `rows`; always points at an Entry.
    pub cursor: usize,
}

impl SettingsState {
    pub fn new(title: impl Into<String>, header: Vec<String>, rows: Vec<SettingRow>) -> Self {
        let mut state = Self {
            title: title.into(),
            header,
            rows,
            cursor: 0,
        };
        state.cursor = state.rows.iter().position(|r| r.selectable()).unwrap_or(0);
        state
    }

    /// Indices of selectable (Entry) rows.
    pub fn entries(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .map(|(i, _)| i)
            .collect()
    }

    pub fn apply(&mut self, action: Action) -> SettingsOutcome {
        let entries = self.entries();
        if entries.is_empty() {
            return match action {
                Action::Quit | Action::Esc => SettingsOutcome::Close,
                _ => SettingsOutcome::Stay,
            };
        }
        let pos = entries.iter().position(|i| *i == self.cursor).unwrap_or(0);
        match action {
            Action::Quit | Action::Esc => SettingsOutcome::Close,
            Action::Enter => SettingsOutcome::Edit(self.cursor),
            Action::Unset => SettingsOutcome::Reset(self.cursor),
            Action::Up => {
                let prev = if pos == 0 { entries.len() - 1 } else { pos - 1 };
                self.cursor = entries[prev];
                SettingsOutcome::Stay
            }
            Action::Down => {
                let next = (pos + 1) % entries.len();
                self.cursor = entries[next];
                SettingsOutcome::Stay
            }
            Action::Resize | Action::Backspace | Action::Char(_) => SettingsOutcome::Stay,
        }
    }

    pub fn hint(&self) -> &'static str {
        hint_line(Surface::Settings)
    }
}

/// Drive state with a scripted key sequence (unit / e2e, no TTY).
pub fn drive_picker(state: &mut PickerState, actions: &[Action]) -> Outcome {
    for action in actions {
        let out = state.apply(*action);
        if out != Outcome::Continue {
            return out;
        }
    }
    Outcome::Continue
}

pub fn drive_menu(state: &mut MenuState, actions: &[Action]) -> Outcome {
    for action in actions {
        let out = state.apply(*action);
        if out != Outcome::Continue {
            return out;
        }
    }
    Outcome::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_enter_selects_current() {
        let mut s = PickerState::new(
            "Pick",
            vec!["alpha".into(), "beta".into(), "gamma".into()],
            Some(1),
        );
        assert_eq!(s.apply(Action::Enter), Outcome::Selected(1));
    }

    #[test]
    fn picker_filter_narrows() {
        let mut s = PickerState::new(
            "Pick",
            vec![
                "openai/gpt".into(),
                "anthropic/claude".into(),
                "google/gemma".into(),
            ],
            Some(0),
        );
        s.apply(Action::Char('c'));
        s.apply(Action::Char('l'));
        assert_eq!(s.filtered().len(), 1);
        assert_eq!(s.apply(Action::Enter), Outcome::Selected(1));
    }

    #[test]
    fn picker_esc_cancels() {
        let mut s = PickerState::new("Pick", vec!["a".into()], Some(0));
        assert_eq!(s.apply(Action::Esc), Outcome::Cancelled);
    }

    #[test]
    fn menu_wraps_and_selects() {
        let mut s = MenuState::new(
            "Menu",
            vec!["header".into()],
            vec!["one".into(), "two".into(), "three".into()],
        );
        s.apply(Action::Down);
        s.apply(Action::Down);
        s.apply(Action::Down);
        assert_eq!(s.cursor, 0);
        s.apply(Action::Up);
        assert_eq!(s.cursor, 2);
        assert_eq!(s.apply(Action::Enter), Outcome::Selected(2));
    }

    #[test]
    fn drive_menu_script() {
        let mut s = MenuState::new("M", vec![], vec!["a".into(), "b".into()]);
        let out = drive_menu(&mut s, &[Action::Down, Action::Enter]);
        assert_eq!(out, Outcome::Selected(1));
    }

    fn sample_settings() -> SettingsState {
        SettingsState::new(
            "Config",
            vec![],
            vec![
                SettingRow::Section("Account".into()),
                SettingRow::Entry {
                    label: "account".into(),
                    value: "duyet".into(),
                    tone: Tone::Normal,
                },
                SettingRow::Entry {
                    label: "api key".into(),
                    value: "sk-ar-v1-ab…wxyz".into(),
                    tone: Tone::Muted,
                },
                SettingRow::Section("Model".into()),
                SettingRow::Entry {
                    label: "default".into(),
                    value: "auto".into(),
                    tone: Tone::Model,
                },
            ],
        )
    }

    #[test]
    fn settings_cursor_skips_sections() {
        let mut s = sample_settings();
        // Cursor starts on the first Entry (row 1), never the Section (row 0).
        assert_eq!(s.cursor, 1);
        s.apply(Action::Down);
        assert_eq!(s.cursor, 2);
        s.apply(Action::Down);
        assert_eq!(s.cursor, 4);
        s.apply(Action::Down); // wraps back to first entry
        assert_eq!(s.cursor, 1);
        s.apply(Action::Up); // wraps up to last entry
        assert_eq!(s.cursor, 4);
    }

    #[test]
    fn settings_edit_and_reset_report_row_index() {
        let mut s = sample_settings();
        s.cursor = 2;
        assert_eq!(s.apply(Action::Enter), SettingsOutcome::Edit(2));
        assert_eq!(s.apply(Action::Unset), SettingsOutcome::Reset(2));
        assert_eq!(s.apply(Action::Esc), SettingsOutcome::Close);
        assert_eq!(s.apply(Action::Quit), SettingsOutcome::Close);
        // Typing does nothing on a settings screen.
        assert_eq!(s.apply(Action::Char('a')), SettingsOutcome::Stay);
    }
}
