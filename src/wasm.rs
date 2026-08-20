//! wasm-bindgen entry for size benches and optional in-browser dry-run.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn version() -> String {
    crate::VERSION.to_string()
}

/// Run one CLI line in the browser demo. Returns stdout-like text.
#[wasm_bindgen]
pub fn run_line(line: &str) -> String {
    crate::demo::run_demo(line)
}
