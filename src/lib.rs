//! AnyRouter native CLI library. Shared by the `anyr` binary and tests.

pub mod auth;
pub mod buildinfo;
pub mod channel;
pub mod commands;
pub mod config;
pub mod demo;
pub mod help;
pub mod http;
pub mod install;
pub mod key;
pub mod onboard;
pub mod parse;
#[cfg(feature = "native")]
pub mod relay;
pub mod spawn;
pub mod spinner;
pub mod term;
#[cfg(feature = "native")]
pub mod tui;
pub mod upgrade;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use commands::run;
pub use config::{parse_config, serialize_config};
pub use parse::parse_cli_args;
pub use spawn::{build_tool_env, redact_value, render_dry_run};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
