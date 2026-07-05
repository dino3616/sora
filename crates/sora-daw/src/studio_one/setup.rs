//! `sora daw setup studio-one` — Sora Bridge 拡張 + Sora Surface の冪等インストール
//! (技術要件書 §11.2.1「初期セットアップ」。Codex 検証済みインストーラの Rust 移植)。
//!
//! 安全規約(§11.4 の適用):
//! - 既存 `.song` には一切触れない
//! - 管理対象は `sora.studioone.bridge` 拡張と `Sora Surface` デバイス定義のみ
//! - Studio One の設定ファイルを書き換える前に必ずタイムスタンプ付きバックアップを取る
//! - アンインストール手段を提供する(無効化 + 退避。削除はしない)
//! - Studio One の再起動はユーザーに依頼する(自動では行わない)

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::StudioOnePaths;
use crate::error::DawError;

/// Bridge 拡張の ID(Extensions ディレクトリ名・settings のセクション名)。
pub const EXTENSION_ID: &str = "sora.studioone.bridge";
/// Bridge ProgramService のクラス ID(classfactory.xml と一致)。
pub const SERVICE_CLASS_ID: &str = "{F8C017B7-9B57-4D94-9B01-6FE33D8F4099}";

const SERVICE_JS: &str = include_str!("../../assets/studio-one/service.js");
const CLASSFACTORY_XML: &str = include_str!("../../assets/studio-one/classfactory.xml");
const PACKAGE_METAINFO_XML: &str = include_str!("../../assets/studio-one/package-metainfo.xml");
const EXT_METAINFO_XML: &str = include_str!("../../assets/studio-one/ext-metainfo.xml");
const INSTALLDATA_XML: &str = include_str!("../../assets/studio-one/installdata.xml");
const SURFACE_DEVICE_XML: &str = include_str!("../../assets/studio-one/Sora Surface.device");
const SURFACE_XML: &str = include_str!("../../assets/studio-one/Sora Surface.surface.xml");

/// セットアップ結果。
#[derive(Debug, Serialize)]
pub struct SetupReport {
    /// 配置・更新したファイル
    pub installed: Vec<PathBuf>,
    /// 書き換えた Studio One 設定ファイル
    pub settings_updated: Vec<PathBuf>,
    /// 設定ファイルのバックアップ
    pub backups: Vec<PathBuf>,
    /// ユーザーに残る手順
    pub user_steps: Vec<String>,
    /// Studio One の再起動が必要か
    pub requires_restart: bool,
}

/// 診断 1 項目。
#[derive(Debug, Serialize)]
pub struct CheckItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// 診断結果。
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub ok: bool,
    pub items: Vec<CheckItem>,
}

fn io_err(path: &Path, e: std::io::Error) -> DawError {
    DawError::Io {
        path: path.to_path_buf(),
        source: e,
    }
}

fn write_file(path: &Path, content: &[u8]) -> Result<(), DawError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    std::fs::write(path, content).map_err(|e| io_err(path, e))
}

/// sorabridge.package(ZIP)をメモリ上で組み立てる。
fn build_package() -> Result<Vec<u8>, DawError> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in [
            ("service.js", SERVICE_JS),
            ("classfactory.xml", CLASSFACTORY_XML),
            ("metainfo.xml", PACKAGE_METAINFO_XML),
        ] {
            zip.start_file(name, options)
                .map_err(|e| DawError::Rejected {
                    operation: "build_package".to_string(),
                    reason: e.to_string(),
                })?;
            zip.write_all(content.as_bytes())
                .map_err(|e| io_err(Path::new(name), e))?;
        }
        zip.finish().map_err(|e| DawError::Rejected {
            operation: "build_package".to_string(),
            reason: e.to_string(),
        })?;
    }
    Ok(buf.into_inner())
}

/// Settings XML(PreSonus CCL 形式)に Section を挿入または更新する。
/// 形式は機械生成で安定しているため、文字列手術で十分(実機の
/// Extensions.settings で検証済みの手法。Codex インストーラと同じ)。
fn upsert_section(content: &str, section_path: &str, attributes: &str) -> String {
    let section_tag = format!("<Section path=\"{section_path}\">");
    if let Some(section_start) = content.find(&section_tag) {
        // 既存セクションの <Attributes .../> を置き換える
        let after = &content[section_start..];
        if let Some(attr_rel) = after.find("<Attributes")
            && let Some(end_rel) = after[attr_rel..].find("/>")
        {
            let attr_abs = section_start + attr_rel;
            let end_abs = attr_abs + end_rel + 2;
            let mut result = String::with_capacity(content.len());
            result.push_str(&content[..attr_abs]);
            result.push_str(attributes);
            result.push_str(&content[end_abs..]);
            return result;
        }
        content.to_string()
    } else if let Some(close) = content.rfind("</Settings>") {
        let mut result = String::with_capacity(content.len() + 128);
        result.push_str(&content[..close]);
        result.push_str(&format!(
            "\t<Section path=\"{section_path}\">\n\t\t{attributes}\n\t</Section>\n"
        ));
        result.push_str(&content[close..]);
        result
    } else {
        content.to_string()
    }
}

fn settings_template(name: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Settings xmlns:x=\"https://www.presonus.software/xml/ccl\" name=\"{name}\" version=\"1\">\n</Settings>\n"
    )
}

/// 設定ファイルを更新する(なければテンプレートから作成、あればバックアップしてから)。
fn update_settings(
    path: &Path,
    template_name: &str,
    section_path: &str,
    attributes: &str,
    backups: &mut Vec<PathBuf>,
) -> Result<(), DawError> {
    let content = if path.is_file() {
        let original = std::fs::read_to_string(path).map_err(|e| io_err(path, e))?;
        // 書き換え前バックアップ(タイムスタンプ付き・上書きしない)
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = sora_core::fsutil::unique_path(
            &path.with_extension(format!("settings.sora-backup.{ts}")),
        );
        std::fs::write(&backup, &original).map_err(|e| io_err(&backup, e))?;
        backups.push(backup);
        original
    } else {
        settings_template(template_name)
    };
    let updated = upsert_section(&content, section_path, attributes);
    write_file(path, updated.as_bytes())
}

/// Services.settings のパスを解決する(アーキテクチャ別サブディレクトリ)。
fn services_settings_path(app_support: &Path) -> PathBuf {
    for arch in ["ARM64", "X64", "Win64", "x64"] {
        let candidate = app_support.join(arch).join("Services.settings");
        if candidate.is_file() {
            return candidate;
        }
    }
    // 既定(Apple Silicon)
    app_support.join("ARM64").join("Services.settings")
}

/// Bridge 拡張 + Sora Surface をインストール/更新する(冪等)。
pub fn install(paths: &StudioOnePaths) -> Result<SetupReport, DawError> {
    if !paths.app_support.is_dir() {
        return Err(DawError::NotConnected {
            adapter: "studio-one".to_string(),
            hint: format!(
                "Studio One 5 の設定ディレクトリが見つかりません: {}(Studio One 未インストールの場合は導入後に再実行。パスが違う場合は sora.config.json の daw.studio_one.app_support で指定)",
                paths.app_support.display()
            ),
        });
    }

    let mut installed = Vec::new();
    let mut settings_updated = Vec::new();
    let mut backups = Vec::new();

    // 1. Bridge 拡張ファイル
    let ext_dir = paths.app_support.join("Extensions").join(EXTENSION_ID);
    for (rel, content) in [
        ("metainfo.xml", EXT_METAINFO_XML.as_bytes().to_vec()),
        ("installdata.xml", INSTALLDATA_XML.as_bytes().to_vec()),
        ("scripts/sorabridge.package", build_package()?),
    ] {
        let dest = ext_dir.join(rel);
        write_file(&dest, &content)?;
        installed.push(dest);
    }

    // 2. Sora Surface デバイス定義(User Devices)
    let user_devices = paths.app_support.join("User Devices");
    for (name, content) in [
        ("Sora Surface.device", SURFACE_DEVICE_XML),
        ("Sora Surface.surface.xml", SURFACE_XML),
    ] {
        let dest = user_devices.join(name);
        write_file(&dest, content.as_bytes())?;
        installed.push(dest);
    }

    // 3. inbox / outbox / media
    let bridge = super::bridge::Bridge::new(&paths.user_content);
    for dir in [bridge.inbox(), bridge.outbox(), bridge.media()] {
        std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    }

    // 4. Extensions.settings で拡張を有効化
    let ext_settings = paths
        .app_support
        .join("Extensions")
        .join("Extensions.settings");
    update_settings(
        &ext_settings,
        "ExtensionManager",
        EXTENSION_ID,
        "<Attributes enabled=\"1\" uninstallPending=\"0\"/>",
        &mut backups,
    )?;
    settings_updated.push(ext_settings);

    // 5. Services.settings で Bridge サービスを有効化
    let services = services_settings_path(&paths.app_support);
    update_settings(
        &services,
        "Services",
        SERVICE_CLASS_ID,
        "<Attributes friendlyName=\"Sora Bridge\" enabled=\"1\"/>",
        &mut backups,
    )?;
    settings_updated.push(services);

    // 6. インストールレシート
    let receipt_path = bridge.receipt();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let receipt = serde_json::json!({
        "installed_at_unix": ts,
        "extension_id": EXTENSION_ID,
        "extension_dir": ext_dir,
        "surface_device": user_devices.join("Sora Surface.device"),
        "inbox": bridge.inbox(),
        "outbox": bridge.outbox(),
        "media": bridge.media(),
        "installer": format!("sora {}", env!("CARGO_PKG_VERSION")),
    });
    let body = serde_json::to_string_pretty(&receipt).map_err(|e| DawError::Rejected {
        operation: "install".to_string(),
        reason: format!("failed to serialize receipt: {e}"),
    })? + "\n";
    write_file(&receipt_path, body.as_bytes())?;
    installed.push(receipt_path);

    Ok(SetupReport {
        installed,
        settings_updated,
        backups,
        user_steps: vec![
            "Studio One 5 を再起動してください(拡張とデバイス定義の読み込みに必要)".to_string(),
            "仮想 MIDI ポートを用意してください(macOS: Audio MIDI 設定 → IAC ドライバでポート「Sora Trigger」を追加 / Windows: loopMIDI)".to_string(),
            "Studio One の 環境設定 > 外部デバイス に「Sora | Sora Surface」を追加し、受信元に上記ポートを割り当ててください".to_string(),
            "sora.config.json の daw.studio_one.trigger_port にポート名を設定してください".to_string(),
            "動作確認: `sora daw transport stop` で Studio One のトランスポートが反応するか確認(§11.2.1 の要検証項目)".to_string(),
        ],
        requires_restart: true,
    })
}

/// インストール状態を診断する。
pub fn check(paths: &StudioOnePaths) -> CheckReport {
    let ext_dir = paths.app_support.join("Extensions").join(EXTENSION_ID);
    let bridge = super::bridge::Bridge::new(&paths.user_content);
    let ext_settings = paths
        .app_support
        .join("Extensions")
        .join("Extensions.settings");
    let user_devices = paths.app_support.join("User Devices");

    let settings_enabled = std::fs::read_to_string(&ext_settings)
        .map(|s| {
            s.find(&format!("<Section path=\"{EXTENSION_ID}\">"))
                .map(|i| s[i..].find("enabled=\"1\"").is_some())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let items = vec![
        CheckItem {
            name: "app_support".to_string(),
            ok: paths.app_support.is_dir(),
            detail: paths.app_support.display().to_string(),
        },
        CheckItem {
            name: "bridge_extension".to_string(),
            ok: ext_dir.join("metainfo.xml").is_file()
                && ext_dir.join("scripts/sorabridge.package").is_file(),
            detail: ext_dir.display().to_string(),
        },
        CheckItem {
            name: "bridge_enabled_in_settings".to_string(),
            ok: settings_enabled,
            detail: ext_settings.display().to_string(),
        },
        CheckItem {
            name: "sora_surface_device".to_string(),
            ok: user_devices.join("Sora Surface.device").is_file()
                && user_devices.join("Sora Surface.surface.xml").is_file(),
            detail: user_devices.display().to_string(),
        },
        CheckItem {
            name: "bridge_dirs".to_string(),
            ok: bridge.inbox().is_dir() && bridge.outbox().is_dir(),
            detail: bridge.inbox().display().to_string(),
        },
        CheckItem {
            name: "inbox_empty".to_string(),
            ok: bridge.pending_requests().is_empty(),
            detail: format!("{} 件の未処理リクエスト", bridge.pending_requests().len()),
        },
    ];
    CheckReport {
        ok: items.iter().all(|i| i.ok),
        items,
    }
}

/// 無効化 + 退避(削除はしない)。
pub fn uninstall(paths: &StudioOnePaths) -> Result<SetupReport, DawError> {
    let mut settings_updated = Vec::new();
    let mut backups = Vec::new();
    let mut installed = Vec::new();

    let ext_settings = paths
        .app_support
        .join("Extensions")
        .join("Extensions.settings");
    if ext_settings.is_file() {
        update_settings(
            &ext_settings,
            "ExtensionManager",
            EXTENSION_ID,
            "<Attributes enabled=\"0\" uninstallPending=\"0\"/>",
            &mut backups,
        )?;
        settings_updated.push(ext_settings);
    }
    let services = services_settings_path(&paths.app_support);
    if services.is_file() {
        update_settings(
            &services,
            "Services",
            SERVICE_CLASS_ID,
            "<Attributes friendlyName=\"Sora Bridge\" enabled=\"0\"/>",
            &mut backups,
        )?;
        settings_updated.push(services);
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ext_dir = paths.app_support.join("Extensions").join(EXTENSION_ID);
    if ext_dir.is_dir() {
        let aside = sora_core::fsutil::unique_path(&PathBuf::from(format!(
            "{}.disabled.{ts}",
            ext_dir.display()
        )));
        std::fs::rename(&ext_dir, &aside).map_err(|e| io_err(&ext_dir, e))?;
        installed.push(aside);
    }
    let user_devices = paths.app_support.join("User Devices");
    for name in ["Sora Surface.device", "Sora Surface.surface.xml"] {
        let file = user_devices.join(name);
        if file.is_file() {
            let aside = sora_core::fsutil::unique_path(&PathBuf::from(format!(
                "{}.disabled.{ts}",
                file.display()
            )));
            std::fs::rename(&file, &aside).map_err(|e| io_err(&file, e))?;
            installed.push(aside);
        }
    }

    Ok(SetupReport {
        installed,
        settings_updated,
        backups,
        user_steps: vec![
            "Studio One 5 を再起動すると無効化が反映されます".to_string(),
            "SoraBridge のデータ(inbox/outbox/media)は残しています。不要なら手動で削除してください"
                .to_string(),
        ],
        requires_restart: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths() -> (tempfile::TempDir, StudioOnePaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = StudioOnePaths {
            user_content: dir.path().join("Documents/Studio One"),
            app_support: dir.path().join("AppSupport/Studio One 5"),
        };
        std::fs::create_dir_all(&paths.app_support).unwrap();
        (dir, paths)
    }

    #[test]
    fn upsert_inserts_then_updates() {
        let base = settings_template("ExtensionManager");
        let inserted = upsert_section(&base, EXTENSION_ID, "<Attributes enabled=\"1\"/>");
        assert!(inserted.contains("sora.studioone.bridge"));
        assert!(inserted.contains("enabled=\"1\""));

        let updated = upsert_section(&inserted, EXTENSION_ID, "<Attributes enabled=\"0\"/>");
        assert!(updated.contains("enabled=\"0\""));
        assert!(!updated.contains("enabled=\"1\""));
        // セクションは重複しない
        assert_eq!(updated.matches("sora.studioone.bridge").count(), 1);
    }

    #[test]
    fn install_is_idempotent_and_check_passes() {
        let (_dir, paths) = temp_paths();
        let first = install(&paths).unwrap();
        assert!(first.requires_restart);
        assert!(check(&paths).ok);

        // 再実行しても成功し、状態は健全なまま(冪等)
        let second = install(&paths).unwrap();
        assert!(!second.installed.is_empty());
        assert!(check(&paths).ok);

        // package が有効な ZIP で 3 ファイルを含む
        let pkg = paths
            .app_support
            .join("Extensions")
            .join(EXTENSION_ID)
            .join("scripts/sorabridge.package");
        let mut archive = zip::ZipArchive::new(std::fs::File::open(pkg).unwrap()).unwrap();
        assert_eq!(archive.len(), 3);
        assert!(archive.by_name("service.js").is_ok());
    }

    #[test]
    fn uninstall_disables_and_moves_aside() {
        let (_dir, paths) = temp_paths();
        install(&paths).unwrap();
        let report = uninstall(&paths).unwrap();
        assert!(!check(&paths).ok);
        assert!(
            report
                .installed
                .iter()
                .any(|p| p.to_string_lossy().contains(".disabled."))
        );
        let settings =
            std::fs::read_to_string(paths.app_support.join("Extensions/Extensions.settings"))
                .unwrap();
        assert!(settings.contains("enabled=\"0\""));
    }

    #[test]
    fn install_requires_studio_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StudioOnePaths {
            user_content: dir.path().join("uc"),
            app_support: dir.path().join("missing"),
        };
        let err = install(&paths).unwrap_err();
        assert_eq!(err.code(), "DAW_NOT_CONNECTED");
    }
}
