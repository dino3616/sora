//! sora — Sora Tool 層の単一バイナリ(技術要件書 §5)。
//!
//! 全サブコマンドは成功時に JSON を stdout へ返し、失敗時は構造化 ErrorReport を
//! 出して規約どおりの終了コードで終わる(§6.3)。Agent が解析・自己修正できる形式。

mod commands;
mod output;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use output::{CmdResult, emit_error, emit_success};

/// Sora — 創作意図を接続された制作アクションへ変換するツール層。
#[derive(Parser)]
#[command(name = "sora", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// MIDI の生成・解析(compile / inspect / analyze / decompile)
    #[command(subcommand)]
    Midi(commands::midi::MidiCommand),
    /// オーディオ解析(analyze / compare)
    #[command(subcommand)]
    Audio(commands::audio::AudioCommand),
    /// Device Profile の検証と検証用 MIDI 生成
    #[command(subcommand)]
    Profile(commands::profile::ProfileCommand),
    /// JSON Schema の出力・ドリフト検査
    #[command(subcommand)]
    Schema(commands::schema::SchemaCommand),
    /// 環境設定の変更(control level 等)
    #[command(subcommand)]
    Config(commands::config::ConfigCommand),
    /// DAW 統合(probe / read / transport / write-clip / render / setup)
    #[command(subcommand)]
    Daw(commands::daw::DawCommand),
    /// オートメーションの適用
    #[command(subcommand)]
    Automation(commands::automation::AutomationCommand),
    /// プロジェクト雛形の生成
    Init(commands::project::InitArgs),
    /// バージョンスナップショットの作成
    #[command(subcommand)]
    Version(commands::project::VersionCommand),
    /// 環境診断
    Doctor,
    /// MCP サーバー(stdio。stdout はトランスポート専用)
    #[command(subcommand)]
    Mcp(commands::mcp::McpCommand),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: CmdResult = match cli.command {
        // MCP サーバーは stdout をトランスポートに使うため、JSON 出力経路を通さない
        Command::Mcp(cmd) => return cmd.run_blocking(),
        Command::Midi(cmd) => cmd.run(),
        Command::Audio(cmd) => cmd.run(),
        Command::Profile(cmd) => cmd.run(),
        Command::Schema(cmd) => cmd.run(),
        Command::Config(cmd) => cmd.run(),
        Command::Daw(cmd) => cmd.run(),
        Command::Automation(cmd) => cmd.run(),
        Command::Init(args) => args.run(),
        Command::Version(cmd) => cmd.run(),
        Command::Doctor => commands::project::doctor(),
    };

    match result {
        Ok(value) => {
            emit_success(&value);
            ExitCode::SUCCESS
        }
        Err(err) => emit_error(&err),
    }
}
