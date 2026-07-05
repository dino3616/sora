//! sora-core — Sora の Data 層モデル・検証・MIDI コンパイラ。
//!
//! アーキテクチャ(docs/technical-requirements.md):
//! - Agent 層(Claude Code)が Part Plan を起草する
//! - 本クレートが検証(L1/L2/L3)と決定論的な `.mid` 生成を担う
//! - エラーは Agent が自己修正に使う「成果物」として構造化される

// テストコードでは unwrap/expect を許可する(本体コードでは warn)
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod error;
pub mod fsutil;
pub mod midi;
pub mod model;
pub mod profile;
pub mod schema;
pub mod select;
pub mod types;
pub mod validate;
