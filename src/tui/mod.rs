//! Native interactive TUI — Ratatui + Crossterm.
//!
//! Public surface used by `commands.rs` / `onboard.rs`:
//!   * `pick` / `pick_with_header` — fuzzy list picker
//!   * `run_menu_select` — centered dialog launcher / config list
//!   * `wants_dump` / `dump_*` — ANSI-free frames for CI (`--dump-tui`)

pub mod keys;
pub mod live;
pub mod state;
pub mod theme;
pub mod view;

use std::collections::BTreeMap;

use crate::parse::ParsedArgs;

pub use keys::Action;
pub use live::{dump_menu, dump_picker, is_interactive, run_menu_live, run_picker_live};
pub use state::{drive_menu, drive_picker, MenuState, Outcome, PickerState};

pub fn wants_dump(parsed: &ParsedArgs, env: &BTreeMap<String, String>) -> bool {
    parsed.flag_true("dump-tui")
        || env
            .get("ANYR_TUI_DUMP")
            .map(|s| {
                let t = s.trim();
                t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
            })
            .unwrap_or(false)
}

pub fn dump_cols(env: &BTreeMap<String, String>) -> usize {
    env.get("COLUMNS")
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n >= 20)
        .unwrap_or(80)
}

/// Interactive fuzzy picker. Returns the index into `items`, or `Err("Cancelled.")`.
pub fn pick(title: &str, items: &[String], current: Option<usize>) -> Result<usize, String> {
    pick_with_header(title, &[], items, current)
}

pub fn pick_with_header(
    title: &str,
    header: &[String],
    items: &[String],
    current: Option<usize>,
) -> Result<usize, String> {
    if items.is_empty() {
        return Err("Nothing to pick.".into());
    }
    let state = PickerState::new(title, items.to_vec(), current).with_header(header.to_vec());
    if !is_interactive() {
        return Err("Cancelled.".into());
    }
    match run_picker_live(state)? {
        Outcome::Selected(i) => Ok(i),
        Outcome::Cancelled | Outcome::Quit => Err("Cancelled.".into()),
        Outcome::Continue => Err("Cancelled.".into()),
    }
}

/// Launcher / config list. Returns selected index, or `None` on quit.
pub fn run_menu_select(
    title: &str,
    header: Vec<String>,
    items: Vec<String>,
) -> Result<Option<usize>, String> {
    if items.is_empty() {
        return Ok(None);
    }
    let state = MenuState::new(title, header, items);
    if !is_interactive() {
        return Ok(None);
    }
    match run_menu_live(state)? {
        Outcome::Selected(i) => Ok(Some(i)),
        Outcome::Quit | Outcome::Cancelled => Ok(None),
        Outcome::Continue => Ok(None),
    }
}

/// Dump one launcher frame (for `--dump-tui` / `ANYR_TUI_DUMP`).
pub fn dump_menu_select(title: &str, header: Vec<String>, items: Vec<String>, cols: usize) -> String {
    dump_menu(&MenuState::new(title, header, items), cols)
}

pub fn dump_pick(title: &str, items: &[String], current: Option<usize>, cols: usize) -> String {
    dump_picker(&PickerState::new(title, items.to_vec(), current), cols)
}
