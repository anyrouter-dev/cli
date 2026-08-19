//! AnyRouter native CLI library. Shared by the `anyr` binary and tests.

pub mod channel;
pub mod commands;
pub mod config;
pub mod help;
pub mod http;
pub mod key;
pub mod parse;
pub mod spawn;
pub mod upgrade;

pub use commands::run;
pub use config::{parse_config, serialize_config};
pub use parse::parse_cli_args;
pub use spawn::{build_tool_env, redact_value, render_dry_run};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
