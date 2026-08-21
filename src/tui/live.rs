//! Live terminal loop: ratatui + crossterm raw mode / alternate screen.

use std::io::{self, stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode as CtKeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::keys::{map_key, KeyCode, KeyEvent, Surface};
use super::state::{MenuState, Outcome, PickerState};
use super::view::{plain_menu_frame, plain_picker_frame, render_menu, render_picker};

pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn translate_key(ev: crossterm::event::KeyEvent) -> Option<KeyEvent> {
    if ev.kind != KeyEventKind::Press {
        return None;
    }
    let code = match ev.code {
        CtKeyCode::Enter => KeyCode::Enter,
        CtKeyCode::Esc => KeyCode::Esc,
        CtKeyCode::Up => KeyCode::Up,
        CtKeyCode::Down => KeyCode::Down,
        CtKeyCode::Tab => KeyCode::Tab,
        CtKeyCode::Backspace => KeyCode::Backspace,
        CtKeyCode::Delete => KeyCode::Delete,
        CtKeyCode::Char(c) => KeyCode::Char(c),
        _ => return None,
    };
    Some(KeyEvent {
        code,
        ctrl: ev.modifiers.contains(KeyModifiers::CONTROL),
    })
}

struct LiveTerminal {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl LiveTerminal {
    fn start() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| e.to_string())?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
        let backend = CrosstermBackend::new(stdout());
        let terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
        Ok(Self { terminal })
    }
}

impl Drop for LiveTerminal {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
        let _ = self.terminal.show_cursor();
    }
}

pub fn run_picker_live(mut state: PickerState) -> Result<Outcome, String> {
    let mut live = LiveTerminal::start()?;
    loop {
        live.terminal
            .draw(|f| render_picker(f, &state))
            .map_err(|e| e.to_string())?;
        if !event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())? {
            continue;
        }
        match event::read().map_err(|e| e.to_string())? {
            Event::Resize(_, _) => {
                state.apply(super::keys::Action::Resize);
            }
            Event::Key(ev) => {
                let Some(key) = translate_key(ev) else {
                    continue;
                };
                let outcome = state.apply(map_key(Surface::Picker, key));
                if outcome != Outcome::Continue {
                    return Ok(outcome);
                }
            }
            _ => {}
        }
    }
}

pub fn run_menu_live(mut state: MenuState) -> Result<Outcome, String> {
    let mut live = LiveTerminal::start()?;
    loop {
        live.terminal
            .draw(|f| render_menu(f, &state))
            .map_err(|e| e.to_string())?;
        if !event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())? {
            continue;
        }
        match event::read().map_err(|e| e.to_string())? {
            Event::Resize(_, _) => {
                state.apply(super::keys::Action::Resize);
            }
            Event::Key(ev) => {
                let Some(key) = translate_key(ev) else {
                    continue;
                };
                let outcome = state.apply(map_key(Surface::Launcher, key));
                if outcome != Outcome::Continue {
                    return Ok(outcome);
                }
            }
            _ => {}
        }
    }
}

pub fn dump_picker(state: &PickerState, cols: usize) -> String {
    plain_picker_frame(state, cols)
}

pub fn dump_menu(state: &MenuState, cols: usize) -> String {
    plain_menu_frame(state, cols)
}
