//! GrokNight palette as ratatui Styles — RGB matches `term.rs`.

use ratatui::style::{Color, Modifier, Style};

pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// Dim backdrop behind the centered launcher dialog.
pub fn backdrop_rgb() -> Color {
    rgb(12, 12, 14)
}

/// Dialog card surface (slightly lifted from backdrop).
pub fn surface_rgb() -> Color {
    rgb(22, 22, 26)
}

pub fn brand() -> Style {
    Style::default()
        .fg(rgb(246, 130, 31))
        .add_modifier(Modifier::BOLD)
}

pub fn accent() -> Style {
    Style::default()
        .fg(rgb(187, 154, 247))
        .add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(rgb(108, 108, 108))
}

pub fn success() -> Style {
    Style::default()
        .fg(rgb(158, 206, 106))
        .add_modifier(Modifier::BOLD)
}

pub fn model() -> Style {
    Style::default().fg(rgb(58, 149, 171))
}

pub fn white() -> Style {
    Style::default().fg(rgb(225, 225, 225))
}

pub fn selected() -> Style {
    Style::default()
        .fg(rgb(225, 225, 225))
        .bg(rgb(54, 54, 54))
        .add_modifier(Modifier::BOLD)
}

pub fn title() -> Style {
    Style::default()
        .fg(rgb(225, 225, 225))
        .add_modifier(Modifier::BOLD)
}
