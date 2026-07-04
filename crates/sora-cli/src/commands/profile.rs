//! `sora profile` サブコマンド(validate / verify-midi)。

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Subcommand;
use serde_json::json;
use sora_core::midi::generate_verify_midi;
use sora_core::model::DeviceProfile;
use sora_core::profile::ResolvedProfile;
use sora_core::validate::load_validated;

use super::write_new_file;
use crate::output::CmdResult;

#[derive(Subcommand)]
pub enum ProfileCommand {
    /// Device Profile をスキーマ + 整合性(音域・キースイッチ衝突等)で検証する
    Validate(ValidateArgs),
    /// 全奏法・全キットピースを鳴らす検証用 MIDI を生成する(実機確認用)
    VerifyMidi(VerifyMidiArgs),
}

impl ProfileCommand {
    pub fn run(self) -> CmdResult {
        match self {
            ProfileCommand::Validate(a) => a.run(),
            ProfileCommand::VerifyMidi(a) => a.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct ValidateArgs {
    /// Device Profile JSON のパス
    profile: PathBuf,
}

impl ValidateArgs {
    fn run(self) -> CmdResult {
        // L1/L2(スキーマ + 型)
        let profile: DeviceProfile = load_validated(&self.profile)
            .with_context(|| format!("validating {}", self.profile.display()))?;
        // L3(ドメイン整合性)
        let resolved = ResolvedProfile::resolve(&profile)
            .with_context(|| format!("resolving {}", self.profile.display()))?;

        // 確信度の集計(unverified があれば実機確認を促す)。
        // instrument は articulation(キースイッチ)、effect は parameters を対象にする。
        let unverified_articulations: Vec<&str> = profile
            .keyswitches
            .iter()
            .filter(|k| matches!(k.confidence, sora_core::model::Confidence::Unverified))
            .map(|k| k.articulation.as_str())
            .collect();
        let unverified_parameters: Vec<&str> = profile
            .parameters
            .iter()
            .filter(|p| matches!(p.confidence, sora_core::model::Confidence::Unverified))
            .map(|p| p.name.as_str())
            .collect();

        let mut hints = Vec::new();
        if !unverified_articulations.is_empty() {
            hints.push("unverified な奏法があります。`sora profile verify-midi` で実機確認し confidence を verified へ更新してください");
        }
        if !unverified_parameters.is_empty() {
            hints.push("unverified なパラメータがあります。実機で値域・挙動を確認し confidence を verified へ更新してください");
        }

        Ok(json!({
            "valid": true,
            "id": resolved.id,
            "device_type": profile.device_type,
            "octave_convention": profile.octave_convention,
            "keyswitch_count": resolved.keyswitches.len(),
            "drum_piece_count": resolved.drum_map.len(),
            "parameter_count": profile.parameters.len(),
            "unverified_articulations": unverified_articulations,
            "unverified_parameters": unverified_parameters,
            "hint": if hints.is_empty() { serde_json::Value::Null } else { json!(hints.join(" / ")) }
        }))
    }
}

#[derive(clap::Args)]
pub struct VerifyMidiArgs {
    /// Device Profile JSON のパス
    profile: PathBuf,
    /// 出力 .mid のパス(省略時は profile と同ディレクトリに <id>.verify.mid)
    #[arg(long)]
    out: Option<PathBuf>,
}

impl VerifyMidiArgs {
    fn run(self) -> CmdResult {
        let profile: DeviceProfile = load_validated(&self.profile)
            .with_context(|| format!("loading {}", self.profile.display()))?;
        let verify = generate_verify_midi(&profile)
            .with_context(|| format!("generating verify MIDI for {}", self.profile.display()))?;

        let out_path = self.out.unwrap_or_else(|| {
            let dir = self.profile.parent().unwrap_or_else(|| Path::new("."));
            dir.join(format!("{}.verify.mid", profile.id))
        });
        write_new_file(&out_path, &verify.bytes)?;

        Ok(json!({
            "output": out_path,
            "items": verify.items,
            "instructions": "この MIDI を DAW にインポートし、対象デバイスで再生してください。各項目の bar とノートが期待どおり発音されるか確認し、正しければ Profile の該当 confidence を verified に更新してください。",
        }))
    }
}
