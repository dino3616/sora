//! `sora schema dump` サブコマンド。

use std::path::PathBuf;

use serde_json::json;
use sora_core::error::CoreError;
use sora_core::schema::dump_all;

use crate::output::CmdResult;

#[derive(clap::Args)]
pub struct SchemaArgs {
    /// 出力ディレクトリ(省略時は stdout に JSON 集約を返す)
    #[arg(long)]
    out: Option<PathBuf>,
    /// 生成物と既存ファイルの差分を検査し、ドリフトがあれば失敗する(CI 用)
    #[arg(long)]
    check: bool,
}

impl SchemaArgs {
    pub fn run(self) -> CmdResult {
        let schemas = dump_all();

        // --out なし: stdout に集約
        let Some(dir) = self.out else {
            let map: serde_json::Map<String, serde_json::Value> = schemas
                .iter()
                .map(|(name, json)| {
                    let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
                    (format!("{name}.schema.json"), value)
                })
                .collect();
            return Ok(serde_json::Value::Object(map));
        };

        if self.check {
            let mut drifted: Vec<String> = Vec::new();
            for (name, expected) in &schemas {
                let path = dir.join(format!("{name}.schema.json"));
                let actual = std::fs::read_to_string(&path).unwrap_or_default();
                if actual != *expected {
                    drifted.push(format!("{name}.schema.json"));
                }
            }
            if !drifted.is_empty() {
                return Err(CoreError::Validation {
                    issues: drifted
                        .iter()
                        .map(|f| sora_core::error::ValidationIssue {
                            pointer: format!("/{f}"),
                            code: "SCHEMA_DRIFT".to_string(),
                            message: format!("{f} is out of date"),
                            hint: Some(
                                "`sora schema dump --out schemas/` で再生成してください"
                                    .to_string(),
                            ),
                        })
                        .collect(),
                }
                .into());
            }
            return Ok(json!({ "checked": schemas.len(), "drift": false }));
        }

        std::fs::create_dir_all(&dir).map_err(|e| CoreError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let mut written = Vec::new();
        for (name, json) in &schemas {
            let path = dir.join(format!("{name}.schema.json"));
            std::fs::write(&path, json).map_err(|e| CoreError::Io {
                path: path.clone(),
                source: e,
            })?;
            written.push(path);
        }
        Ok(json!({ "written": written }))
    }
}
