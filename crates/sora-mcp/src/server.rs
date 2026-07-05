//! `sora mcp serve` — stdio で MCP ツールを公開する(技術要件書 §8)。
//!
//! 公開ツールと要求 control level:
//!
//! | ツール | 実体 | 破壊性 | level |
//! |---|---|---|---|
//! | analyze_project | project-context + 解析成果物の集約読み取り | read-only | 0 |
//! | read_midi | midi inspect / analyze | read-only | 0 |
//! | compose_part | Part Plan 検証 + midi compile | 新規ファイルのみ | 1 |
//! | apply_articulations | midi decompile → 注釈 → compile | 新規ファイルのみ | 1 |
//! | export_midi | コンパイル済み .mid の配置 | 新規ファイルのみ | 1 |
//! | suggest_plugin_settings | Profile 参照 + settings.json 生成 | 提案のみ | 1 |
//! | analyze_audio | audio analyze / compare | read-only | 1 |
//! | send_midi | midi send | 実時間送信 | 2 |
//!
//! DAW 統合系ツール(read_daw_project / write_clip 等、level 3+)は
//! Milestone 5 の sora-daw アダプタ実装後に追加する。

use std::path::{Path, PathBuf};

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use sora_core::error::{CoreError, ValidationIssue};
use sora_core::midi;
use sora_core::model::PartPlan;
use sora_core::validate::parse_validated;

use crate::{gate, ops, report};

/// Sora MCP サーバー。プロジェクトルートを基準に動作する。
#[derive(Clone)]
pub struct SoraMcp {
    root: PathBuf,
    tool_router: ToolRouter<Self>,
}

/// stdio で MCP サーバーを起動し、クライアント切断まで待つ。
/// stdout は MCP トランスポート専用(診断は stderr / tracing へ)。
pub async fn serve_stdio(root: PathBuf) -> anyhow::Result<()> {
    let service = SoraMcp::new(root)
        .serve(stdio())
        .await
        .map_err(|e| anyhow::anyhow!("MCP server initialization failed: {e}"))?;
    service
        .waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP server terminated abnormally: {e}"))?;
    Ok(())
}

impl SoraMcp {
    /// `root` をプロジェクトルートとするサーバーを作る。
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            tool_router: Self::tool_router(),
        }
    }

    fn config_path(&self) -> PathBuf {
        self.root.join("sora.config.json")
    }

    /// 相対パスをプロジェクトルート基準で解決する。
    fn resolve(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            self.root.join(p)
        }
    }

    /// ゲート検査 → 実行 → actions.jsonl 記録 → CallToolResult 正規化(§8 横断要件)。
    fn run_tool(
        &self,
        tool: &str,
        required_level: u8,
        args_summary: Value,
        f: impl FnOnce() -> anyhow::Result<Value>,
    ) -> CallToolResult {
        if let Err(rejection) = gate::check(&self.root, tool, required_level) {
            ops::record_action(
                &self.root,
                tool,
                json!({ "args": args_summary, "rejected": rejection.code }),
            );
            return CallToolResult::structured_error(json!({ "error": rejection }));
        }
        match f() {
            Ok(value) => {
                ops::record_action(
                    &self.root,
                    tool,
                    json!({ "args": args_summary, "result": "ok" }),
                );
                CallToolResult::structured(value)
            }
            Err(err) => {
                let (report, _exit) = report::normalize(&err);
                ops::record_action(
                    &self.root,
                    tool,
                    json!({ "args": args_summary, "error": report.code }),
                );
                CallToolResult::structured_error(json!({ "error": report }))
            }
        }
    }
}

/// read_midi の動作モード。
#[derive(Debug, Default, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReadMidiMode {
    /// ノート/テンポ/CC のダンプと統計
    #[default]
    Inspect,
    /// BPM・調性中心・リズム統計の推定
    Analyze,
}

/// read_midi のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadMidiParams {
    /// SMF ファイルのパス(プロジェクトルート相対または絶対)
    pub file: String,
    /// 動作モード(既定: inspect)
    #[serde(default)]
    pub mode: ReadMidiMode,
    /// inspect 時に全ノートをダンプへ含める
    #[serde(default)]
    pub include_notes: bool,
}

/// compose_part のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComposePartParams {
    /// Part Plan(schemas/part-plan.schema.json に従う JSON オブジェクト)
    pub plan: Value,
    /// Device Profile のパス(省略時は sora.config.json の devices から解決)
    #[serde(default)]
    pub profile_path: Option<String>,
    /// 出力 .mid のパス(省略時 exports/<part_id>.mid。既存パスへは書かない)
    #[serde(default)]
    pub out: Option<String>,
}

/// apply_articulations の 1 編集。bars か note_indices のどちらかで対象を指定する。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ArticulationEdit {
    /// 奏法 ID(Device Profile の keyswitches の articulation)
    pub articulation: String,
    /// 対象小節範囲 [start, end](1 始まり・両端含む)
    #[serde(default)]
    pub bars: Option<[u32; 2]>,
    /// 対象ノートの通し番号(decompile 結果の時間順 0 始まり)
    #[serde(default)]
    pub note_indices: Option<Vec<usize>>,
}

/// apply_articulations のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyArticulationsParams {
    /// 対象 SMF ファイルのパス
    pub file: String,
    /// 対象デバイス ID(Device Profile の解決に使う)
    pub device: String,
    /// 適用する奏法編集
    pub edits: Vec<ArticulationEdit>,
    /// 生成 Plan の part_id(省略時 `<ファイル名>-articulated`)
    #[serde(default)]
    pub part_id: Option<String>,
    /// 出力 .mid のパス(省略時 exports/<part_id>.mid)
    #[serde(default)]
    pub out: Option<String>,
}

/// export_midi のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportMidiParams {
    /// コピー元 .mid のパス
    pub source: String,
    /// 配置先パス(既存パスへは書かない)
    pub dest: String,
}

/// suggest_plugin_settings の 1 パラメータ提案。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SettingSuggestion {
    /// パラメータ名(Device Profile の parameters[].name)
    pub parameter: String,
    /// 提案値
    pub value: f64,
    /// このパラメータ設定の音楽的理由
    #[serde(default)]
    pub note: Option<String>,
}

/// suggest_plugin_settings のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestPluginSettingsParams {
    /// 対象デバイス ID
    pub device: String,
    /// 提案名(出力ファイル名の基底。例: "warm-clean-master")
    pub name: String,
    /// パラメータ提案の列
    pub settings: Vec<SettingSuggestion>,
    /// 提案全体の音楽的意図
    #[serde(default)]
    pub notes: Option<String>,
}

/// analyze_audio のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeAudioParams {
    /// オーディオファイルのパス(WAV/AIFF/FLAC/MP3 等)
    pub file: String,
    /// 指定時は A/B 比較になる(file=A, compare_with=B。delta は B - A)
    #[serde(default)]
    pub compare_with: Option<String>,
}

/// send_midi のパラメータ。
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMidiParams {
    /// 送信する SMF ファイルのパス
    pub file: String,
    /// 出力ポート名(部分一致。省略時は sora.config.json の midi.port_name)
    #[serde(default)]
    pub port: Option<String>,
}

#[tool_router]
impl SoraMcp {
    /// project-context・config・成果物一覧を集約して返す(level 0)。
    #[tool(
        name = "analyze_project",
        description = "プロジェクトの現状を集約して返す: sora.config.json(control level 含む)、project-context.json、exports/analysis/devices の成果物一覧。read-only。"
    )]
    pub async fn analyze_project(&self) -> CallToolResult {
        self.run_tool("analyze_project", 0, json!({}), || {
            let read_json = |path: &Path| -> Value {
                std::fs::read_to_string(path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(Value::Null)
            };
            let list_dir = |name: &str| -> Vec<String> {
                std::fs::read_dir(self.root.join(name))
                    .map(|entries| {
                        let mut files: Vec<String> = entries
                            .filter_map(|e| e.ok())
                            .filter(|e| e.path().is_file())
                            .map(|e| format!("{name}/{}", e.file_name().to_string_lossy()))
                            .collect();
                        files.sort();
                        files
                    })
                    .unwrap_or_default()
            };
            Ok(json!({
                "root": self.root,
                "config": read_json(&self.config_path()),
                "project_context": read_json(&self.root.join("project-context.json")),
                "exports": list_dir("exports"),
                "analysis": list_dir("analysis"),
                "devices": list_dir("devices"),
                "versions": list_dir("versions"),
            }))
        })
    }

    /// SMF の読み取り(inspect / analyze。level 0)。
    #[tool(
        name = "read_midi",
        description = "SMF を読み取る。mode=inspect はノート/テンポ/CC ダンプ + 統計、mode=analyze は BPM・調性中心・リズム統計の推定。read-only。"
    )]
    pub async fn read_midi(&self, params: Parameters<ReadMidiParams>) -> CallToolResult {
        let p = params.0;
        let file = self.resolve(&p.file);
        let args = json!({ "file": p.file, "mode": format!("{:?}", p.mode) });
        self.run_tool("read_midi", 0, args, || match p.mode {
            ReadMidiMode::Inspect => {
                let inspection = midi::inspect_file(&file, p.include_notes)?;
                Ok(serde_json::to_value(inspection)?)
            }
            ReadMidiMode::Analyze => {
                let analysis = midi::analyze_file(&file)?;
                Ok(serde_json::to_value(analysis)?)
            }
        })
    }

    /// Part Plan を検証して .mid へコンパイルする(level 1)。
    #[tool(
        name = "compose_part",
        description = "Part Plan(JSON)を 3 層検証し、Device Profile を解決して .mid へコンパイルする。Plan と .mid を新規ファイルとして保存(既存パスへは書かない)。エラー時は ErrorReport(code/pointer/hint)を返すので Plan を修正して再実行する。"
    )]
    pub async fn compose_part(&self, params: Parameters<ComposePartParams>) -> CallToolResult {
        let p = params.0;
        let args = json!({
            "part_id": p.plan.get("part_id").cloned().unwrap_or(Value::Null),
            "device": p.plan.get("device").cloned().unwrap_or(Value::Null),
        });
        self.run_tool("compose_part", 1, args, || {
            let plan: PartPlan = parse_validated(&p.plan)?;
            let profile = ops::resolve_profile(
                &plan.device,
                p.profile_path.as_ref().map(|s| self.resolve(s)).as_deref(),
                Some(&self.config_path()),
            )?;
            let output = midi::compile(&plan, &profile)?;

            let out_path = p.out.as_ref().map(|s| self.resolve(s)).unwrap_or_else(|| {
                self.root
                    .join("exports")
                    .join(format!("{}.mid", plan.part_id))
            });
            ops::write_new_file(&out_path, &output.bytes)?;

            // Plan も成果物として保存(レビュー・diff・バージョン管理の単位)
            let plan_path = out_path.with_extension("plan.json");
            let plan_json = serde_json::to_string_pretty(&plan)? + "\n";
            ops::write_new_file(&plan_path, plan_json.as_bytes())?;

            Ok(json!({
                "output": out_path,
                "plan_path": plan_path,
                "report": output.report,
            }))
        })
    }

    /// 既存 SMF に奏法注釈を付けて再コンパイルする(level 1)。
    #[tool(
        name = "apply_articulations",
        description = "既存 SMF を Part Plan へ逆コンパイルし、指定範囲のノートへ奏法(articulation)を付与して新しい .mid へ再コンパイルする。元ファイルは変更しない。"
    )]
    pub async fn apply_articulations(
        &self,
        params: Parameters<ApplyArticulationsParams>,
    ) -> CallToolResult {
        let p = params.0;
        let file = self.resolve(&p.file);
        let args = json!({ "file": p.file, "device": p.device, "edits": p.edits.len() });
        self.run_tool("apply_articulations", 1, args, || {
            let profile = ops::resolve_profile(&p.device, None, Some(&self.config_path()))?;
            let part_id = p.part_id.clone().unwrap_or_else(|| {
                let stem = file
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "decompiled".to_string());
                format!("{stem}-articulated")
            });
            let decompiled = midi::decompile_file(&file, &profile, &part_id)?;
            let mut plan: PartPlan = parse_validated(&decompiled.plan)?;
            let edited = apply_edits(&mut plan, &p.edits)?;

            let output = midi::compile(&plan, &profile)?;
            let out_path = p
                .out
                .as_ref()
                .map(|s| self.resolve(s))
                .unwrap_or_else(|| self.root.join("exports").join(format!("{part_id}.mid")));
            ops::write_new_file(&out_path, &output.bytes)?;
            let plan_path = out_path.with_extension("plan.json");
            let plan_json = serde_json::to_string_pretty(&plan)? + "\n";
            ops::write_new_file(&plan_path, plan_json.as_bytes())?;

            Ok(json!({
                "output": out_path,
                "plan_path": plan_path,
                "edited_notes": edited,
                "report": output.report,
            }))
        })
    }

    /// コンパイル済み .mid を配置する(level 1)。
    #[tool(
        name = "export_midi",
        description = "コンパイル済み .mid を指定先へコピーして配置する。既存パスへは書かない(非破壊)。"
    )]
    pub async fn export_midi(&self, params: Parameters<ExportMidiParams>) -> CallToolResult {
        let p = params.0;
        let args = json!({ "source": p.source, "dest": p.dest });
        self.run_tool("export_midi", 1, args, || {
            let source = self.resolve(&p.source);
            let dest = self.resolve(&p.dest);
            let bytes = std::fs::read(&source).map_err(|e| CoreError::Io {
                path: source.clone(),
                source: e,
            })?;
            ops::write_new_file(&dest, &bytes)?;
            Ok(json!({ "source": source, "dest": dest, "bytes": bytes.len() }))
        })
    }

    /// プラグイン設定の提案を settings.json として生成する(level 1、提案のみ)。
    #[tool(
        name = "suggest_plugin_settings",
        description = "Device Profile の parameters と照合してプラグイン設定案を検証し、settings.json として保存する(提案のみ。実機には作用しない)。safe_range 逸脱はエラー、confidence=unverified のパラメータは警告になる。"
    )]
    pub async fn suggest_plugin_settings(
        &self,
        params: Parameters<SuggestPluginSettingsParams>,
    ) -> CallToolResult {
        let p = params.0;
        let args = json!({ "device": p.device, "name": p.name, "settings": p.settings.len() });
        self.run_tool("suggest_plugin_settings", 1, args, || {
            let profile =
                ops::resolve_profile(&p.device, None, Some(&self.config_path()))?;

            let mut issues: Vec<ValidationIssue> = Vec::new();
            let mut warnings: Vec<String> = Vec::new();
            for (i, s) in p.settings.iter().enumerate() {
                let Some(param) = profile.parameters.iter().find(|q| q.name == s.parameter)
                else {
                    let available: Vec<&str> =
                        profile.parameters.iter().map(|q| q.name.as_str()).collect();
                    issues.push(ValidationIssue {
                        pointer: format!("/settings/{i}/parameter"),
                        code: "UNKNOWN_PARAMETER".to_string(),
                        message: format!(
                            "parameter `{}` is not defined in profile `{}`",
                            s.parameter, profile.id
                        ),
                        hint: Some(format!("利用可能: {}", available.join(", "))),
                    });
                    continue;
                };
                match param.safe_range {
                    Some([min, max]) if s.value < min || s.value > max => {
                        issues.push(ValidationIssue {
                            pointer: format!("/settings/{i}/value"),
                            code: "VALUE_OUT_OF_SAFE_RANGE".to_string(),
                            message: format!(
                                "value {} for `{}` is outside safe range [{min}, {max}]",
                                s.value, s.parameter
                            ),
                            hint: Some(
                                "safe_range 内に収めるか、根拠を design notes に記述した上でユーザーに確認してください"
                                    .to_string(),
                            ),
                        });
                    }
                    Some(_) => {}
                    None => warnings.push(format!(
                        "`{}` は safe_range 未定義です(値の妥当性は実機確認が必要)",
                        s.parameter
                    )),
                }
                if param.confidence == sora_core::model::Confidence::Unverified {
                    warnings.push(format!(
                        "`{}` は confidence: unverified です(実機確認を推奨)",
                        s.parameter
                    ));
                }
            }
            if !issues.is_empty() {
                return Err(CoreError::Validation { issues }.into());
            }

            let settings_doc = json!({
                "schema_version": "1.0",
                "device": p.device,
                "settings": p.settings.iter().map(|s| json!({
                    "parameter": s.parameter,
                    "value": s.value,
                    "note": s.note,
                })).collect::<Vec<_>>(),
                "notes": p.notes,
                "warnings": warnings,
            });
            let out_path = self
                .root
                .join("exports")
                .join(format!("{}.settings.json", p.name));
            let doc = serde_json::to_string_pretty(&settings_doc)? + "\n";
            ops::write_new_file(&out_path, doc.as_bytes())?;

            Ok(json!({ "output": out_path, "warnings": warnings }))
        })
    }

    /// オーディオ解析 / A/B 比較(level 1)。
    #[tool(
        name = "analyze_audio",
        description = "オーディオファイルのラウドネス(LUFS/LRA/true peak)・7 バンド帯域バランス・クレストファクタ・ステレオ相関を測定する。compare_with 指定時は 2 ファイルの差分(B - A)を返す。read-only。"
    )]
    pub async fn analyze_audio(&self, params: Parameters<AnalyzeAudioParams>) -> CallToolResult {
        let p = params.0;
        let file = self.resolve(&p.file);
        let args = json!({ "file": p.file, "compare_with": p.compare_with });
        self.run_tool("analyze_audio", 1, args, || match &p.compare_with {
            Some(b) => {
                let comparison = sora_audio::compare_files(&file, &self.resolve(b))?;
                Ok(serde_json::to_value(comparison)?)
            }
            None => {
                let analysis = sora_audio::analyze_file(&file)?;
                Ok(serde_json::to_value(analysis)?)
            }
        })
    }

    /// SMF を仮想 MIDI ポートへ実時間送信する(level 2)。
    #[tool(
        name = "send_midi",
        description = "SMF を仮想 MIDI ポート(IAC Driver / loopMIDI)へ実時間送信する。完了・中断・エラーのいずれでも全チャンネルへ All-Notes-Off を送る。control level 2 が必要。"
    )]
    pub async fn send_midi(
        &self,
        params: Parameters<SendMidiParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let args = json!({ "file": p.file, "port": p.port });

        // ゲートは送信スレッドを起こす前に検査する
        if let Err(rejection) = gate::check(&self.root, "send_midi", 2) {
            ops::record_action(
                &self.root,
                "send_midi",
                json!({ "args": args, "rejected": rejection.code }),
            );
            return Ok(CallToolResult::structured_error(
                json!({ "error": rejection }),
            ));
        }

        let file = self.resolve(&p.file);
        let port = match self.resolve_port(p.port.clone()) {
            Ok(port) => port,
            Err(err) => {
                let (report, _) = report::normalize(&err);
                ops::record_action(
                    &self.root,
                    "send_midi",
                    json!({ "args": args, "error": report.code }),
                );
                return Ok(CallToolResult::structured_error(json!({ "error": report })));
            }
        };

        // 実時間送信はブロッキング処理なので専用スレッドで実行する
        let result = tokio::task::spawn_blocking(move || ops::send_file(&file, &port, None))
            .await
            .map_err(|e| McpError::internal_error(format!("send task panicked: {e}"), None))?;

        match result {
            Ok(stats) => {
                ops::record_action(
                    &self.root,
                    "send_midi",
                    json!({ "args": args, "result": "ok", "messages_sent": stats.messages_sent }),
                );
                Ok(CallToolResult::structured(
                    serde_json::to_value(stats).map_err(|e| {
                        McpError::internal_error(format!("serializing stats: {e}"), None)
                    })?,
                ))
            }
            Err(err) => {
                let (report, _) = report::normalize(&err);
                ops::record_action(
                    &self.root,
                    "send_midi",
                    json!({ "args": args, "error": report.code }),
                );
                Ok(CallToolResult::structured_error(json!({ "error": report })))
            }
        }
    }
}

impl SoraMcp {
    /// 送信ポートを解決する(明示指定 > sora.config.json の midi.port_name)。
    fn resolve_port(&self, port: Option<String>) -> anyhow::Result<String> {
        if let Some(p) = port {
            return Ok(p);
        }
        let cfg: sora_core::model::SoraConfig =
            sora_core::validate::load_validated(&self.config_path())?;
        cfg.midi.map(|m| m.port_name).ok_or_else(|| {
            anyhow::anyhow!(
                "sora.config.json に midi.port_name がありません。port パラメータで指定してください"
            )
        })
    }
}

/// 奏法編集を Plan に適用し、編集したノート数を返す。
/// 対象指定(bars / note_indices)に合致するノートが無い編集はエラーにする
/// (黙って何もしないと Agent が成功と誤認するため)。
fn apply_edits(plan: &mut PartPlan, edits: &[ArticulationEdit]) -> anyhow::Result<usize> {
    let mut total = 0usize;
    for (ei, edit) in edits.iter().enumerate() {
        if edit.bars.is_none() && edit.note_indices.is_none() {
            anyhow::bail!("edits[{ei}]: bars か note_indices のどちらかで対象を指定してください");
        }
        let mut hit = 0usize;
        let mut index = 0usize;
        for section in &mut plan.sections {
            for phrase in &mut section.phrases {
                for note in &mut phrase.notes {
                    let by_index = edit
                        .note_indices
                        .as_ref()
                        .is_some_and(|xs| xs.contains(&index));
                    let by_bar = edit.bars.is_some_and(|[start, end]| {
                        note.start
                            .split('.')
                            .next()
                            .and_then(|b| b.parse::<u32>().ok())
                            .is_some_and(|bar| bar >= start && bar <= end)
                    });
                    if by_index || by_bar {
                        note.articulation = Some(edit.articulation.clone());
                        hit += 1;
                    }
                    index += 1;
                }
            }
        }
        if hit == 0 {
            anyhow::bail!(
                "edits[{ei}]: 対象ノートが見つかりません(bars={:?}, note_indices={:?}。ノート総数 {index})",
                edit.bars,
                edit.note_indices
            );
        }
        total += hit;
    }
    Ok(total)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SoraMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = rmcp::model::Implementation::new("sora", env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Sora — 音楽制作のための接続されたプロダクションレイヤー。\
             Part Plan(JSON)を起草して compose_part でコンパイルし、\
             エラー時は ErrorReport の code/pointer/hint を読んで Plan を修正して再実行する。\
             実環境に作用するツール(send_midi 等)は control level が不足すると拒否される。\
             level の引き上げはユーザーの明示的な依頼に基づき CLI(sora config set control-level)でのみ行える。"
                .to_string(),
        );
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sora_core::model::{PlanNote, PlanSection};

    fn plan_with_notes(starts: &[&str]) -> PartPlan {
        let notes: Vec<PlanNote> = starts
            .iter()
            .map(|s| PlanNote {
                pitch: Some(sora_core::types::NoteSpec::Name("E2".to_string())),
                kit_piece: None,
                start: (*s).to_string(),
                duration: "0.1.000".to_string(),
                velocity: 100,
                articulation: None,
            })
            .collect();
        PartPlan {
            schema_version: "1.0".to_string(),
            part_id: "test".to_string(),
            device: "dev".to_string(),
            bpm: 120.0,
            time_signature: "4/4".to_string(),
            ppq: 480,
            sections: vec![PlanSection {
                label: "A".to_string(),
                start_bar: 1,
                phrases: vec![sora_core::model::Phrase { label: None, notes }],
            }],
            humanize: None,
            controls: vec![],
            program_changes: vec![],
            design_notes: None,
        }
    }

    #[test]
    fn apply_edits_by_bar_range() {
        let mut plan = plan_with_notes(&["1.1.000", "1.3.000", "2.1.000", "3.1.000"]);
        let edits = vec![ArticulationEdit {
            articulation: "palm_mute".to_string(),
            bars: Some([1, 2]),
            note_indices: None,
        }];
        #[allow(clippy::unwrap_used)]
        let edited = apply_edits(&mut plan, &edits).unwrap();
        assert_eq!(edited, 3);
        let notes = &plan.sections[0].phrases[0].notes;
        assert_eq!(notes[0].articulation.as_deref(), Some("palm_mute"));
        assert_eq!(notes[2].articulation.as_deref(), Some("palm_mute"));
        assert_eq!(notes[3].articulation, None);
    }

    #[test]
    fn apply_edits_by_note_index() {
        let mut plan = plan_with_notes(&["1.1.000", "1.3.000"]);
        let edits = vec![ArticulationEdit {
            articulation: "pinch_harmonic".to_string(),
            bars: None,
            note_indices: Some(vec![1]),
        }];
        #[allow(clippy::unwrap_used)]
        let edited = apply_edits(&mut plan, &edits).unwrap();
        assert_eq!(edited, 1);
        let notes = &plan.sections[0].phrases[0].notes;
        assert_eq!(notes[0].articulation, None);
        assert_eq!(notes[1].articulation.as_deref(), Some("pinch_harmonic"));
    }

    #[test]
    fn apply_edits_rejects_no_target() {
        let mut plan = plan_with_notes(&["1.1.000"]);
        let edits = vec![ArticulationEdit {
            articulation: "palm_mute".to_string(),
            bars: None,
            note_indices: None,
        }];
        assert!(apply_edits(&mut plan, &edits).is_err());
    }

    #[test]
    fn apply_edits_rejects_no_match() {
        let mut plan = plan_with_notes(&["1.1.000"]);
        let edits = vec![ArticulationEdit {
            articulation: "palm_mute".to_string(),
            bars: Some([5, 6]),
            note_indices: None,
        }];
        assert!(apply_edits(&mut plan, &edits).is_err());
    }
}
