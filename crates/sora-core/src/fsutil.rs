//! ファイル書き込みユーティリティ(非破壊規約。技術要件書 §8, §13)。

use std::path::{Path, PathBuf};

use crate::error::CoreError;

/// ファイルへ書き込む(非破壊: 既存パスは上書きしない)。
pub fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    if path.exists() {
        return Err(CoreError::FileExists {
            path: path.to_path_buf(),
        });
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

/// 既存パスと衝突しないパスを返す(`name.ext` → `name-2.ext` → `name-3.ext` …)。
/// 生成物の再出力など「上書きせず新パス」で保存したい場合に使う。
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for n in 2u32.. {
        let candidate = parent.join(format!("{stem}-{n}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // 2..u32::MAX まで全衝突は現実に起こらない
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_new_file_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        write_new_file(&path, b"one").unwrap();
        let err = write_new_file(&path, b"two").unwrap_err();
        assert_eq!(err.code(), "FILE_EXISTS");
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
    }

    #[test]
    fn unique_path_appends_counter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mid");
        assert_eq!(unique_path(&path), path);
        std::fs::write(&path, b"x").unwrap();
        assert_eq!(unique_path(&path), dir.path().join("clip-2.mid"));
        std::fs::write(dir.path().join("clip-2.mid"), b"x").unwrap();
        assert_eq!(unique_path(&path), dir.path().join("clip-3.mid"));
    }
}
