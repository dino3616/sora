//! サブコマンド実装。

pub mod audio;
pub mod config;
pub mod midi;
pub mod profile;
pub mod project;
pub mod schema;
pub mod send;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sora_core::error::CoreError;

/// ファイルへ書き込む(非破壊: 既存パスは上書きしない。技術要件書 §13)。
pub(crate) fn write_new_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::bail!(
            "refusing to overwrite existing file `{}` (Sora は非破壊: 別名で保存するか version snapshot を使ってください)",
            path.display()
        );
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| CoreError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::write(path, bytes).map_err(|e| CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// `<base>/logs/actions.jsonl` へ 1 行追記する(技術要件書 §8)。
/// ベストエフォート: 失敗しても本処理は成功扱い(ログのために操作を止めない)。
pub(crate) fn record_action(base: &Path, action: &str, details: serde_json::Value) {
    let log_dir = base.join("logs");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = serde_json::json!({ "ts": ts, "action": action, "details": details });
    if let Ok(line) = serde_json::to_string(&entry) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("actions.jsonl"))
        {
            let _ = writeln!(file, "{line}");
        }
    }
}
