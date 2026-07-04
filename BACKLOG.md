# Sora 実装バックログ

実装タスクの単一の管理台帳。**セッション再開時はまずこのファイルを読むこと。**
仕様の正: [docs/technical-requirements.md](docs/technical-requirements.md)(§番号は同書を指す)

## 運用ルール

- タスク完了ごとにチェックを付け、対応するコミットを作って push する
- 中断・再開はこのファイルの「現在地」と git log で判断する
- 要決定事項が出たら「未決事項」へ追記し、ユーザーへ質問する(勝手に決めない)
- 仕様変更が必要になったら docs/ を先に直してから実装する(上流→下流の順)

## 現在地

**Milestone 1 実装中** — プロジェクト足場を構築した段階。次: sora-core の型定義。

## 実環境メモ(ユーザー PC・実態確認可能)

- DAW: **Studio One 5**(docs の "Fender Studio Pro" は仮名。M5 のアダプタ調査対象は Studio One)
- インストール済みプラグイン: Heavier7Strings / MODO BASS / MODO DRUM / AmpliTube 5 / Waves Gold 同梱 VST 各種 / Ozone 9
- M2 の Profile 作成・M1 の検証用 MIDI 実機確認はこの環境で行える

---

## Milestone 0: プロジェクト足場

- [x] git init / Cargo workspace(sora-core, sora-audio, sora-cli)
- [x] rust-toolchain.toml / .gitignore / BACKLOG.md
- [ ] CLAUDE.md(ビルド・テスト・行動規範)
- [ ] GitHub リポジトリ作成(dino3616/sora, public)+ 初回 push
- [ ] CI(GitHub Actions: fmt / clippy / test / schema drift)

## Milestone 1: MVP コアループ(§15 M1)

sora-core:
- [ ] newtype 群: `MidiNote`(octave_convention 対応パース), `Velocity`, `BarBeatTick`, `Ppq`(§4.6 L3)
- [ ] エラー型: `CoreError` / `ValidationIssue` / `ErrorReport` + 終了コード規約(§6)
- [ ] モデル: `SoraConfig`(§4.1), `DeviceProfile`(§4.2), `ProjectContext`(§4.3, active_source 含む), `PartPlan`(§4.4)
- [ ] スキーマ生成: schemars → JSON Schema 出力 + description 必須(§4.6)
- [ ] 検証 3 層: jsonschema(L1)→ serde deny_unknown_fields(L2)→ ドメイン検証(L3)
- [ ] MIDI コンパイラ: PartPlan + Profile → SMF Format 1(§7 の全規約: keyswitch lead/momentary/latch, 音域検査, 衝突検査, humanize seed 固定, モノフォニックトリム)
- [ ] MIDI inspect: SMF → ノート/テンポ/CC ダンプ + 統計
- [ ] MIDI analyze: BPM 推定・調性中心(Krumhansl-Schmuckler)・リズム統計
- [ ] 検証用 MIDI 生成: Profile → 全奏法 1 音ずつの verify.mid(§16 リスク 1)

sora-cli:
- [ ] clap 骨格 + `--format json` + ErrorReport 出力 + 終了コード(§6.3)
- [ ] `sora midi compile` / `sora midi inspect` / `sora midi analyze`
- [ ] `sora profile validate` / `sora profile verify-midi`
- [ ] `sora schema dump [--out] [--check]`
- [ ] `sora config set control-level <0-5>`(確認表示 + actions.jsonl 記録、§2.4)
- [ ] `sora init`(プロジェクト雛形生成、§14)
- [ ] `sora version snapshot <label>`
- [ ] `sora doctor`(環境診断)

テスト・検証:
- [ ] golden file テスト(コンパイラのバイト同一性)+ insta スナップショット
- [ ] Heavier7Strings サンプル Profile(実機確認前は confidence: manual)
- [ ] examples/: ベースライン MIDI + E2E デモ手順
- [ ] CLAUDE.md に Sora ワークフロー(Plan 起草 → compile → 説明)を記述
- [ ] 【要ユーザー】verify.mid を Studio One 5 + Heavier7Strings で実機確認 → confidence 昇格

受け入れ(§15 M1): ベースライン → Plan → compile → 実 DAW で奏法発音 / ErrorReport のみで 1 往復自己修正 / v2 生成で v1 が versions/ に残る

## Milestone 2: Device Profile パイプライン + マルチ楽器(§15 M2)

- [ ] `sora midi decompile`(SMF → PartPlan、キースイッチ逆解決)
- [ ] drum_map コンパイル(kit_piece 解決、ch10 既定)
- [ ] エフェクト系 Profile スキーマ(parameters / safe_range / automation_target)
- [ ] マニュアル読解 → Profile 起草の Agent ワークフローを CLAUDE.md に追記
- [ ] 【要ユーザー】MODO BASS / MODO DRUM / AmpliTube 5 / Ozone 9 のマニュアル所在確認
- [ ] 各デバイスの Profile 作成 + validate + verify.mid 実機確認
- [ ] missing_context 機構: references/context-requirements.json + レポート警告(§4.3)

## Milestone 3: オーディオ解析 + トーン/マスタリングプラン(§15 M3)

- [ ] sora-audio: symphonia デコード → ebur128(LUFS-I/LRA/true peak)
- [ ] 7 バンド帯域バランス / クレストファクタ / ステレオ相関(realfft)
- [ ] `sora audio analyze` / `sora audio compare`(§10)
- [ ] references/genre-targets.json(初版: modern metal, j-rock)
- [ ] Automation Plan スキーマ + 手動適用ドキュメント生成(§4.5)
- [ ] トーン/マスタリングプラン生成ワークフロー(UC7〜10, 16)を CLAUDE.md に追記

## Milestone 4: MCP 化 + 仮想 MIDI(§15 M4)

- [ ] sora-mcp クレート(rmcp + tokio)、`sora mcp serve`
- [ ] control level ゲート(全ツールに要求 level、超過拒否 + 有効化案内)
- [ ] actions.jsonl(tracing JSON レイヤ、全実行系操作の記録)
- [ ] `sora midi send` / `sora midi panic`(midir + RAII オールノートオフ、§9)
- [ ] doctor 拡張(IAC Driver 検出・セットアップ案内)
- [ ] 【要ユーザー】IAC Driver 有効化 + Studio One 5 でのルーティング確認

## Milestone 5: DAW 統合(§15 M5)

- [ ] sora-daw クレート: DawAdapter trait + DawCapabilities + DawError(§11.1)
- [ ] Generic(file-based)アダプタ(常設フォールバック)
- [ ] REAPER 参照アダプタ(OSC + ReaScript、§11.2)※REAPER は無料評価版で検証可
- [ ] 【調査】Studio One 5 の制御経路(公式スクリプティング API 非公開の可能性が高い → 到達可能な範囲を実態調査し §11.2 へ追記)
- [ ] `sora daw probe/read/transport/write-clip` / `sora automation apply` / `sora daw render`
- [ ] 書き込み前バックアップ + WriteReceipt(§11.4)
- [ ] selection ケイパビリティ(「これ」の決定的参照、§11.3)

## Milestone 6: 制作コパイロット(§15 M6)

- [ ] .claude/agents/: arrangement-reviewer / mix-reviewer / master-reviewer(§12.1)
- [ ] Production Memory: memory/ 雛形 + `sora memory compact`(§12.2)
- [ ] A/B 自動バウンス比較ワークフロー(§12.3)
- [ ] North Star シナリオ E2E テスト(§12.4)

---

## 未決事項(ユーザー確認待ち)

(なし)

## 決定ログ

- 2026-07-04: リポジトリは dino3616/sora・Public、gh auth で作成(ユーザー回答)
- 2026-07-04: タスク管理は BACKLOG.md(本ファイル)で行う(Linear は不使用)
- 2026-07-04: 実 DAW は Studio One 5 と判明。M5 の調査対象を Studio One に設定、参照実装は REAPER のまま
