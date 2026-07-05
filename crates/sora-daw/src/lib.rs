//! sora-daw — DAW 統合レイヤー(技術要件書 §11)。
//!
//! DAW ごとの制御手段は互換性がないため、[`adapter::DawAdapter`] トレイトに
//! 抽象操作(capabilities / read / transport / write_clip / write_automation /
//! render)を定義し、DAW ごとにアダプタを実装する。Agent と MCP ツールは
//! アダプタの抽象操作のみを見る。
//!
//! - [`generic`]: ファイル書き出し + インポート手順の提示(常設フォールバック)
//! - [`studio_one`]: Sora Bridge 拡張(EditTask)+ Sora Surface(仮想 MIDI
//!   トリガー)による Studio One 5 統合(§11.2.1)

// テストコードでは unwrap/expect を許可する
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod adapter;
pub mod error;
pub mod generic;
pub mod merge;
pub mod studio_one;
pub mod types;
