//! Studio One 5 アダプタ(技術要件書 §11.2.1)。
//!
//! 2 部品構成:
//! - **Sora Bridge 拡張(EditTask)**: `~/Documents/Studio One/SoraBridge/inbox/` の
//!   JSON リクエストを Studio One 内部で実行する(構造編集。実機検証済み)
//! - **Sora Surface(仮想 MIDI トリガー式コントロールサーフェス)**: 特定ノートを
//!   受けて `interpretCommand` を発火する純 XML デバイス(トランスポート +
//!   Bridge トリガー。OS 非依存。結合は要実機検証)
//!
//! `.song` はホットリロード不可(検証済み)のため、read は「最後に保存された
//! 状態」のオフライン読解になる。

pub mod bridge;
pub mod setup;
pub mod song;
pub mod trigger;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sora_core::model::SoraConfig;

use crate::adapter::{DawAdapter, not_supported};
use crate::error::DawError;
use crate::types::{
    DawCapabilities, DawProjectState, RenderReceipt, RenderRequest, TransportCmd, TransportState,
    WriteClipRequest, WriteReceipt, WriteStatus,
};

const ADAPTER: &str = "studio-one";

/// Sora Surface のノートマップ(assets/studio-one/Sora Surface.surface.xml と一致させる)。
/// ノート番号はチャンネル 1(status 0x90)。
pub mod surface_notes {
    /// Bridge inbox 処理: Sora Bridge サービスコマンド経由
    pub const APPLY_VIA_SERVICE: u8 = 0x14;
    /// Bridge inbox 処理: EditTask コマンド経由(カテゴリ TrackEdit)
    pub const APPLY_VIA_TASK: u8 = 0x15;
    /// Bridge inbox 処理: EditTask コマンド経由(カテゴリ Track)
    pub const APPLY_VIA_TRACK: u8 = 0x16;
    /// Transport: Start
    pub const PLAY: u8 = 0x18;
    /// Transport: Stop
    pub const STOP: u8 = 0x19;
    /// Transport: Record
    pub const RECORD: u8 = 0x1A;
    /// Transport: Rewind
    pub const REWIND: u8 = 0x1B;
    /// Transport: Forward
    pub const FORWARD: u8 = 0x1C;
    /// Transport: Return to Zero
    pub const RETURN_TO_ZERO: u8 = 0x1D;
}

/// Bridge inbox 処理を試す 3 経路(すべて送る。空 inbox への発火は無害)。
pub const APPLY_NOTE_SEQUENCE: [u8; 3] = [
    surface_notes::APPLY_VIA_SERVICE,
    surface_notes::APPLY_VIA_TASK,
    surface_notes::APPLY_VIA_TRACK,
];

fn transport_note(cmd: TransportCmd) -> u8 {
    match cmd {
        TransportCmd::Play => surface_notes::PLAY,
        TransportCmd::Stop => surface_notes::STOP,
        TransportCmd::Record => surface_notes::RECORD,
        TransportCmd::Rewind => surface_notes::REWIND,
        TransportCmd::Forward => surface_notes::FORWARD,
        TransportCmd::ReturnToZero => surface_notes::RETURN_TO_ZERO,
    }
}

/// Studio One 関連の解決済みパス。
#[derive(Debug, Clone)]
pub struct StudioOnePaths {
    /// ユーザーコンテンツ(既定: ~/Documents/Studio One)
    pub user_content: PathBuf,
    /// アプリ設定(既定: ~/Library/Application Support/PreSonus/Studio One 5)
    pub app_support: PathBuf,
}

impl StudioOnePaths {
    /// config(daw.studio_one)と OS 既定から解決する。
    pub fn resolve(config: Option<&SoraConfig>) -> Self {
        let settings = config
            .and_then(|c| c.daw.as_ref())
            .and_then(|d| d.studio_one.as_ref());
        // ホームディレクトリが取れない環境(CI 等)ではカレント相対に倒す
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let user_content = settings
            .and_then(|s| s.user_content.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Documents").join("Studio One"));
        let app_support = settings
            .and_then(|s| s.app_support.as_ref())
            .map(PathBuf::from)
            .unwrap_or_else(|| default_app_support(&home));
        Self {
            user_content,
            app_support,
        }
    }
}

fn default_app_support(home: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        // %APPDATA%\PreSonus\Studio One 5
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("PreSonus")
            .join("Studio One 5")
    } else {
        home.join("Library")
            .join("Application Support")
            .join("PreSonus")
            .join("Studio One 5")
    }
}

/// Studio One 5 アダプタ。
pub struct StudioOneAdapter {
    root: PathBuf,
    paths: StudioOnePaths,
    song_path: Option<PathBuf>,
    trigger_port: Option<String>,
    bridge_timeout: Duration,
}

impl StudioOneAdapter {
    pub fn new(root: &Path, config: Option<&SoraConfig>) -> Self {
        let settings = config
            .and_then(|c| c.daw.as_ref())
            .and_then(|d| d.studio_one.as_ref());
        Self {
            root: root.to_path_buf(),
            paths: StudioOnePaths::resolve(config),
            song_path: settings
                .and_then(|s| s.song_path.as_ref())
                .map(PathBuf::from),
            trigger_port: settings.and_then(|s| s.trigger_port.clone()),
            bridge_timeout: Duration::from_millis(
                settings.and_then(|s| s.bridge_timeout_ms).unwrap_or(10_000),
            ),
        }
    }

    fn bridge(&self) -> bridge::Bridge {
        bridge::Bridge::new(&self.paths.user_content)
    }

    /// トリガーポート名(未設定はセットアップ導線へ誘導するエラー)。
    fn trigger_port(&self) -> Result<&str, DawError> {
        self.trigger_port.as_deref().ok_or_else(|| DawError::NotConnected {
            adapter: ADAPTER.to_string(),
            hint: "sora.config.json の daw.studio_one.trigger_port に Sora Surface 用の仮想 MIDI ポート名を設定してください(`sora daw setup studio-one` と `sora doctor` がセットアップ手順を案内します)".to_string(),
        })
    }

    /// 書き込み前スナップショット(§11.4)。保存済み .song を versions/daw-backups/ へコピーする。
    /// バックアップ元が用意できない場合は書き込みを拒否する。
    fn backup_song(&self) -> Result<PathBuf, DawError> {
        let song = self.song_path.as_ref().ok_or_else(|| {
            DawError::BackupUnavailable {
                reason: "daw.studio_one.song_path が未設定のため、書き込み前スナップショットを取得できません".to_string(),
            }
        })?;
        if !song.is_file() {
            return Err(DawError::BackupUnavailable {
                reason: format!("song_path {} が存在しません", song.display()),
            });
        }
        let stem = song
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "song".to_string());
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = sora_core::fsutil::unique_path(
            &self
                .root
                .join("versions")
                .join("daw-backups")
                .join(format!("{stem}.{ts}.song")),
        );
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DawError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        std::fs::copy(song, &dest).map_err(|e| DawError::Io {
            path: song.clone(),
            source: e,
        })?;
        Ok(dest)
    }

    /// Bridge inbox の処理をトリガーし、消化を待つ。
    /// トリガーポート未設定・タイムアウト時は false(手動トリガー待ち)。
    fn trigger_and_wait(&self, queued: &Path) -> bool {
        let Ok(port) = self.trigger_port() else {
            return false;
        };
        if trigger::send_notes(port, &APPLY_NOTE_SEQUENCE).is_err() {
            return false;
        }
        self.bridge().wait_consumed(queued, self.bridge_timeout)
    }
}

impl DawAdapter for StudioOneAdapter {
    fn name(&self) -> &'static str {
        ADAPTER
    }

    fn capabilities(&self) -> DawCapabilities {
        let installed = self.bridge().is_installed(&self.paths.app_support);
        DawCapabilities {
            adapter: ADAPTER.to_string(),
            read: true,
            transport: true,
            write_clip: true,
            write_automation: false,
            render: false,
            selection: false,
            notes: vec![
                format!(
                    "Sora Bridge 拡張: {}(`sora daw setup studio-one` で導入)",
                    if installed {
                        "導入済み"
                    } else {
                        "未導入"
                    }
                ),
                "read は最後に保存された .song のオフライン読解(ライブ状態ではない)".to_string(),
                "transport / Bridge トリガー(Sora Surface)は要実機検証(§11.2.1)".to_string(),
                "selection 非対応: 範囲指定は Note Selector(bars/section/pitch)で行う(§11.3)"
                    .to_string(),
            ],
        }
    }

    fn read_project(&mut self) -> Result<DawProjectState, DawError> {
        let song = self.song_path.as_ref().ok_or_else(|| DawError::NotConnected {
            adapter: ADAPTER.to_string(),
            hint: "sora.config.json の daw.studio_one.song_path に対象 .song のパスを設定してください".to_string(),
        })?;
        let mut state = song::read_song(song)?;
        state.notes.push(
            "最後に保存された時点の状態です(Studio One はディスク上の変更をホットリロードしません)"
                .to_string(),
        );
        Ok(state)
    }

    fn transport(&mut self, cmd: TransportCmd) -> Result<TransportState, DawError> {
        let port = self.trigger_port()?;
        let note = transport_note(cmd);
        trigger::send_notes(port, &[note]).map_err(|e| DawError::NotConnected {
            adapter: ADAPTER.to_string(),
            hint: format!(
                "仮想 MIDI ポート `{port}` へ送信できません: {e}。`sora doctor` でポートを確認してください"
            ),
        })?;
        Ok(TransportState {
            requested: cmd,
            delivered: true,
            verified: false,
            route: format!("Sora Surface note 0x{note:02X} via `{port}`"),
        })
    }

    fn write_clip(&mut self, req: WriteClipRequest) -> Result<WriteReceipt, DawError> {
        // 1. 書き込み前スナップショット(§11.4。取れなければ拒否)
        let backup = self.backup_song()?;

        // 2. MIDI を Bridge の media 領域へ配置(Studio One から見えるパス)
        let bridge = self.bridge();
        let media_path = bridge.stage_media(&req.midi_file)?;

        // 3. import_file リクエストを inbox へキュー
        let request = serde_json::json!({
            "type": "import_file",
            "path": media_path,
        });
        let queued = bridge.queue_request(&request, "import-midi")?;

        // 4. Sora Surface トリガー → 消化待ち
        let consumed = self.trigger_and_wait(&queued);

        let track = req
            .track_hint
            .as_deref()
            .unwrap_or("新規トラック(Import File が作成)");
        let mut notes = vec![
            "Import File はパス引数を受け取らないため、Studio One がファイル選択ダイアログを開いた場合は media パスを選択してください(検証済みの制約)".to_string(),
        ];
        if !consumed {
            notes.push(
                "自動トリガーが確認できませんでした。Studio One のメニュー「トラック > Apply Sora Bridge Inbox」を実行してください"
                    .to_string(),
            );
        }
        Ok(WriteReceipt {
            action: "write_clip".to_string(),
            adapter: ADAPTER.to_string(),
            status: if consumed {
                WriteStatus::Applied
            } else {
                WriteStatus::Queued
            },
            target: format!("Studio One 現行ソング / {track}"),
            files: vec![media_path, queued],
            backup: Some(backup.clone()),
            undo: vec![
                "Studio One で 編集 > 取り消し(Cmd+Z)".to_string(),
                format!(
                    "復元が必要な場合: バックアップ {} を開き直す",
                    backup.display()
                ),
            ],
            notes,
        })
    }

    fn write_automation(
        &mut self,
        _plan: &sora_core::model::AutomationPlan,
    ) -> Result<WriteReceipt, DawError> {
        Err(not_supported(
            ADAPTER,
            "write_automation",
            "generic アダプタが手動適用手順書を生成できます(`sora automation apply --adapter generic`)。マップ済みパラメータの MIDI CC 経路は要検証のため未実装(§11.2.1)",
        ))
    }

    fn render(&mut self, _req: RenderRequest) -> Result<RenderReceipt, DawError> {
        Err(not_supported(
            ADAPTER,
            "render",
            "Studio One 上で手動レンダリング(Song > Export Mixdown)し、結果を analyze_audio に渡してください(render 経路は未検証、§11.2.1)",
        ))
    }
}
