//! Sora Bridge のファイルキュー(inbox / outbox / media)。
//!
//! Studio One 側の Sora Bridge 拡張(EditTask)が inbox の JSON を実行し、
//! 処理済みファイルを outbox へ移動する(実機検証済みのプロトコル。§11.2.1)。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::DawError;

/// Bridge のディレクトリ群を扱う。
pub struct Bridge {
    dir: PathBuf,
}

impl Bridge {
    /// `<user_content>/SoraBridge` を基点にする。
    pub fn new(user_content: &Path) -> Self {
        Self {
            dir: user_content.join("SoraBridge"),
        }
    }

    pub fn inbox(&self) -> PathBuf {
        self.dir.join("inbox")
    }

    pub fn outbox(&self) -> PathBuf {
        self.dir.join("outbox")
    }

    /// Sora が生成した MIDI 等を Studio One から参照させる置き場。
    pub fn media(&self) -> PathBuf {
        self.dir.join("media")
    }

    /// インストールレシートのパス。
    pub fn receipt(&self) -> PathBuf {
        self.dir.join("install-receipt.json")
    }

    /// Bridge 拡張が導入済みか(拡張ファイルの存在で判定)。
    pub fn is_installed(&self, app_support: &Path) -> bool {
        let ext = app_support
            .join("Extensions")
            .join(super::setup::EXTENSION_ID);
        ext.join("metainfo.xml").is_file()
            && ext.join("scripts").join("sorabridge.package").is_file()
    }

    /// ファイルを media 領域へコピーする(上書きせず新パス)。
    pub fn stage_media(&self, file: &Path) -> Result<PathBuf, DawError> {
        let bytes = std::fs::read(file).map_err(|e| DawError::Io {
            path: file.to_path_buf(),
            source: e,
        })?;
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "clip.mid".to_string());
        let dest = sora_core::fsutil::unique_path(&self.media().join(name));
        sora_core::fsutil::write_new_file(&dest, &bytes).map_err(|e| DawError::Io {
            path: dest.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;
        Ok(dest)
    }

    /// リクエスト JSON を inbox へキューし、キューしたファイルパスを返す。
    pub fn queue_request(
        &self,
        request: &serde_json::Value,
        label: &str,
    ) -> Result<PathBuf, DawError> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = sora_core::fsutil::unique_path(&self.inbox().join(format!("{ts}-{label}.json")));
        let body = serde_json::to_string_pretty(request).map_err(|e| DawError::Rejected {
            operation: "queue_request".to_string(),
            reason: format!("failed to serialize request: {e}"),
        })? + "\n";
        sora_core::fsutil::write_new_file(&path, body.as_bytes()).map_err(|e| DawError::Io {
            path: path.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;
        Ok(path)
    }

    /// キューしたリクエストが消化される(inbox から消える)まで待つ。
    /// タイムアウトで false(エラーではない: 手動トリガー待ちの可能性)。
    pub fn wait_consumed(&self, queued: &Path, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if !queued.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        !queued.exists()
    }

    /// inbox に残っている未処理リクエストを列挙する(doctor 用)。
    pub fn pending_requests(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.inbox())
            .map(|entries| {
                let mut files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect();
                files.sort();
                files
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_and_consume_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::new(dir.path());
        let queued = bridge
            .queue_request(&serde_json::json!({"type": "command"}), "test")
            .unwrap();
        assert!(queued.exists());
        assert_eq!(bridge.pending_requests(), vec![queued.clone()]);

        // 消化前はタイムアウトで false
        assert!(!bridge.wait_consumed(&queued, Duration::from_millis(50)));

        // Studio One 側の処理(outbox への移動)を模す
        std::fs::create_dir_all(bridge.outbox()).unwrap();
        std::fs::rename(&queued, bridge.outbox().join("done.json")).unwrap();
        assert!(bridge.wait_consumed(&queued, Duration::from_millis(50)));
        assert!(bridge.pending_requests().is_empty());
    }

    #[test]
    fn stage_media_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::new(dir.path());
        let midi = dir.path().join("riff.mid");
        std::fs::write(&midi, b"MThd").unwrap();
        let a = bridge.stage_media(&midi).unwrap();
        let b = bridge.stage_media(&midi).unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().contains("riff-2"));
    }
}
