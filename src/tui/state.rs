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
                    self.cursor = if self.cursor == 0 { n - 1 } else { self.cursor - 1 };
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
                    self.cursor = if self.cursor == 0 { n - 1 } else { self.cursor - 1 };
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
            Action::Char(_) => Outcome::Continue,
        }
    }

    pub fn hint(&self) -> &'static str {
        hint_line(Surface::Launcher)
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
            vec!["openai/gpt".into(), "anthropic/claude".into(), "google/gemma".into()],
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
}
