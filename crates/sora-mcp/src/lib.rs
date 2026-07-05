//! sora-mcp — Sora の MCP サーバー(技術要件書 §8)。
//!
//! `sora mcp serve` から起動され、stdio で MCP ツールを公開する。
//! 各ツールは要求 control level(§2.4)を持ち、上限超過の呼び出しは
//! 実行前に拒否して「必要な level と有効化方法」を返す。
//!
//! CLI と MCP でエラー表現(`ErrorReport`)を同一に保つため(§6.4)、
//! anyhow → ErrorReport の正規化は本クレートの [`report`] に一元化し、
//! sora-cli も同じ関数を使う。運用ヘルパ([`ops`])も同様に共有する。

pub mod gate;
pub mod ops;
pub mod report;
pub mod server;
