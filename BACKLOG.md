# Sora 実装バックログ

実装タスクの単一の管理台帳。**セッション再開時はまずこのファイルを読むこと。**
仕様の正: [docs/technical-requirements.md](docs/technical-requirements.md)(§番号は同書を指す)

## 運用ルール

- タスク完了ごとにチェックを付け、対応するコミットを作って push する
- 中断・再開はこのファイルの「現在地」と git log で判断する
- 要決定事項が出たら「未決事項」へ追記し、ユーザーへ質問する(勝手に決めない)
- 仕様変更が必要になったら docs/ を先に直してから実装する(上流→下流の順)

## 現在地

**M1 完了(実機確認除く)/ M2・M3 コア完了。** GitHub 公開・CI green。
- M1: sora-core + sora-cli 全機能、golden テスト、example、CLAUDE.md ワークフロー ✅
- M2: decompile / drum_map / エフェクト Profile スキーマ ✅(残: missing_context、実デバイス Profile=要マニュアル)
- M3: sora-audio(loudness/spectrum/compare)、AutomationPlan スキーマ、genre-targets ✅(残: tone/master ワークフロー doc)
- 次の大物: **M4 MCP サーバー + 仮想 MIDI(midir)**。実装は自己完結だが実機ルーティング確認に IAC Driver 設定が要る。

**M4 実装完了(実機ルーティング確認除く)/ M5 実装完了(Studio One 実機検証除く)。** sora-daw クレート(DawAdapter + Generic + Studio One アダプタ)、`sora daw` / `sora automation` CLI、MCP の DAW ツール(level 3-5)、Note Selector まで実装・テスト済み。残りはユーザーの実機セットアップ(Sora Surface 有効化・トリガー結合検証)と M6。

## 実環境メモ(ユーザー PC・実態確認可能)

- DAW: **Studio One 5**(docs の "Fender Studio Pro" は仮名。M5 のアダプタ調査対象は Studio One)
- インストール済みプラグイン: Heavier7Strings / MODO BASS / MODO DRUM / AmpliTube 5 / Waves Gold 同梱 VST 各種 / Ozone 9
- M2 の Profile 作成・M1 の検証用 MIDI 実機確認はこの環境で行える

---

## Milestone 0: プロジェクト足場

- [x] git init / Cargo workspace(sora-core, sora-audio, sora-cli)
- [x] rust-toolchain.toml / .gitignore / BACKLOG.md
- [x] CLAUDE.md(ビルド・テスト・行動規範)
- [x] GitHub リポジトリ作成(dino3616/sora, public)+ 初回 push
- [x] CI(GitHub Actions: fmt / clippy / test / schema drift)

## Milestone 1: MVP コアループ(§15 M1)

sora-core:
- [x] newtype 群: `MidiNote`(octave_convention 対応パース), `Velocity`, `BarBeatTick`(§4.6 L3)
- [x] エラー型: `CoreError` / `ValidationIssue` / `ErrorReport` + 終了コード規約(§6)
- [x] モデル: `SoraConfig`(§4.1), `DeviceProfile`(§4.2), `ProjectContext`(§4.3, active_source 含む), `PartPlan`(§4.4)
- [x] スキーマ生成: schemars → JSON Schema 出力 + description 必須(§4.6)
- [x] 検証 3 層: jsonschema(L1)→ serde deny_unknown_fields(L2)→ ドメイン検証(L3)
- [x] MIDI コンパイラ: PartPlan + Profile → SMF Format 1(§7 の全規約: keyswitch lead/momentary/latch, 音域検査, 衝突検査, humanize seed 固定, モノフォニックトリム)
- [x] MIDI inspect: SMF → ノート/テンポ/CC ダンプ + 統計
- [x] MIDI analyze: BPM 推定・調性中心(Krumhansl-Schmuckler)・リズム統計
- [x] 検証用 MIDI 生成: Profile → 全奏法 1 音ずつの verify.mid(§16 リスク 1)
- [ ] drum_map コンパイル(kit_piece 解決、ch10 既定)※compile 側は実装済、CLI 未接続

sora-cli:
- [x] clap 骨格 + JSON 出力 + ErrorReport 出力 + 終了コード(§6.3)
- [x] `sora midi compile` / `sora midi inspect` / `sora midi analyze`
- [x] `sora profile validate` / `sora profile verify-midi`
- [x] `sora schema dump [--out] [--check]`
- [x] `sora config set control-level <0-5>`(確認表示 + actions.jsonl 記録、§2.4)
- [x] `sora init`(プロジェクト雛形生成、§14)
- [x] `sora version snapshot <label>`
- [x] `sora doctor`(環境診断)

テスト・検証:
- [x] golden file テスト(コンパイラのバイト同一性、SORA_BLESS で更新)
- [x] Heavier7Strings サンプル Profile(confidence: unverified、実機確認前提のテンプレート)
- [x] examples/: E2E デモ手順(examples/heavier7strings-riff/)
- [x] CLAUDE.md に Sora ワークフロー(Plan 起草 → compile → 説明)を記述
- [ ] 【要ユーザー】verify.mid を Studio One 5 + Heavier7Strings で実機確認 → confidence 昇格

**M1 は実機確認以外完了。** insta スナップショットは JSON レポート系で M2 以降に追加検討。

受け入れ(§15 M1): ベースライン → Plan → compile → 実 DAW で奏法発音 / ErrorReport のみで 1 往復自己修正 / v2 生成で v1 が versions/ に残る

## Milestone 2: Device Profile パイプライン + マルチ楽器(§15 M2)

- [x] `sora midi decompile`(SMF → PartPlan、キースイッチ逆解決)
- [x] drum_map コンパイル(kit_piece 解決、ch10 既定)
- [x] エフェクト系 Profile スキーマ(parameters / safe_range / automation_target)
- [x] マニュアル読解 → Profile 起草の Agent ワークフローを CLAUDE.md に追記
- [x] Heavier7Strings 実 Profile 作成(マニュアル 1.7.0 読解、`~/Documents/Heavier7Strings/sora-project/devices/`)+ validate + verify.mid 生成
- [x] CC レーン対応(Part Plan `controls`): H7S のパームミュート=CC16 のように連続 CC 型奏法を表現。実 Profile で CC16 パームミュートリフのコンパイル確認済み
- [x] 【ユーザー確認済み】H7S verify.mid + palm-riff.mid + one-key-fifth-ab.mid を Studio One 5 で実機確認 → 音域・4キースイッチ・CC16・CC26 を confidence: verified に更新
- [x] Ozone 9 実 Profile 作成(公式 Web ドキュメント読解、`~/Documents/Sora/devices/ozone9.profile.json`、confidence: manual、Dynamics ratio 2件は unverified)
- [ ] 【要ユーザー】Ozone 9 の実機確認(特に Dynamics ratio の値域、多数の safe_range 未確定パラメータ)
- [x] AmpliTube 5 実 Profile 作成(マニュアル読解 + プリセット776件の実測 + MidiAssignments 実例調査、`~/Documents/Sora/devices/amplitube5.profile.json`)。固定 CC マップが無いこと・Program Change でプリセット丸ごと切替・DAW Automation は Param1-16 固定スロットであることを実データで確認
- [x] Program Change 対応の実証: 「Metal→Warm Clean 2小節遷移」プロンプトに対する実出力を生成・バイトレベルで検証(`~/Documents/Sora/exports/metal-to-warm-clean.{plan.json,mid}` + 手順書 `tone/amplitube-transition-plan.md`)
- [ ] 【要ユーザー】AmpliTube 実機で MIDI Learn 設定後、`metal-to-warm-clean.mid` の実際の音の変化を確認
- [ ] 【要ユーザー】MODO BASS / MODO DRUM のマニュアル所在 → 実 Profile 作成
- [ ] missing_context 機構: references/context-requirements.json + レポート警告(§4.3)

**メモ**: 実 Profile は著作権配慮(§16 リスク4)で公開リポジトリに置かず、ユーザーのローカル(`~/Documents/Heavier7Strings/sora-project/`, `~/Documents/Sora/`)に配置。

**この作業で見つかったコード側のギャップ(修正済み)**:
- Part Plan に CC レーン(`controls`)が無く、CC 中心の奏法(H7S のパームミュート等)を表現できなかった → 追加
- `DeviceProfile.octave_convention` が全デバイス必須だった → マスタリング等、音程を扱わない effect(Ozone)には無意味な値を強制していた → note_range/keyswitches/drum_map があるときのみ必須に変更
- `profile validate` が keyswitches の unverified しか見ておらず、effect の parameters の unverified を見落としていた → 両方見るように修正
- Part Plan に Program Change が無く、「プリセット全体の切替」(AmpliTube 等)を表現できなかった → `program_changes` を追加。ノートを持たない effect 専用 Plan(controls/program_changes のみ)も compile 可能なことを確認

**AmpliTube の重要な学び**: H7S(固定デフォルト CC マップ)や Ozone(パラメータ一覧はマニュアル記載)と異なり、AmpliTube は**CC/DAW オートメーションの割当そのものがユーザーのその場限りの GUI 操作に依存**する。Sora が生成する CC/PC 入り MIDI は、ユーザーが AmpliTube 側で一度 MIDI Learn / Program Change 割当をしない限り無音のまま(壊れているのではなく仕様どおり)。この種のデバイスでは Plan 単体でなく「Plan + 手順書」の対で出すことが必須と判明(tone/amplitube-transition-plan.md がそのパターン)。

## Milestone 3: オーディオ解析 + トーン/マスタリングプラン(§15 M3)

- [x] sora-audio: symphonia デコード → ebur128(LUFS-I/LRA/true peak)
- [x] 7 バンド帯域バランス / クレストファクタ / ステレオ相関(realfft)
- [x] `sora audio analyze` / `sora audio compare`(§10)
- [x] references/genre-targets.json(初版: modern metal, j-rock)
- [x] Automation Plan スキーマ(§4.5)
- [ ] トーン/マスタリングプラン生成ワークフロー(UC7〜10, 16)を CLAUDE.md に追記

## Milestone 4: MCP 化 + 仮想 MIDI(§15 M4)

- [x] `sora midi send` / `sora midi panic`(midir + RAII オールノートオフ、§9)
- [x] plan_playback(テンポ解決・純粋関数・テスト済)
- [x] sora-mcp クレート(rmcp 2.x + tokio)、`sora mcp serve`(stdio、level 0-2 の 8 ツール公開。DAW 系 level 3+ は M5 で追加)
- [x] control level ゲート(全ツールに要求 level、超過は実行前拒否 + 有効化案内。config set は MCP 非公開)
- [x] actions.jsonl を MCP 実行系にも適用(拒否・エラーも記録。ops::record_action を CLI と共有)
- [x] doctor 拡張(仮想 MIDI ポート検出・IAC/loopMIDI セットアップ案内)
- [x] エラー表現の CLI/MCP 同一性を結合テストで保証(§6.4。report::normalize を共有)
- [ ] 【要ユーザー】IAC Driver 有効化 + Studio One 5 でのルーティング確認(doctor が手順を提示)
- [ ] 【要ユーザー】Claude Code / Codex への MCP サーバー登録(`sora mcp serve --root <音楽プロジェクト>`)→ UC1 の E2E 確認(M4 受け入れ基準)

## Milestone 5: DAW 統合(§15 M5)

- [x] 【調査完了 2026-07-05】Studio One 5 の制御経路を実機調査(§11.2.1 に確定事項を記載)。Claude Code レートリミット中に Codex が主に検証、Fable が別アングル調査で補強。
  - **確定**: 公式 AppleScript 辞書・公開 API は無い。だが内部 JS SDK(`musicdevices.bundle/.../sdk/*.d.ts`)+ MCU コントロールサーフェス + 拡張機構が使える。
  - **実機検証済み(Codex)**: Sora Bridge 拡張(`EditTask`)経由で、開いたままの GUI に MIDI インポート/コマンドを反映できる(初回検証時のトリガーは AppleScript)。
  - **別アングル調査(Fable)**: 純正 ATOM スクリプトが MIDI イベントから `interpretCommand` を実行している実例を確認 → **Sora Surface(MIDI トリガー式サーフェス)でトリガーを OS 非依存化できる**。UCNET は proprietary で行き止まり、`.song` ホットリロードも不可と確認。
  - **動作要件の明文化(§3.2)を受けた方針**: AppleScript は macOS 限定オプション(ダイアログ自動化)に隔離。コアトリガーは仮想 MIDI(全 OS)。
  - Codex 成果物: `~/Documents/Codex/2026-07-05/cl/outputs/`(検証レポート・Bridge プロトタイプ・インストーラ)。**Sora 作業ディレクトリ外**。M5 実装時にこれを参照実装として Rust 化する。
- [x] sora-daw クレート: DawAdapter trait + DawCapabilities + DawError(§11.1)
- [x] Generic(file-based)アダプタ(常設フォールバック): write_clip = exports/daw-import/ 配置 + 手順、write_automation = 手動適用手順書(Markdown)
- [x] Studio One アダプタ(§11.2.1): .song オフライン読解(BPM/拍子/トラック/マーカー。実サンプルで BPM 123 / 9 トラック読解確認)+ Bridge inbox/outbox キュー + Sora Surface(純 XML サーフェス。nanoKONTROL 2 実例と同じ `<Command>` マッピング)への仮想 MIDI トリガー
- [x] `sora daw setup studio-one`(Bridge 拡張 + Sora Surface の冪等インストール/--check/--uninstall。設定ファイルは必ずバックアップ。Codex インストーラの Rust 化)
- [x] `sora daw probe/read/transport/write-clip/render` / `sora automation apply`(CLI にも control level ゲート適用: probe/read/setup=3, transport/write/automation=4, render=5)
- [x] MCP ツール追加: read_daw_project / daw_transport / write_clip / write_automation / render_stem(計 13 ツール。stdio 疎通確認済み)
- [x] 書き込み前バックアップ + WriteReceipt(§11.4。song_path の .song を versions/daw-backups/ へコピー、取れなければ書き込み拒否。レシートを actions.jsonl に記録)
- [x] selection ケイパビリティ → **Note Selector で代替**(§11.3 更新済み)。Studio One に選択状態の取得経路が無いため、bars/section/pitch_min/pitch_max/note_indices の AND 結合セレクタを実装し、apply_articulations が対応(「Verse セクションの C2 以下」等の自然言語指定を Agent が翻訳)
- [x] daw read は project-context への直接書き込みをせず fill/conflict/add の反映提案を返す(§11.3 の両論併記は Agent の仕事)
- [~] REAPER 参照アダプタ → **スコープ外(2026-07-05 ユーザー判断)**。抽象の妥当性は Generic + Studio One + モックテストで確認
- [ ] 【要検証・要ユーザー】Sora Surface(MIDI トリガー)→ コマンド発火の結合(§11.2.1 で唯一未検証の結合部)。手順: (1) `sora config set control-level 3` 以上 → `sora daw setup studio-one` 実行(2) Studio One 再起動(3) Audio MIDI 設定で IAC ポート(例: "Sora Trigger")作成(4) Studio One 環境設定 > 外部デバイス に「Sora | Sora Surface」を追加し受信ポート割当(5) config の daw.studio_one.trigger_port 設定 → `sora daw transport stop` で反応確認
- [ ] 【要検証】EditTask コマンドカテゴリの特定: Sora Surface は Sora/TrackEdit/Track の 3 経路にマッピング済み(0x14-0x16)。どれが効くかは実機で確認し、不要な経路は後で削る
- [ ] 【残】Studio One の write_automation(マップ済みパラメータの MIDI CC 経路、要検証)と render(§11.2.1 未検証)。当面は generic フォールバック / 手動

## Milestone 6: 制作コパイロット(§15 M6)

- [ ] .claude/agents/: arrangement-reviewer / mix-reviewer / master-reviewer(§12.1)
- [ ] Production Memory: memory/ 雛形 + `sora memory compact`(§12.2)
- [ ] A/B 自動バウンス比較ワークフロー(§12.3)
- [ ] North Star シナリオ E2E テスト(§12.4)

---

## 未決事項(ユーザー確認待ち)

1. **実デバイス Profile の作成方針**(M2)。各プラグインのマニュアル(PDF)の所在を教えてもらえれば、Agent が読解して実 Profile を起草できる。マニュアルがなければ実測(verify-midi を鳴らして挙動観察)ベースで起草する。どちらで進めるか。
2. **verify.mid の実機確認**(M1/M2 の受け入れに必要)。Heavier7Strings 等で検証用 MIDI を鳴らし、キースイッチ発音を確認 → confidence 昇格。ユーザーの手が要る。
3. **M4 仮想 MIDI**: macOS の IAC Driver 有効化(Audio MIDI 設定)。実装後にルーティング確認をお願いする。
4. **M5 DAW 統合の対象**: 参照実装は REAPER(無料評価版で検証可)。実利用 DAW は Studio One 5 だが公式スクリプティング API が乏しい可能性。Studio One への到達手段(仮想 MIDI/ファイルインポート止まりか、それ以上を狙うか)は調査後に相談。

## 決定ログ

- 2026-07-04: リポジトリは dino3616/sora・Public、gh auth で作成(ユーザー回答)
- 2026-07-04: タスク管理は BACKLOG.md(本ファイル)で行う(Linear は不使用)
- 2026-07-04: 実 DAW は Studio One 5 と判明。M5 の調査対象を Studio One に設定、参照実装は REAPER のまま
- 2026-07-05: REAPER 参照アダプタはスコープ外(ユーザー指示)。selection 非対応の代替として Note Selector(自然言語範囲指定の構造化)を実装(ユーザー指示、§11.3 に明文化)
- 2026-07-05: `sora daw setup` の要求 control level は 3 とした(DAW 統合を有効化する環境セットアップ。読み取り系と同格)。--check は無変更のためゲート外(実装判断。異論あれば変更可)
- 2026-07-05: CLI の `midi send` には control level ゲート未適用のまま(M4 実装時の挙動を維持)。§5 の表では level 2 のため、適用するか要ユーザー確認
