//! Keymap for the interactive TUI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Launcher,
    Picker,
    Settings,
    Palette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Enter,
    Up,
    Down,
    /// Reset the focused settings row to its default (`x`).
    Unset,
    Backspace,
    Esc,
    Char(char),
    Resize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub ctrl: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Up,
    Down,
    Tab,
    Backspace,
    Delete,
}

pub fn map_key(surface: Surface, key: KeyEvent) -> Action {
    if key.ctrl {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('d') => Action::Quit,
            KeyCode::Char('p') => Action::Up,
            KeyCode::Char('n') => Action::Down,
            _ => Action::Esc,
        };
    }
    match key.code {
        KeyCode::Enter => Action::Enter,
        KeyCode::Esc => Action::Esc,
        KeyCode::Up => Action::Up,
        KeyCode::Down => Action::Down,
        KeyCode::Tab => Action::Down,
        KeyCode::Backspace | KeyCode::Delete => Action::Backspace,
        KeyCode::Char(c) => match (surface, c) {
            (Surface::Launcher, 'q' | 'x' | 'Q' | 'X') => Action::Quit,
            (Surface::Settings, 'q') => Action::Quit,
            (Surface::Settings, 'x' | 'X') => Action::Unset,
            // The palette is type-first: every printable char — including
            // q / j / k — goes into the query.
            (Surface::Palette, _) => Action::Char(c),
            (Surface::Launcher | Surface::Settings, 'j') => Action::Down,
            (Surface::Launcher | Surface::Settings, 'k') => Action::Up,
            (_, c) => Action::Char(c),
        },
    }
}

pub fn hint_line(surface: Surface) -> &'static str {
    match surface {
        Surface::Launcher => "↑↓/jk move  ↵ select  q/esc quit",
        Surface::Settings => "↑↓/jk move  ↵ edit  x reset  q/esc close",
        Surface::Picker => "type to search  ↑↓ move  ↵ select  esc cancel",
        Surface::Palette => "type to filter  ↑↓ move  ↵ run  esc quit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_q_quits() {
        let a = map_key(
            Surface::Launcher,
            KeyEvent {
                code: KeyCode::Char('q'),
                ctrl: false,
            },
        );
        assert_eq!(a, Action::Quit);
    }

    #[test]
    fn picker_q_is_char() {
        let a = map_key(
            Surface::Picker,
            KeyEvent {
                code: KeyCode::Char('q'),
                ctrl: false,
            },
        );
        assert_eq!(a, Action::Char('q'));
    }

    #[test]
    fn ctrl_c_quits() {
        let a = map_key(
            Surface::Picker,
            KeyEvent {
                code: KeyCode::Char('c'),
                ctrl: true,
            },
        );
        assert_eq!(a, Action::Quit);
    }

    #[test]
    fn palette_q_types_into_query_not_quit() {
        let a = map_key(
            Surface::Palette,
            KeyEvent {
                code: KeyCode::Char('q'),
                ctrl: false,
            },
        );
        assert_eq!(a, Action::Char('q'));
    }

    #[test]
    fn settings_x_resets_and_q_quits() {
        let x = map_key(
            Surface::Settings,
            KeyEvent {
                code: KeyCode::Char('x'),
                ctrl: false,
            },
        );
        assert_eq!(x, Action::Unset);
        let q = map_key(
            Surface::Settings,
            KeyEvent {
                code: KeyCode::Char('q'),
                ctrl: false,
            },
        );
        assert_eq!(q, Action::Quit);
    }
}
