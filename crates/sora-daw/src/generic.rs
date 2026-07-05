//! Generic(file-based)アダプタ — 全 DAW で動く常設フォールバック(§11.2)。
//!
//! capabilities は「出力のみ」: MIDI クリップは `exports/daw-import/` へ配置して
//! インポート手順を提示し、オートメーションは手動適用手順書(Markdown)を生成する。
//! DAW には一切接触しないため、undo は生成ファイルの削除だけで完結する。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sora_core::fsutil;
use sora_core::model::AutomationPlan;

use crate::adapter::{DawAdapter, not_supported};
use crate::error::DawError;
use crate::types::{
    DawCapabilities, DawProjectState, RenderReceipt, RenderRequest, TransportCmd, TransportState,
    WriteClipRequest, WriteReceipt, WriteStatus,
};

const ADAPTER: &str = "generic";

/// ファイル書き出しのみを行うアダプタ。
pub struct GenericFileAdapter {
    root: PathBuf,
}

impl GenericFileAdapter {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    fn import_dir(&self) -> PathBuf {
        self.root.join("exports").join("daw-import")
    }
}

impl DawAdapter for GenericFileAdapter {
    fn name(&self) -> &'static str {
        ADAPTER
    }

    fn capabilities(&self) -> DawCapabilities {
        DawCapabilities {
            adapter: ADAPTER.to_string(),
            read: false,
            transport: false,
            write_clip: true,
            write_automation: true,
            render: false,
            selection: false,
            notes: vec![
                "ファイル書き出しのみ(DAW には接触しない)。write_clip は exports/daw-import/ へ配置し、インポートは手動".to_string(),
                "write_automation は手動適用手順書(Markdown)の生成".to_string(),
            ],
        }
    }

    fn read_project(&mut self) -> Result<DawProjectState, DawError> {
        Err(not_supported(
            ADAPTER,
            "read_project",
            "project-context.json を手動で記述するか、対応 DAW のアダプタ(studio-one)を設定してください",
        ))
    }

    fn transport(&mut self, _cmd: TransportCmd) -> Result<TransportState, DawError> {
        Err(not_supported(
            ADAPTER,
            "transport",
            "DAW 上で手動操作してください。試聴目的なら send_midi(level 2)が使えます",
        ))
    }

    fn write_clip(&mut self, req: WriteClipRequest) -> Result<WriteReceipt, DawError> {
        let bytes = std::fs::read(&req.midi_file).map_err(|e| DawError::Io {
            path: req.midi_file.clone(),
            source: e,
        })?;
        let file_name = req
            .midi_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "clip.mid".to_string());
        let dest = fsutil::unique_path(&self.import_dir().join(&file_name));
        fsutil::write_new_file(&dest, &bytes).map_err(|e| DawError::Io {
            path: dest.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;

        let track = req.track_hint.as_deref().unwrap_or("新規トラック");
        Ok(WriteReceipt {
            action: "write_clip".to_string(),
            adapter: ADAPTER.to_string(),
            status: WriteStatus::Exported,
            target: format!("{}(手動インポート先: {track})", dest.display()),
            files: vec![dest.clone()],
            backup: None,
            undo: vec![format!(
                "{} を削除する(DAW には未接触のためこれで完了)",
                dest.display()
            )],
            notes: vec![format!(
                "DAW で {} をインポートしてください(Studio One: Song > Import File / ドラッグ&ドロップ)",
                dest.display()
            )],
        })
    }

    fn write_automation(&mut self, plan: &AutomationPlan) -> Result<WriteReceipt, DawError> {
        let mut doc = String::new();
        let _ = writeln!(
            doc,
            "# オートメーション適用手順: {} / {}",
            plan.target.track, plan.target.parameter
        );
        let _ = writeln!(doc);
        let _ = writeln!(doc, "- トラック: `{}`", plan.target.track);
        let _ = writeln!(doc, "- デバイス: `{}`", plan.target.device);
        let _ = writeln!(doc, "- パラメータ: `{}`", plan.target.parameter);
        if let Some(unit) = &plan.unit {
            let _ = writeln!(doc, "- 単位: {unit}");
        }
        if let Some(rationale) = &plan.rationale {
            let _ = writeln!(doc);
            let _ = writeln!(doc, "**意図**: {rationale}");
        }
        let _ = writeln!(doc);
        let _ = writeln!(doc, "| 位置 (bar.beat.tick) | 値 | カーブ |");
        let _ = writeln!(doc, "|---|---|---|");
        for point in &plan.points {
            let _ = writeln!(
                doc,
                "| {} | {} | {:?} |",
                point.at, point.value, point.curve
            );
        }
        let _ = writeln!(doc);
        let _ = writeln!(
            doc,
            "DAW で `{}` のオートメーションレーンを表示し、上記の制御点を打ってください。",
            plan.target.parameter
        );

        let name = format!(
            "{}-{}.automation.md",
            plan.target.track, plan.target.parameter
        );
        let dest = fsutil::unique_path(&self.import_dir().join(name));
        fsutil::write_new_file(&dest, doc.as_bytes()).map_err(|e| DawError::Io {
            path: dest.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;

        Ok(WriteReceipt {
            action: "write_automation".to_string(),
            adapter: ADAPTER.to_string(),
            status: WriteStatus::Exported,
            target: format!("手動適用手順書 {}", dest.display()),
            files: vec![dest.clone()],
            backup: None,
            undo: vec![format!(
                "{} を削除する(DAW には未接触のためこれで完了)",
                dest.display()
            )],
            notes: vec!["手順書に従って DAW 上で手動適用してください".to_string()],
        })
    }

    fn render(&mut self, _req: RenderRequest) -> Result<RenderReceipt, DawError> {
        Err(not_supported(
            ADAPTER,
            "render",
            "DAW 上で手動レンダリングし、結果を analyze_audio に渡してください",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sora_core::model::{AutomationPoint, AutomationTarget};

    fn plan() -> AutomationPlan {
        AutomationPlan {
            schema_version: "1.0".to_string(),
            target: AutomationTarget {
                track: "bass".to_string(),
                device: "test".to_string(),
                parameter: "drive".to_string(),
            },
            unit: Some("%".to_string()),
            points: vec![AutomationPoint {
                at: "1.1.000".to_string(),
                value: 30.0,
                curve: Default::default(),
            }],
            rationale: Some("サビ前の高揚感".to_string()),
        }
    }

    #[test]
    fn write_clip_exports_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let midi = dir.path().join("riff.mid");
        std::fs::write(&midi, b"MThd").unwrap();

        let mut adapter = GenericFileAdapter::new(dir.path());
        let req = WriteClipRequest {
            midi_file: midi.clone(),
            track_hint: None,
        };
        let first = adapter.write_clip(req.clone()).unwrap();
        assert_eq!(first.status, WriteStatus::Exported);
        assert!(first.files[0].ends_with("exports/daw-import/riff.mid"));
        assert!(first.backup.is_none());

        // 同名の再出力は上書きせず新パス
        let second = adapter.write_clip(req).unwrap();
        assert!(second.files[0].ends_with("exports/daw-import/riff-2.mid"));
    }

    #[test]
    fn write_automation_generates_instructions() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = GenericFileAdapter::new(dir.path());
        let receipt = adapter.write_automation(&plan()).unwrap();
        let doc = std::fs::read_to_string(&receipt.files[0]).unwrap();
        assert!(doc.contains("drive"));
        assert!(doc.contains("1.1.000"));
        assert!(doc.contains("サビ前の高揚感"));
    }

    #[test]
    fn unsupported_operations_return_fallback_hint() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = GenericFileAdapter::new(dir.path());
        let err = adapter.read_project().unwrap_err();
        assert_eq!(err.code(), "DAW_NOT_SUPPORTED");
        assert!(err.hint().unwrap().contains("studio-one"));
    }
}
