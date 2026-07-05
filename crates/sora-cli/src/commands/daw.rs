//! `sora daw` — DAW 統合コマンド群(技術要件書 §5 Phase 4〜5, §11)。
//!
//! 要求 control level: probe/read = 3、transport/write-clip = 4、render = 5
//! (§2.4)。setup は DAW 統合を有効化する環境セットアップとして level 3。
//! level 4+ の書き込みは WriteReceipt(undo 情報)を actions.jsonl に記録する(§11.4)。

use std::path::PathBuf;

use clap::Subcommand;
use serde_json::json;
use sora_core::model::SoraConfig;
use sora_core::validate::load_validated;
use sora_daw::adapter::{self, DawAdapter};
use sora_daw::studio_one::{StudioOneAdapter, StudioOnePaths, setup};
use sora_daw::types::{TransportCmd, WriteClipRequest};
use sora_mcp::gate;

use crate::commands::record_action;
use crate::output::CmdResult;

#[derive(Subcommand)]
pub enum DawCommand {
    /// 接続可能なアダプタとケイパビリティの検出(level 3)
    Probe(RootArgs),
    /// DAW プロジェクト状態の読み取り + project-context への反映提案(level 3)
    Read(ReadArgs),
    /// トランスポート制御(level 4)
    Transport(TransportArgs),
    /// MIDI クリップの配置(level 4)
    WriteClip(WriteClipArgs),
    /// ステム/ミックスのレンダリング要求(level 5)
    Render(RenderArgs),
    /// DAW 統合のセットアップ(Bridge 拡張 + Sora Surface の冪等インストール)
    #[command(subcommand)]
    Setup(SetupCommand),
}

impl DawCommand {
    pub fn run(self) -> CmdResult {
        match self {
            DawCommand::Probe(a) => a.run_probe(),
            DawCommand::Read(a) => a.run(),
            DawCommand::Transport(a) => a.run(),
            DawCommand::WriteClip(a) => a.run(),
            DawCommand::Render(a) => a.run(),
            DawCommand::Setup(c) => c.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct RootArgs {
    /// プロジェクトルート(既定: カレントディレクトリ)
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// アダプタの明示指定("studio-one" / "generic"。省略時は config の daw.name から解決)
    #[arg(long)]
    adapter: Option<String>,
}

impl RootArgs {
    fn config(&self) -> Option<SoraConfig> {
        load_validated(&self.root.join("sora.config.json")).ok()
    }

    fn resolve(&self) -> Box<dyn DawAdapter> {
        let config = self.config();
        match self.adapter.as_deref() {
            Some("generic") => Box::new(sora_daw::generic::GenericFileAdapter::new(&self.root)),
            Some("studio-one") => Box::new(StudioOneAdapter::new(&self.root, config.as_ref())),
            _ => adapter::resolve_adapter(&self.root, config.as_ref()),
        }
    }

    fn run_probe(self) -> CmdResult {
        gate::require(&self.root, "daw_probe", 3)?;
        let config = self.config();
        let resolved = adapter::resolve_adapter(&self.root, config.as_ref());
        Ok(json!({
            "resolved_adapter": resolved.name(),
            "adapters": adapter::probe_all(&self.root, config.as_ref()),
        }))
    }
}

#[derive(clap::Args)]
pub struct ReadArgs {
    #[command(flatten)]
    common: RootArgs,
    /// 読み取る .song のパス(省略時は config の daw.studio_one.song_path)
    #[arg(long)]
    song: Option<PathBuf>,
}

impl ReadArgs {
    fn run(self) -> CmdResult {
        gate::require(&self.common.root, "read_daw_project", 3)?;
        let mut config = self.common.config();
        // --song 明示指定は config より優先
        if let Some(song) = &self.song {
            let cfg = config.get_or_insert_with(minimal_config);
            let daw = cfg.daw.get_or_insert_with(|| sora_core::model::DawInfo {
                name: "Studio One".to_string(),
                version: None,
                os: None,
                studio_one: None,
            });
            daw.studio_one
                .get_or_insert_with(Default::default)
                .song_path = Some(song.to_string_lossy().to_string());
        }
        let mut adapter = match self.common.adapter.as_deref() {
            Some("generic") => Box::new(sora_daw::generic::GenericFileAdapter::new(
                &self.common.root,
            )) as Box<dyn DawAdapter>,
            _ => Box::new(StudioOneAdapter::new(&self.common.root, config.as_ref())),
        };
        let state = adapter.read_project()?;

        // §11.3: 手動記述と衝突する値は上書きせず両論併記 → 反映の判断は Agent が行う。
        // CLI は差分の提案(fill = 空欄を埋める / conflict = 両論併記が必要)まで。
        let context: Option<sora_core::model::ProjectContext> =
            load_validated(&self.common.root.join("project-context.json")).ok();
        let suggestions = merge_suggestions(context.as_ref(), &state);

        record_action(
            &self.common.root,
            "daw.read",
            json!({ "source": state.source, "tracks": state.tracks.len() }),
        );
        Ok(json!({
            "state": state,
            "merge_suggestions": suggestions,
            "note": "project-context.json への反映は提案を確認のうえ Agent/ユーザーが行う(confidence: daw、衝突は両論併記。§11.3)",
        }))
    }
}

fn minimal_config() -> SoraConfig {
    SoraConfig {
        schema_version: "1.0".to_string(),
        daw: None,
        control_level: 1,
        devices: vec![],
        preferences: None,
        midi: None,
        paths: None,
    }
}

/// project-context.json への反映提案を作る(機械処理のみ。判断は Agent)。
fn merge_suggestions(
    context: Option<&sora_core::model::ProjectContext>,
    state: &sora_daw::types::DawProjectState,
) -> Vec<serde_json::Value> {
    let mut suggestions = Vec::new();
    let current_bpm = context.and_then(|c| c.bpm);
    match (current_bpm, state.bpm) {
        (None, Some(daw)) => suggestions.push(json!({
            "field": "bpm", "action": "fill", "daw_value": daw, "confidence": "daw"
        })),
        (Some(cur), Some(daw)) if (cur - daw).abs() > 0.01 => suggestions.push(json!({
            "field": "bpm", "action": "conflict", "current": cur, "daw_value": daw,
            "note": "手動記述と DAW 由来が衝突。上書きせず両論併記し、ユーザーに確認する"
        })),
        _ => {}
    }
    let current_ts = context.and_then(|c| c.time_signature.clone());
    match (current_ts, state.time_signature.clone()) {
        (None, Some(daw)) => suggestions.push(json!({
            "field": "time_signature", "action": "fill", "daw_value": daw, "confidence": "daw"
        })),
        (Some(cur), Some(daw)) if cur != daw => suggestions.push(json!({
            "field": "time_signature", "action": "conflict", "current": cur, "daw_value": daw,
            "note": "手動記述と DAW 由来が衝突。上書きせず両論併記し、ユーザーに確認する"
        })),
        _ => {}
    }
    // context の tracks に無い DAW トラックを列挙
    let known: Vec<String> = context
        .map(|c| {
            c.tracks
                .iter()
                .map(|t| t.id.to_lowercase().replace(['-', '_'], " "))
                .collect()
        })
        .unwrap_or_default();
    for track in &state.tracks {
        let name = track.name.to_lowercase().replace(['-', '_'], " ");
        if !name.is_empty() && !known.iter().any(|k| k == &name) {
            suggestions.push(json!({
                "field": "tracks", "action": "add", "daw_value": track,
            }));
        }
    }
    suggestions
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum TransportArg {
    Play,
    Stop,
    Record,
    Rewind,
    Forward,
    ReturnToZero,
}

impl From<TransportArg> for TransportCmd {
    fn from(value: TransportArg) -> Self {
        match value {
            TransportArg::Play => TransportCmd::Play,
            TransportArg::Stop => TransportCmd::Stop,
            TransportArg::Record => TransportCmd::Record,
            TransportArg::Rewind => TransportCmd::Rewind,
            TransportArg::Forward => TransportCmd::Forward,
            TransportArg::ReturnToZero => TransportCmd::ReturnToZero,
        }
    }
}

#[derive(clap::Args)]
pub struct TransportArgs {
    /// 操作
    #[arg(value_enum)]
    cmd: TransportArg,
    #[command(flatten)]
    common: RootArgs,
}

impl TransportArgs {
    fn run(self) -> CmdResult {
        gate::require(&self.common.root, "daw_transport", 4)?;
        let cmd: TransportCmd = self.cmd.into();
        let state = self.common.resolve().transport(cmd)?;
        record_action(
            &self.common.root,
            "daw.transport",
            json!({ "cmd": cmd.as_str(), "delivered": state.delivered }),
        );
        Ok(serde_json::to_value(state)?)
    }
}

#[derive(clap::Args)]
pub struct WriteClipArgs {
    /// 配置する SMF ファイル
    file: PathBuf,
    /// 配置先トラックのヒント
    #[arg(long)]
    track: Option<String>,
    #[command(flatten)]
    common: RootArgs,
}

impl WriteClipArgs {
    fn run(self) -> CmdResult {
        gate::require(&self.common.root, "write_clip", 4)?;
        let receipt = self.common.resolve().write_clip(WriteClipRequest {
            midi_file: self.file,
            track_hint: self.track,
        })?;
        // §11.4: undo 情報を必ず actions.jsonl に残す
        record_action(
            &self.common.root,
            "daw.write_clip",
            serde_json::to_value(&receipt)?,
        );
        Ok(serde_json::to_value(receipt)?)
    }
}

#[derive(clap::Args)]
pub struct RenderArgs {
    /// 出力先パス
    #[arg(long)]
    out: PathBuf,
    /// 対象トラック(省略時はミックス全体)
    #[arg(long)]
    track: Option<String>,
    #[command(flatten)]
    common: RootArgs,
}

impl RenderArgs {
    fn run(self) -> CmdResult {
        gate::require(&self.common.root, "render_stem", 5)?;
        let receipt = self
            .common
            .resolve()
            .render(sora_daw::types::RenderRequest {
                track: self.track,
                out: self.out,
            })?;
        record_action(
            &self.common.root,
            "daw.render",
            serde_json::to_value(&receipt)?,
        );
        Ok(serde_json::to_value(receipt)?)
    }
}

#[derive(Subcommand)]
pub enum SetupCommand {
    /// Studio One 5: Sora Bridge 拡張 + Sora Surface のインストール/診断/退避
    StudioOne(SetupStudioOneArgs),
}

impl SetupCommand {
    fn run(self) -> CmdResult {
        match self {
            SetupCommand::StudioOne(a) => a.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct SetupStudioOneArgs {
    #[command(flatten)]
    common: RootArgs,
    /// インストール状態の診断のみ(変更なし)
    #[arg(long)]
    check: bool,
    /// 無効化 + 退避(削除はしない)
    #[arg(long)]
    uninstall: bool,
}

impl SetupStudioOneArgs {
    fn run(self) -> CmdResult {
        let config = self.common.config();
        let paths = StudioOnePaths::resolve(config.as_ref());
        if self.check {
            // 診断は環境を変更しないため gate 不要
            return Ok(serde_json::to_value(setup::check(&paths))?);
        }
        gate::require(&self.common.root, "daw_setup", 3)?;
        let report = if self.uninstall {
            setup::uninstall(&paths)?
        } else {
            setup::install(&paths)?
        };
        record_action(
            &self.common.root,
            if self.uninstall {
                "daw.setup.uninstall"
            } else {
                "daw.setup.install"
            },
            serde_json::to_value(&report)?,
        );
        Ok(serde_json::to_value(report)?)
    }
}
