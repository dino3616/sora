//! `sora midi` サブコマンド(compile / inspect / analyze)。

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Subcommand;
use serde_json::json;
use sora_core::error::CoreError;
use sora_core::midi;
use sora_core::model::{DeviceProfile, PartPlan, SoraConfig};
use sora_core::validate::load_validated;

use super::write_new_file;
use crate::output::CmdResult;

#[derive(Subcommand)]
pub enum MidiCommand {
    /// Part Plan を検証して SMF へコンパイルする
    Compile(CompileArgs),
    /// SMF を読み取り、ノート/テンポ/CC と統計を出力する
    Inspect(InspectArgs),
    /// SMF を解析し、BPM/調性中心/リズム統計を推定する
    Analyze(AnalyzeArgs),
    /// SMF を Part Plan へ逆コンパイルする(キースイッチを articulation へ逆解決)
    Decompile(DecompileArgs),
    /// 仮想 MIDI ポートへ実時間送信する(control level 2+)
    Send(super::send::SendArgs),
    /// 全チャンネルへ All-Notes-Off を送る(鳴りっぱなしの手動リセット)
    Panic(super::send::PanicArgs),
}

impl MidiCommand {
    pub fn run(self) -> CmdResult {
        match self {
            MidiCommand::Compile(a) => a.run(),
            MidiCommand::Inspect(a) => a.run(),
            MidiCommand::Analyze(a) => a.run(),
            MidiCommand::Decompile(a) => a.run(),
            MidiCommand::Send(a) => a.run(),
            MidiCommand::Panic(a) => a.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct CompileArgs {
    /// Part Plan JSON のパス
    plan: PathBuf,
    /// Device Profile JSON のパス(省略時は sora.config.json から device を解決)
    #[arg(long)]
    profile: Option<PathBuf>,
    /// 出力 .mid のパス(省略時は plan と同ディレクトリに <part_id>.mid)
    #[arg(long)]
    out: Option<PathBuf>,
    /// 参照する sora.config.json(profile 未指定時に使用。既定: ./sora.config.json)
    #[arg(long)]
    config: Option<PathBuf>,
}

impl CompileArgs {
    fn run(self) -> CmdResult {
        let plan: PartPlan = load_validated(&self.plan)
            .with_context(|| format!("loading plan {}", self.plan.display()))?;
        let profile = resolve_profile(
            &plan.device,
            self.profile.as_deref(),
            self.config.as_deref(),
        )
        .with_context(|| format!("resolving profile for device `{}`", plan.device))?;

        let output = midi::compile(&plan, &profile)
            .with_context(|| format!("compiling {}", self.plan.display()))?;

        let out_path = self.out.unwrap_or_else(|| {
            let dir = self.plan.parent().unwrap_or_else(|| Path::new("."));
            dir.join(format!("{}.mid", plan.part_id))
        });
        write_new_file(&out_path, &output.bytes)?;

        Ok(json!({
            "output": out_path,
            "report": output.report,
        }))
    }
}

#[derive(clap::Args)]
pub struct InspectArgs {
    /// SMF ファイルのパス
    file: PathBuf,
    /// 全ノートをダンプに含める
    #[arg(long)]
    notes: bool,
}

impl InspectArgs {
    fn run(self) -> CmdResult {
        let inspection = midi::inspect_file(&self.file, self.notes)
            .with_context(|| format!("inspecting {}", self.file.display()))?;
        Ok(serde_json::to_value(inspection)?)
    }
}

#[derive(clap::Args)]
pub struct AnalyzeArgs {
    /// SMF ファイルのパス
    file: PathBuf,
}

impl AnalyzeArgs {
    fn run(self) -> CmdResult {
        let analysis = midi::analyze_file(&self.file)
            .with_context(|| format!("analyzing {}", self.file.display()))?;
        Ok(serde_json::to_value(analysis)?)
    }
}

#[derive(clap::Args)]
pub struct DecompileArgs {
    /// SMF ファイルのパス
    file: PathBuf,
    /// 対象デバイス ID(profile 解決に使用)
    #[arg(long)]
    device: String,
    /// Device Profile JSON のパス(省略時は sora.config.json から device を解決)
    #[arg(long)]
    profile: Option<PathBuf>,
    /// 生成 Part Plan の part_id(省略時はファイル名から)
    #[arg(long)]
    part_id: Option<String>,
    /// 出力 .plan.json のパス(省略時は stdout の plan フィールドのみ)
    #[arg(long)]
    out: Option<PathBuf>,
    /// 参照する sora.config.json(既定: ./sora.config.json)
    #[arg(long)]
    config: Option<PathBuf>,
}

impl DecompileArgs {
    fn run(self) -> CmdResult {
        let profile = resolve_profile(
            &self.device,
            self.profile.as_deref(),
            self.config.as_deref(),
        )
        .with_context(|| format!("resolving profile for device `{}`", self.device))?;
        let part_id = self.part_id.unwrap_or_else(|| {
            self.file
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "decompiled".to_string())
        });
        let output = midi::decompile_file(&self.file, &profile, &part_id)
            .with_context(|| format!("decompiling {}", self.file.display()))?;

        if let Some(out_path) = self.out {
            let json = serde_json::to_string_pretty(&output.plan)? + "\n";
            write_new_file(&out_path, json.as_bytes())?;
            return Ok(json!({ "output": out_path, "summary": output.summary }));
        }
        Ok(json!({ "plan": output.plan, "summary": output.summary }))
    }
}

/// Profile を解決する。--profile 明示があればそれを、なければ config 経由で。
fn resolve_profile(
    device_id: &str,
    profile: Option<&Path>,
    config: Option<&Path>,
) -> anyhow::Result<DeviceProfile> {
    if let Some(path) = profile {
        return Ok(load_validated(path)?);
    }
    let config_path = config.unwrap_or_else(|| Path::new("sora.config.json"));
    let cfg: SoraConfig = load_validated(config_path).with_context(|| {
        format!(
            "no --profile given and could not read config {} to resolve device `{}`",
            config_path.display(),
            device_id
        )
    })?;
    let entry = cfg
        .devices
        .iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| CoreError::Validation {
            issues: vec![sora_core::error::ValidationIssue {
                pointer: "/device".to_string(),
                code: "DEVICE_NOT_IN_CONFIG".to_string(),
                message: format!(
                    "device `{device_id}` not found in {}",
                    config_path.display()
                ),
                hint: Some(
                    "sora.config.json の devices に追加するか --profile で明示してください"
                        .to_string(),
                ),
            }],
        })?;
    // profile パスは config からの相対
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(load_validated(&base.join(&entry.profile))?)
}
