//! GrokNight palette as ratatui Styles — RGB matches `term.rs`.

use ratatui::style::{Color, Modifier, Style};

pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// `NO_COLOR` or `TERM=dumb`: styles keep bold and the selection, drop color.
fn plain() -> bool {
    static PLAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PLAIN.get_or_init(|| std::env::var_os("NO_COLOR").is_some() || crate::term::dumb_terminal())
}

fn fg(c: Color) -> Style {
    if plain() {
        Style::default()
    } else {
        Style::default().fg(c)
    }
}

pub fn brand() -> Style {
    fg(rgb(246, 130, 31)).add_modifier(Modifier::BOLD)
}

pub fn accent() -> Style {
    fg(rgb(230, 160, 60)).add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    fg(rgb(108, 108, 108))
}

pub fn success() -> Style {
    fg(rgb(158, 206, 106)).add_modifier(Modifier::BOLD)
}

pub fn model() -> Style {
    fg(rgb(58, 149, 171))
}

pub fn white() -> Style {
    fg(rgb(225, 225, 225))
}

pub fn selected() -> Style {
    let style = if plain() {
        Style::default()
    } else {
        fg(rgb(225, 225, 225)).bg(rgb(46, 46, 51))
    };
    style.add_modifier(Modifier::BOLD)
}

/// The selected row's background under a cell's own color.
pub fn on_selected(style: Style) -> Style {
    if plain() {
        style
    } else {
        style.bg(rgb(46, 46, 51))
    }
}

pub fn warn() -> Style {
    fg(rgb(230, 160, 60))
}

pub fn title() -> Style {
    fg(rgb(225, 225, 225)).add_modifier(Modifier::BOLD)
}
