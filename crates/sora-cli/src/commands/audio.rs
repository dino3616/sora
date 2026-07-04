//! `sora audio` サブコマンド(analyze / compare)。

use std::path::PathBuf;

use anyhow::Context;
use clap::Subcommand;
use sora_audio::{analyze_file, compare_files};

use crate::output::CmdResult;

#[derive(Subcommand)]
pub enum AudioCommand {
    /// ラウドネス・帯域バランス・ダイナミクスを測定する
    Analyze(AnalyzeArgs),
    /// 2 ファイルの解析差分を出す(A/B 比較・トーンマッチング)
    Compare(CompareArgs),
}

impl AudioCommand {
    pub fn run(self) -> CmdResult {
        match self {
            AudioCommand::Analyze(a) => a.run(),
            AudioCommand::Compare(a) => a.run(),
        }
    }
}

#[derive(clap::Args)]
pub struct AnalyzeArgs {
    /// オーディオファイル(WAV/AIFF/FLAC/MP3 等)
    file: PathBuf,
}

impl AnalyzeArgs {
    fn run(self) -> CmdResult {
        let analysis = analyze_file(&self.file)
            .with_context(|| format!("analyzing {}", self.file.display()))?;
        Ok(serde_json::to_value(analysis)?)
    }
}

#[derive(clap::Args)]
pub struct CompareArgs {
    /// 基準ファイル(A)
    a: PathBuf,
    /// 比較ファイル(B)。delta は B - A
    b: PathBuf,
}

impl CompareArgs {
    fn run(self) -> CmdResult {
        let comparison = compare_files(&self.a, &self.b)
            .with_context(|| format!("comparing {} vs {}", self.a.display(), self.b.display()))?;
        Ok(serde_json::to_value(comparison)?)
    }
}
