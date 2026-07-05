//! サブコマンド実装。
//!
//! ファイル書き込み(非破壊)・操作ログ等の運用ヘルパは sora-mcp の
//! `ops` と共有する(CLI と MCP の挙動差を防ぐ。技術要件書 §6.4)。

pub mod audio;
pub mod automation;
pub mod config;
pub mod daw;
pub mod mcp;
pub mod midi;
pub mod profile;
pub mod project;
pub mod schema;
pub mod send;

pub(crate) use sora_mcp::ops::{record_action, write_new_file};
