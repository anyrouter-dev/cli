//! wasm-bindgen entry for the live website playground.

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
