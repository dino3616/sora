# Sora 技術要件書

- 対象読者: Sora の実装者(Claude Code を含む)
- ステータス: Draft v0.3(全 Phase 対応・エラーハンドリング設計・スキーマ方針の再検証を追加)

## ドキュメント階層

`vision.md` → `journey-map.md` → `use-case.md` → 本書の順で上流である。上流ドキュメントは下流ドキュメントに依存しない(参照しない)。本書は最下流であり、上流が定義する要求・概念(Device Profile、Part Plan、control level 等)を実装方式へ具体化する。上流と本書が矛盾する場合、要求レベルの記述は上流が正、実現方式は本書が正である。

---

## 1. スコープと前提

本書は Vision の **Phase 1〜5 すべて**を対象とする。ただし確度は段階的である。

| Phase | 内容 | 本書での確度 |
|---|---|---|
| 1 | ファイルベースアシスタント | 実装可能レベルで具体化(§4〜§7, §10) |
| 2 | Device Profile システム | 実装可能レベルで具体化(§4.2, §5) |
| 3 | Claude Code + MCP 制御 | 実装可能レベルで具体化(§8, §9) |
| 4 | DAW-aware ワークフロー | アーキテクチャ確定・実装は DAW アダプタ調査後(§11) |
| 5 | 制作コパイロット | 構成要素と受け入れ基準を定義・詳細は Phase 4 完了後(§12) |

### 前提となるプロダクト判断(参照元ドキュメントより)

1. Sora は「接続されたコンテキスト」がプロダクトであり、生成器そのものではない。
2. すべての出力はレビュー可能(diff・バージョン管理・説明付き)でなければならない。
3. アーキテクチャは初日から Phase 5 を前提にしない。ただし Phase 5 への進化を妨げない。本書ではこれを **ケイパビリティモデル(§2.4)** として実装する。
4. 非破壊がデフォルト。提案と実行は分離する。

---

## 2. アーキテクチャ方針

### 2.1 基本構成: エージェント + 決定論的ツール + 構造化データ

Sora は単一のアプリケーションではなく、次の 3 層で構成する。

```text
┌─────────────────────────────────────────────┐
│ Agent 層: Claude Code(音楽的推論・意図解釈・説明) │
│   - CLAUDE.md / skills / subagents(Phase 5)     │
├─────────────────────────────────────────────┤
│ Tool 層: 決定論的 CLI ツール群(Rust 単一バイナリ)  │
│   - MIDI コンパイル/解析、奏法適用、オーディオ解析     │
│   - MCP サーバー(Phase 3)、DAW アダプタ(Phase 4) │
├─────────────────────────────────────────────┤
│ Data 層: スキーマ定義された JSON / MIDI / MD 成果物  │
│   - Device Profile, Project Context, Part Plan,  │
│     Automation Plan, Production Memory           │
└─────────────────────────────────────────────┘
```

- **音楽的判断は Agent 層**が行う(どこに余白を残すか、リズムをどう補完するか)。
- **バイナリ生成・数値処理・実機制御は Tool 層**が行う。LLM は `.mid` バイナリを直接生成せず、DAW を直接触らない。
- **両者のインターフェースが Data 層のスキーマ**である。スキーマが安定していれば、Agent 側のモデルやプロンプトを差し替えても成果物の互換性が保たれる。

### 2.2 最重要の設計判断: MIDI は中間表現(IR)経由で生成する

LLM がバイナリ MIDI を直接出力する方式は採らない。代わりに:

```text
自然言語リクエスト
  → Agent が Part Plan(JSON IR)を生成      … 音楽的判断・レビュー対象
  → sora midi compile が IR を検証           … スキーマ + Device Profile 制約
  → 奏法(キースイッチ/CC)を Profile から解決
  → 決定論的に .mid を出力                    … 常に再現可能
```

理由:

- Part Plan JSON がそのまま「レビュー可能な成果物」「diff 対象」「バージョン管理単位」になる(Vision 原則 5)。
- キースイッチのノート番号・タイミングのような機械的正確性が要求される処理を LLM から排除できる。
- Use case 5(既存 MIDI への奏法適用)は「.mid → IR 逆コンパイル → 奏法注釈 → 再コンパイル」として同じパイプラインで実現できる。
- Device Profile の差し替え(例: Heavier7Strings → 別のギター音源)が IR の再コンパイルだけで済む。

この「Agent は Plan を書き、Tool が実行する」パターンは Phase 4 でも同型で拡張する(Automation Plan → DAW アダプタが適用、§11)。

### 2.3 状態管理: プロジェクトディレクトリ = 単一の真実

Sora の全状態はユーザーのプロジェクトディレクトリ内のファイルとして持つ。DB・常駐プロセス・隠れた状態を持たない。これにより:

- git によるバージョン管理がそのまま Sora のバージョン管理になる(Journey stage 9)。
- ユーザーがいつでも全状態を検査・修正・削除できる(非ブラックボックス原則)。
- Claude Code のセッションをまたいだ「反復コンテキストの保持」がファイル読み込みで再現できる。

Phase 5 の Production Memory(§12.2)も同じ原則に従い、ファイルとして持つ。

### 2.4 ケイパビリティモデル: 段階的制御深度の実装

Vision 原則 6「段階的な制御深度」を、config で宣言する **control level** として実装する。全 Phase を貫く安全機構であり、後付けしない。

| Level | 内容 | 実行系ツール | 既定 |
|---|---|---|---|
| 0 | 提案のみ | なし(`.md`/`.json` 生成のみ) | |
| 1 | ファイル書き出し | `midi compile` 等の成果物生成 | ✅ 初期値 |
| 2 | 仮想 MIDI 送信 | `send_midi` | |
| 3 | DAW 読み取り | `read_daw_project`, transport 位置取得 | |
| 4 | DAW 書き込み | `write_clip`, `write_automation`, transport 操作 | |
| 5 | レンダリング・フルエージェント編集 | `render_stem`, 複合編集 | |

- `sora.config.json` の `control_level` が上限を宣言する。変更は `sora config set control-level <n>`(§5)でのみ行う。**引き上げはユーザーの明示的な依頼が前提**であり、Agent はユーザーが自然言語で依頼した場合に限り CLI 経由で実行してよい(自発的な実行の禁止は CLAUDE.md の行動規範として規定)。MCP ツールとしては公開しない。コマンドは現在値 → 新値と新たに有効になる操作を表示し、変更を actions.jsonl に記録する。
- 各 MCP ツール(§8)は要求 level をメタデータとして持ち、上限超過の呼び出しは実行前に拒否して「必要な level と有効化方法」を返す。
- Level 4 以上の操作は実行後に必ず undo 情報(§11.4)を actions.jsonl に残す。

---

## 3. 技術スタック

### 3.1 選定

Tool 層は **Rust(stable、edition 2024)** で実装し、単一バイナリとして配布する。

| 領域 | 採用クレート | 理由 |
|---|---|---|
| SMF 読み書き | midly | ゼロコピーで高速な SMF パーサ/ライタ。Format 0/1、メタイベント対応 |
| リアルタイム MIDI | midir | CoreMIDI(macOS)/ WinMM(Windows)/ ALSA(Linux)を単一 API で抽象化。仮想ポート送信(Phase 3)に使用 |
| スキーマ定義/検証 | serde + schemars + jsonschema(方針の根拠は §4.6) | Rust 型を単一ソースに、JSON Schema を自動生成。Agent 起草の JSON は 3 層検証 |
| エラーハンドリング | thiserror(lib 層)+ anyhow(bin 層)+ tracing | §6 で設計。型付きエラー・文脈付与・発生箇所追跡を分担 |
| ロギング/計測 | tracing + tracing-subscriber | `#[instrument(err)]` によるエラー追跡性の担保、actions.jsonl への構造化出力 |
| オーディオデコード | symphonia | WAV/AIFF/MP3/FLAC を pure Rust でデコード。**ffmpeg 依存を排除できる** |
| ラウドネス測定 | ebur128 | ITU-R BS.1770-4 / EBU R128 準拠(libebur128 の Rust 実装)。LUFS-I/LRA/true peak を網羅 |
| スペクトル解析 | realfft(+ rubato) | 帯域バランス・相関解析用 FFT とリサンプリング |
| 乱数(ヒューマナイズ) | rand + rand_chacha | ChaCha は seed 固定でバージョン間もストリームが安定 → 再現性保証 |
| CLI フレームワーク | clap(derive) | サブコマンド構成・型連動・シェル補完生成 |
| MCP サーバー | rmcp(公式 Rust SDK)+ tokio | Phase 3 で CLI と同一バイナリからツール公開 |
| OSC(Phase 4) | rosc | REAPER / AbletonOSC 等の DAW ブリッジ通信 |
| テスト | cargo test + insta | golden file(バイト列)+ スナップショット(JSON 出力)の回帰テスト |

Rust 採用が要件に与える利点:

- **単一静的バイナリ配布**: 音楽制作者は Python/Node ランタイムを持たない前提に立てる。`sora` バイナリ 1 つで Tool 層が完結し、オンボーディング(Journey stage 1)の摩擦が消える。
- **決定論性の担保**: GC・実行環境差の影響がなく、golden file テスト(§13)と相性が良い。
- **リアルタイム MIDI 送信・DAW 制御の信頼性**: 送信スレッドのタイミング精度とパニック時の確実なクリーンアップ(§9)を言語レベルで扱いやすい。

トレードオフとして、Python の music21 / librosa に相当する高水準の音楽理論・MIR ライブラリは存在しない。ただし本設計で必要なのは調性推定(Krumhansl-Schmuckler 鍵プロファイル相関)とリズム統計程度であり、小規模な自前実装で足りる。**PDF 抽出も Rust エコシステムが弱い領域だが、これは Tool 層から外し Agent 層に担わせる(§5 参照)**。

### 3.2 対応プラットフォーム

- Phase 1〜2: macOS / Windows / Linux(ファイル I/O のみなので差異なし)
- Phase 3(仮想 MIDI): macOS は IAC Driver(CoreMIDI)、Windows は loopMIDI 前提。OS 検出とセットアップガイドを `sora doctor` コマンドで提供する。
- Phase 4(DAW 制御): アダプタごとに対応 OS が決まる(§11)。

---

## 4. データモデルとスキーマ

すべてのスキーマは `schemas/` に JSON Schema として置き、Rust 型(serde + schemars)から `sora schema dump` で自動生成する。バージョンフィールド(`schema_version`)を必須とし、後方互換の移行パスを保つ。方針の根拠と検証設計は §4.6。

### 4.1 `sora.config.json` — 環境プロファイル(Journey stage 1)

```json
{
  "schema_version": "1.0",
  "daw": { "name": "Fender Studio Pro", "version": "2.1", "os": "macOS 15" },
  "control_level": 1,
  "devices": [
    { "id": "heavier7strings", "profile": "devices/heavier7strings.profile.json" },
    { "id": "ozone", "profile": "devices/ozone.profile.json" }
  ],
  "preferences": {
    "genres": ["modern metal", "j-rock"],
    "default_ppq": 480,
    "humanize": { "timing_ms": 8, "velocity": 10 }
  },
  "midi": { "port_name": "Sora Out" },
  "paths": { "exports": "exports/", "manuals": "manuals/" }
}
```

### 4.2 Device Profile — `devices/<id>.profile.json`(Use case 6)

Vision 原則 3「ハードコードより Device Profile」の実体。**確信度の明示**が仕様上の必須要件である点に注意(受け入れ基準: 不確かなフィールドが明示的にマークされている)。

```json
{
  "schema_version": "1.0",
  "id": "heavier7strings",
  "name": "Heavier7Strings",
  "vendor": "Three-Body Technology",
  "device_type": "instrument",
  "roles": ["rhythm_guitar", "lead_guitar"],
  "octave_convention": "C3=60",
  "note_range": { "low": "B0", "high": "E5" },
  "keyswitches": [
    {
      "articulation": "palm_mute",
      "note": "C0",
      "mode": "momentary",
      "confidence": "verified",
      "source": "manual p.23"
    },
    {
      "articulation": "pinch_harmonic",
      "note": "D#0",
      "mode": "latch",
      "confidence": "unverified",
      "source": "manual p.24 (表記が曖昧)"
    }
  ],
  "cc_map": [
    { "cc": 1, "function": "vibrato depth", "safe_range": [0, 100], "confidence": "verified" }
  ],
  "drum_map": null,
  "playing_conventions": [
    "メタルリフでは低音弦中心・パームミュート主体",
    "ピッキングハーモニクスはフレーズ末尾の長めノートで有効"
  ],
  "constraints": ["キースイッチは演奏ノートの直前に置く必要がある(同時発音は無効)"],
  "manual_refs": ["manuals/h7s-manual.pdf"]
}
```

設計要件:

- `confidence` は `verified`(ユーザーが動作確認済み) / `manual`(マニュアル記載のみ) / `unverified`(推測)の 3 値。コンパイラは `unverified` の奏法を使う際に出力レポートへ警告を含める。
- ノート表記は文字列(`"C0"`)と MIDI 番号のどちらも受けるが、正規化して番号で保持する。**オクターブ表記基準(C3=60 か C4=60 か)はプロファイルごとに `octave_convention` で宣言必須**。音源ベンダー間で不統一なため、ここを曖昧にするとキースイッチが全部ずれる。
- エフェクト系(AmpliTube, Ozone)は `keyswitches` の代わりに `parameters`(名称・単位・safe_range・プリセットカテゴリ)を持つ。instrument / effect でスキーマを分けず、nullable フィールドで単一スキーマに収める(ツール側の分岐を減らす)。Phase 4 では `parameters` に `automation_target`(DAW 側パラメータパス)を追記できる。

### 4.3 Project Context — `project-context.json`(Journey stage 2)

```json
{
  "schema_version": "1.0",
  "bpm": 142,
  "time_signature": "4/4",
  "key": { "tonic": "E", "mode": "minor", "confidence": "estimated" },
  "sections": [
    { "label": "verse", "start_bar": 1, "end_bar": 8 },
    { "label": "chorus", "start_bar": 9, "end_bar": 16 }
  ],
  "tracks": [
    {
      "id": "bass",
      "role": "bass",
      "source": "imports/bassline.mid",
      "active_source": "imports/bassline.mid",
      "device": "modo-bass",
      "analysis": "analysis/bassline-analysis.md"
    }
  ],
  "chord_progression": null,
  "user_notes": ["ヴァースのベースラインは確定。サビはまだ未定"],
  "references": []
}
```

- BPM・キー等は MIDI 解析からの推定値に `confidence: "estimated"` を付け、ユーザー申告値は `"stated"` とする。矛盾時はユーザー申告を優先し、Agent が確認する(Journey stage 2「曖昧さが有用な出力を妨げる場合のみ確認」)。
- Phase 4 では `read_daw_project` が同スキーマへ書き込む(`confidence: "daw"`)。手動記述と DAW 由来を同一スキーマで持つことで、Phase 1 → 4 の移行でファイル形式が変わらない。
- **指示語の解決(`active_source`)**: `tracks[].active_source` は「このリフ」等の指示語が指す現在の正(最新採用版)を保持し、Agent が新バージョン採用のたびに更新する。解決の優先順位は (1) ユーザーが明示したファイルパス、(2) DAW の選択状態(Phase 4、§11.3)、(3) `active_source`。Agent は**解決結果のファイルパスを応答で復唱し**(「= `exports/guitar-riff-v2.mid` として扱います」)、曖昧な場合のみ確認する(Journey stage 9)。
- **コンテキスト充足度チェック(missing_context)**: 生成系コマンドは、操作ごとの必要コンテキストマップ(`references/context-requirements.json`: どの操作に何が必須/推奨か)と project-context を突き合わせ、不足項目を `missing_context` 警告(項目・品質への影響・提供方法のヒント)としてレポートに含める。Agent はこれを「不足コンテキストのヒント提示」(Journey stage 2)に使う。提供を強制せず、不足のまま実行した場合の限界を成果物の説明に明記する。

### 4.4 Part Plan(IR)— `exports/<name>.plan.json`(全生成ユースケースの中核)

Agent が生成し、コンパイラが `.mid` へ変換する中間表現。

```json
{
  "schema_version": "1.0",
  "part_id": "guitar-riff-v1",
  "device": "heavier7strings",
  "bpm": 142,
  "time_signature": "4/4",
  "ppq": 480,
  "sections": [
    {
      "label": "verse",
      "start_bar": 1,
      "phrases": [
        {
          "notes": [
            {
              "pitch": "E1",
              "start": "1.1.000",
              "duration": "0.0.240",
              "velocity": 112,
              "articulation": "palm_mute"
            }
          ]
        }
      ]
    }
  ],
  "humanize": { "timing_ms": 8, "velocity": 10, "seed": 42 },
  "design_notes": "ベースのルート移動 E-G-A に対しリフは E ペダルで対比。"
}
```

設計要件:

- 時間表現は `bar.beat.tick`(人間可読・レビュー可能)。コンパイラが tick に解決する。
- `articulation` は文字列 ID。コンパイラが Device Profile の `keyswitches` から実ノートに解決する。**Profile に存在しない articulation はコンパイルエラー**(黙って無視しない)。
- `humanize.seed` 必須。同一 IR + 同一 Profile → バイト同一の `.mid` を保証する(再現性・テスト容易性)。
- ドラムは `pitch` の代わりに `kit_piece`(`"kick"`, `"snare"` 等)を書き、Profile の `drum_map` で解決する。

### 4.5 Automation Plan — `tone/<name>.automation.json`(Phase 4, UC の `write_automation`)

Part Plan と同型の「Agent が書き、Tool が適用する」IR。Phase 2〜3 では手動適用用ドキュメントの生成元、Phase 4 では DAW アダプタの入力になる。

```json
{
  "schema_version": "1.0",
  "target": { "track": "guitar-L", "device": "amplitube", "parameter": "gate.threshold" },
  "unit": "dB",
  "points": [
    { "at": "9.1.000", "value": -42.0, "curve": "linear" },
    { "at": "9.3.000", "value": -35.0, "curve": "smooth" }
  ],
  "rationale": "サビ頭でゲートを緩め、リフのサステインを残す"
}
```

- `parameter` は Device Profile の `parameters` に存在するものに限る(コンパイル時検証)。`safe_range` 外の値は明示フラグなしにエラー。

### 4.6 スキーマ方針の根拠と検証設計

v0.2 の「serde + schemars + jsonschema」という 1 行方針を再検証した結論: **方向性(Rust 型を単一ソースとするコードファースト)は維持するが、検証を 3 層に分けて役割を明確化する**。「serde があるのに jsonschema での事前検証は冗長ではないか」という妥当な批判があるため、採用理由を明記する。

#### 検証 3 層

| 層 | 手段 | 責務 | なぜこの層が必要か |
|---|---|---|---|
| L1 構造検証 | jsonschema クレート(draft 2020-12) | 型・必須フィールド・enum 値・パターン | **全エラーを JSON Pointer 付きで一括列挙**できる。serde は最初のエラーで停止しメッセージも不親切なため、JSON の書き手が LLM である本プロダクトでは L1 の網羅的エラーリストが Agent の自己修正ループの効率を直接決める |
| L2 型付き変換 | serde(`deny_unknown_fields`) | JSON → Rust 型 | Agent 起草入力(Plan/Profile)のフィールド名 typo(`articulaton` 等)を黙殺しない。schemars 側では `additionalProperties: false` として L1 にも反映される |
| L3 ドメイン検証 | Rust コード(newtype + TryFrom) | 相互制約: キースイッチ×音域の衝突、bar の単調増加、safe_range、Profile 参照整合 | JSON Schema では表現できない制約。`MidiNote` / `Velocity` / `BarBeatTick` を newtype にし "parse, don't validate" を徹底 — 検証済みであることを型で運ぶ |

L1〜L3 のエラーはすべて §6 の `ErrorReport` に正規化して返す。

#### 検討した代替案と不採用理由

| 代替案 | 不採用理由 |
|---|---|
| スキーマファースト(JSON Schema 手書き + typify で Rust 生成) | 生成型にドメイン newtype・メソッド・不変条件を付与しにくく、L3 が型から分離してしまう。スキーマの表現力に設計が制約される。Agent への仕様伝達という目的は、コードファーストで生成したスキーマでも同等に果たせる |
| validator / garde によるフィールドアノテーション検証 | 本プロダクトの検証の主戦場は相互制約(L3)であり、フィールド単発検証の宣言的記法の利点が薄い。単純な範囲制約は schemars の属性で JSON Schema 側(L1)に寄せられる。将来 L3 が肥大化したら garde 併用を再検討 |
| serde 単独(L1 省略) | 実装は最小になるが、Agent が受け取るエラーが「1 件ずつ・パス情報が貧弱」になり、自己修正の往復回数が増える。Plan JSON は数百ノート規模になるため一括列挙の価値が大きい |
| protobuf / 別 IDL | 成果物は「ユーザーがエディタで読める JSON」であること自体が要件(レビュー可能性)。JSON Schema は Agent へのフォーマット仕様伝達手段としてそのまま使える点で優位 |

#### 運用要件

- `sora schema dump --check` を CI に置き、Rust 型と `schemas/` 生成物のドリフトを検出する。
- schemars は 1.x 系(draft 2020-12 既定)を使用する。0.8 系とは API・出力が異なるため混在させない。
- `schema_version` は「major が異なる入力は拒否 + 移行手順提示、minor は受理」とする。破壊的変更時は `sora migrate <file>` を同時に出荷する。
- スキーマ生成物には `description` を必ず付ける(Rust doc コメント → schemars 経由)。Agent がスキーマを読んで Plan を起草するため、description がプロンプトの一部として機能する。

---

## 5. Tool 層: CLI コマンド仕様

単一バイナリ `sora`、エントリポイント `sora <サブコマンド>`。全コマンドは JSON を stdout に返し(`--format json`)、Agent が解析できるようにする。

### Phase 1〜2 で実装するコマンド

| コマンド | 入力 | 出力 | 対応ユースケース |
|---|---|---|---|
| `sora midi inspect <file.mid>` | SMF | ノート/テンポ/CC の JSON ダンプ + 統計(音域・密度・拍分布) | UC1, stage 2 |
| `sora midi analyze <file.mid>` | SMF | 推定 BPM/調性中心/リズムモチーフ/セクション境界候補 | stage 2 |
| `sora midi compile <plan.json>` | Part Plan + Profile | `.mid` + コンパイルレポート(警告含む) | UC1〜5 |
| `sora midi decompile <file.mid> --device <id>` | SMF + Profile | Part Plan JSON(キースイッチを articulation へ逆解決) | UC5 |
| `sora profile validate <profile.json>` | Profile | スキーマ検証 + 整合性チェック(キースイッチと音域の衝突等) | UC6 |
| `sora schema dump [--out schemas/] [--check]` | — | Rust 型から生成した JSON Schema 一式 / CI 用ドリフト検査 | 全 UC の前提 |
| `sora audio analyze <file.wav>` | WAV/AIFF/MP3/FLAC | LUFS-I/LRA/true peak/帯域バランス/クレストファクタの JSON | UC9, stage 6 |
| `sora version snapshot <label>` | exports/ | versions/vN/ 作成 + CHANGELOG 追記 | UC15 |
| `sora config set control-level <0-5>` | — | 現在値 → 新値と新たに有効になる操作を表示して変更、actions.jsonl に記録。**ユーザーの明示的依頼時のみ実行可・MCP 非公開**(§2.4) | 全 UC の前提 |
| `sora doctor` | — | 環境診断(仮想 MIDI ポート検出、プロジェクト構成、control_level 表示等) | オンボーディング |

役割分担の原則: **「マニュアルのどこが重要か」「どんな奏法が音楽的に妥当か」の判断は Agent、検証・変換の機械処理は CLI**。

UC6(マニュアル → Device Profile)の PDF 読解は **Tool 層に置かず Agent 層が担う**。Claude Code はネイティブに PDF を読めるため、Rust 側に品質の劣る PDF 抽出を実装する必要がない。フローは「Agent が PDF を読み Profile を起草 → `sora profile validate` で機械検証 → 不整合を Agent が修正」の往復になる。巨大マニュアル(数百ページ)は Agent がページ範囲指定で分割読解する。

### Phase 3 で追加するコマンド

| コマンド | 内容 | 要求 level |
|---|---|---|
| `sora midi send <file.mid> --port <name>` | 仮想 MIDI ポートへリアルタイム送信(§9) | 2 |
| `sora midi panic --port <name>` | 全ノートオフ + サステインリセット | 2 |
| `sora mcp serve` | 全コマンドを MCP ツールとして公開(§8) | — |

### Phase 4〜5 で追加するコマンド

| コマンド | 内容 | 要求 level |
|---|---|---|
| `sora daw probe` | 接続可能な DAW アダプタとケイパビリティの検出(§11.2) | 3 |
| `sora daw read [--section <label>]` | DAW プロジェクト状態 → project-context.json へ反映 | 3 |
| `sora daw transport <locate\|play\|stop\|record>` | トランスポート制御(UC13) | 4 |
| `sora daw write-clip <file.mid> --track <id>` | MIDI クリップの配置 | 4 |
| `sora automation apply <plan.automation.json>` | Automation Plan を DAW へ適用(§4.5) | 4 |
| `sora daw render --track <id> --out <path>` | ステム/ミックスのレンダリング要求 | 5 |
| `sora audio compare <a.wav> <b.wav>` | A/B の解析差分レポート(LUFS/帯域/ダイナミクス差) | 1 |
| `sora memory compact` | decision-log から Production Memory への要約反映の機械部分(§12.2) | 1 |

---

## 6. エラーハンドリング設計

方針の土台として [Rustのエラークレート選定方法](https://qiita.com/namn1125/items/2cf917604b476d6a43bb) の評価 4 軸 — **回復性・網羅性・可視性・追跡性** — を採用し、Sora の各クレートに割り当てる。同記事の整理(小規模・回復不要なら anyhow、回復前提の規模なら thiserror + tracing)に対し、Sora は「lib 層は回復・分岐が必要、bin 層は最上位で一括報告」という構造なので、**thiserror(lib)+ anyhow(bin)+ tracing(横断)** のハイブリッドとする。

### 6.1 クレート別の役割分担

| クレート | エラー型 | 理由(4 軸との対応) |
|---|---|---|
| sora-core / sora-audio / sora-daw(lib) | thiserror による型付き enum | [回復性] Agent が自己修正すべきエラー(検証系)と環境エラーを呼び出し側が `match` で分岐できる。[網羅性] variant 追加時にコンパイラが処理漏れを検出。[可視性] シグネチャ `Result<T, CoreError>` で失敗可能性が明示される |
| sora-cli / sora-mcp(bin) | anyhow::Result + `.context()` | 最上位は分岐せず報告するだけなので型情報は不要。`.with_context(\|\| format!("compiling {}", path))` で操作文脈を積む |
| 横断 | tracing の `#[instrument(err)]` | [追跡性] リリースビルドでもエラー発生関数と引数が actions.jsonl / stderr ログに残る。anyhow 単独の弱点(リリースビルドで発生箇所が不明)を補完する |

### 6.2 lib 層エラー型の設計

エラーは Agent が読んで自己修正するための「成果物」でもあるため、Display 文字列に加えて**構造化ペイロード**を持たせる。

```rust
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("plan validation failed with {} issue(s)", .issues.len())]
    PlanValidation { issues: Vec<ValidationIssue> },   // L1〜L3 の全検証エラーを集約

    #[error("unknown articulation `{name}` for device `{device}`")]
    UnknownArticulation { name: String, device: String, available: Vec<String> },

    #[error("note {note} out of range {low}..={high} for `{device}`")]
    NoteOutOfRange { note: u8, low: u8, high: u8, device: String, transpose_hint: Option<i8> },

    #[error("keyswitch {note} collides with playable range of `{device}`")]
    KeyswitchCollision { note: u8, device: String },

    #[error("MIDI parse error in {path}")]
    MidiParse { path: PathBuf, #[source] source: midly::Error },

    #[error("I/O error on {path}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
}
```

設計規約:

1. **variant は「Agent が取るべき次のアクション」が異なる単位で分ける**。同じ修正方法になるものを細分化しない。
2. 下位エラーは `#[source]` / `#[from]` で必ず連鎖させ、原因チェーンを保つ(anyhow の `{:#}` 表示および tracing に流れる)。
3. 修正のヒントになる情報(`available`, `transpose_hint` 等)を variant のフィールドとして持つ。文言に埋め込むのではなくデータで持つ — Agent とユーザー向け表示の両方で使うため。
4. パニックはバグとして扱う。回復可能な状況で `panic!` / `unwrap()` を使わない。ただし §9 の MIDI クリーンアップは RAII ガードにより panic 経路でも保証される。`.expect()` はブートストラップ(config 読込前の初期化)に限り許可。

### 6.3 bin 層: ErrorReport への正規化と終了コード

CLI / MCP の最上位で anyhow のチェーンを走査し、`CoreError` を downcast して構造化 JSON `ErrorReport` に正規化する。

```json
{
  "error": {
    "code": "UNKNOWN_ARTICULATION",
    "message": "unknown articulation `palm-mute` for device `heavier7strings`",
    "details": { "name": "palm-mute", "available": ["palm_mute", "pinch_harmonic"] },
    "hint": "articulation は Profile の keyswitches に定義された ID を使ってください",
    "chain": ["while compiling exports/guitar-riff-v1.plan.json"]
  }
}
```

- `code` は SCREAMING_SNAKE_CASE で安定 ID(variant 名から導出)。Agent はこれで分岐する。
- `chain` は anyhow の context スタック。追跡性をユーザー向け出力にも反映する。
- downcast できない予期しないエラーは `code: "INTERNAL"` とし、tracing ログの参照を促す。

終了コード規約:

| code | 意味 | Agent の対応 |
|---|---|---|
| 0 | 成功(警告があっても) | レポートの warnings を確認 |
| 1 | 検証・ドメインエラー(自己修正可能) | ErrorReport を読んで入力を修正し再実行 |
| 2 | 使用法エラー(clap) | コマンドラインを修正 |
| 3 | 環境エラー(ポート未検出・ファイル不在・DAW 未接続) | ユーザーへ環境設定を案内 |
| 4 | 内部エラー(バグ) | 再試行せず報告 |

MCP サーバー(§8)は同じ `ErrorReport` を MCP のツールエラーレスポンスに載せる。CLI と MCP でエラー表現が同一であることを結合テストで保証する。

---

## 7. MIDI 技術仕様

コンパイラが遵守する規約。全生成ユースケースの受け入れ基準(「DAW に正常にインポートできる」「奏法が正しく表現されている」)を保証する層。

1. **フォーマット**: SMF Format 1、PPQ 480。トラック 0 にテンポ・拍子メタイベント、トラック 1 以降に演奏データ。
2. **キースイッチ配置**: 対象ノートの `keyswitch_lead_ticks`(デフォルト 20 tick ≒ 10ms@142BPM、Profile で上書き可)前に置く。`mode: momentary` はノートと同長、`mode: latch` は短ノート(10 tick)で次の切替まで有効とみなす。
3. **キースイッチ衝突検査**: キースイッチノートが Profile の演奏音域と重なる場合はコンパイルエラー(UC5 受け入れ基準)。
4. **音域検査**: 演奏ノートが `note_range` 外ならエラー。移調で解決可能な場合は警告 + 提案をレポートに出す。
5. **ヒューマナイズ**: タイミングは正規分布(σ = `timing_ms`、±3σ でクリップ)、ベロシティは一様分布。**小節頭のキックとベースのダウンビートは対象外**(グルーヴの基準点を守る)。seed 固定で決定論的。
6. **ノートオーバーラップ**: モノフォニック指定の Profile(ベース等)では同一ピッチの重なりを自動トリム。ポリフォニックはそのまま。
7. **ドラムチャンネル**: `drum_map` を持つ Profile はチャンネル 10 を既定にしつつ Profile で上書き可(MODO DRUM 等はマルチチャンネル構成があり得る)。

---

## 8. MCP サーバー仕様(Phase 3〜)

`sora mcp serve` が stdio で公開するツール。Journey stage 8 のツール一覧と対応させ、各ツールに要求 control level(§2.4)を付す。

| MCP ツール | 実体 | 破壊性 | 要求 level |
|---|---|---|---|
| `analyze_project` | project-context.json + 各解析の集約読み取り | read-only | 0 |
| `read_midi` | `midi inspect` / `midi analyze` | read-only | 0 |
| `compose_part` | Part Plan の検証 + `midi compile` | 新規ファイル作成のみ | 1 |
| `apply_articulations` | `midi decompile` → 注釈 → `midi compile` | 新規ファイル作成のみ | 1 |
| `export_midi` | コンパイル済み `.mid` の配置 | 新規ファイル作成のみ | 1 |
| `suggest_plugin_settings` | Profile 参照 + settings.json 生成 | 提案のみ | 1 |
| `analyze_audio` | `audio analyze` / `audio compare` | read-only | 1 |
| `send_midi` | `midi send` | 実時間送信 | 2 |
| `read_daw_project` | `daw read`(§11) | read-only(DAW 接続) | 3 |
| `daw_transport` | `daw transport`(UC13) | 再生位置・録音状態の変更 | 4 |
| `write_clip` | `daw write-clip` | DAW プロジェクト変更 | 4 |
| `write_automation` | `automation apply` | DAW プロジェクト変更 | 4 |
| `render_stem` | `daw render` | CPU 負荷・ファイル生成 | 5 |

横断要件:

- **操作ログ**: 全ツール呼び出しを `logs/actions.jsonl`(timestamp, tool, args, result summary, undo 情報)へ追記(UC13 受け入れ基準)。tracing の JSON レイヤで実装し、§6 のエラーも同じストリームに乗せる。
- **上書き禁止**: 既存パスへの書き込み要求は既定で拒否し、`--force` 相当のフラグを MCP には公開しない。別名保存 + snapshot を返す(安全ルール 3)。
- **提案/実行の分離**: `suggest_*` 系はファイル生成のみ。実環境に作用するツール(level 2+)は `control_level` の引き上げ(ユーザーの明示的依頼に基づく `config set`、§2.4)が前提で、level 4+ は §11.4 の undo 情報記録を伴う。

---

## 9. 仮想 MIDI 送信(Phase 3, UC14)

- macOS: CoreMIDI IAC Driver、Windows: loopMIDI。midir 経由で接続し、ポート名は config の `midi.port_name` で指定。存在しない場合は `sora doctor` がセットアップ手順を提示(自動作成はしない)。
- 送信は専用スレッドで `.mid` イベントを実時間スケジューリングする。送信ハンドルを RAII ガードで包み、**Drop 時(正常終了・エラー・panic のいずれでも)に全チャンネルへ CC123(All Notes Off)+ CC64=0 を送る**ことを型レベルで保証する。加えて Ctrl-C は signal ハンドラで捕捉して同じクリーンアップ経路に合流させる。
- **ノート鳴りっぱなし防止が受け入れ基準**なので、(a) RAII ガードによる自動クリーンアップ、(b) `panic` コマンド(手動リセット)、(c) 送信終了時の明示的 all-notes-off、の 3 重で担保する。
- 送信結果(送信イベント数、所要時間、中断有無)を JSON で返し actions.jsonl に記録する。

---

## 10. オーディオ解析仕様(Phase 1 後半〜, UC9/UC10)

`sora audio analyze` の測定項目:

| 項目 | 手法 | 用途 |
|---|---|---|
| Integrated Loudness / LRA | ebur128 クレート(BS.1770-4 / EBU R128) | マスタリング目標(例: 配信 -14 LUFS)との比較 |
| True Peak | ebur128 の true peak モード(4x オーバーサンプリング) | クリッピングリスク検出 |
| 帯域バランス | realfft による 7 バンド(sub/low/low-mid/mid/high-mid/high/air)RMS 比 | 「低域の蓄積」「耳に痛い高域」の定量化 |
| クレストファクタ | ピーク/RMS(全体 + 帯域別) | 「スネアを潰さない」系リクエストの検証 |
| ステレオ相関 | 帯域別相関係数 | 低域モノ互換チェック |

デコードは symphonia で行い、外部バイナリ(ffmpeg)依存を持たない。symphonia が扱えない形式(例: 一部の AAC コンテナ)は非対応としてエラーメッセージで変換方法を案内する。

- 出力は `analysis/loudness.json` + Agent が所見を書く `rough-mix-report.md` の 2 段構え。**数値化はツール、解釈と優先度付けは Agent**(UC9 受け入れ基準「具体的で優先度付けされている」)。
- ジャンル別リファレンス値(モダンメタルの典型帯域バランス等)は `references/genre-targets.json` として同梱し、Agent が比較に使う。初版は少数ジャンルのみ、確信度を明示。
- `sora audio compare <a> <b>`(Phase 4〜5 の A/B ワークフロー用)は上記全項目の差分を構造化 JSON で返し、Agent が `variant-comparison.md`(UC11)を書く材料にする。
- **トーンマッチング(UC16)**: プリセット内部パラメータは非公開形式で直接読めない(§16 リスク 6)ため、「参照元プリセットで鳴らしたステムの `analyze` → 再現先チェーン提案 → 再現側レンダリングとの `compare` → 差分に基づく調整」の反復で音ベースの再現を行う。収束判定は帯域バランス差・クレストファクタ差の閾値で Agent が説明する。

---

## 11. Phase 4: DAW 統合レイヤー

### 11.1 設計方針

DAW ごとの制御手段(スクリプティング API・OSC・リモートプロトコル)は互換性がないため、**`sora-daw` クレートに `DawAdapter` トレイトを置き、DAW ごとにアダプタを実装する**。Agent と MCP ツールはアダプタの抽象操作のみを見る。

```rust
pub trait DawAdapter {
    fn capabilities(&self) -> DawCapabilities;   // 何ができるかを実行時に申告
    fn read_project(&mut self) -> Result<DawProjectState, DawError>;
    fn transport(&mut self, cmd: TransportCmd) -> Result<TransportState, DawError>;
    fn write_clip(&mut self, req: WriteClipRequest) -> Result<WriteReceipt, DawError>;
    fn write_automation(&mut self, plan: &AutomationPlan) -> Result<WriteReceipt, DawError>;
    fn render(&mut self, req: RenderRequest) -> Result<RenderReceipt, DawError>;
}
```

- `capabilities()` が返す集合(read / transport / clip / automation / render の各可否)に応じて、MCP ツールは非対応操作を「このアダプタでは未対応、代替はファイル書き出し」と即答する。**アダプタが未成熟でも Phase 1〜3 のファイルベース経路が常にフォールバックとして残る**ことが、段階的深化(Vision 原則 6)の実装上の意味である。
- `DawError` は thiserror enum(§6.1 の lib 層規約に従う)。接続断・タイムアウト・非対応操作・DAW 側拒否を variant で区別し、Agent がフォールバック判断できるようにする。

### 11.2 アダプタ実装の優先順位と通信経路

| 優先 | DAW | 経路 | 備考 |
|---|---|---|---|
| 参照実装 | REAPER | OSC(rosc)+ ReaScript ブリッジ | API が最も広く文書化されており、read/write/render/transport の全ケイパビリティを検証できる。アダプタ抽象の妥当性確認に使う |
| 2 | Ableton Live | AbletonOSC(または Max for Live ブリッジ) | clip/transport は可能、render は制限あり |
| 3 | ユーザーの実 DAW | 調査タスク(§16 リスク 3) | journey-map 記載の DAW のスクリプティング可否を確認後に実装判断 |
| 常設 | Generic(file-based) | ファイル書き出し + インポート手順の提示 | capabilities = 出力のみ。全 DAW で動く最後の砦 |

- 通信はローカルホスト限定(OSC の bind は 127.0.0.1 固定)。リモート DAW 制御は要件外とし、経路を持たない。
- ヘッドレス VST ホスティング(DAW を介さず Rust 側でプラグインをロードしてレンダリング)は **R&D 項目とし、Phase 4 のクリティカルパスに置かない**(§16 リスク 7)。

### 11.3 read_daw_project の動作

- DAW から取得した BPM・トラック構成・マーカー等を project-context.json に `confidence: "daw"` でマージする。手動記述(`stated`)と衝突した場合は上書きせず両論併記し、Agent がユーザーに確認する。
- 取得できない情報(コード進行等)は従来どおり手動/解析由来のまま残る。**DAW 統合はコンテキストの取得経路が増えるだけで、データモデルは Phase 1 と同一**(§4.3)。
- アダプタが選択中のトラック/クリップの安定 ID を取得できる場合(capabilities: selection)、「これ」系の指示語解決(§4.3)においてこの選択状態を `active_source` より優先する — ユーザーが DAW 上で指しているものが最も決定的な参照であるため。

### 11.4 書き込み操作の安全規約(Level 4+)

1. **書き込み前スナップショット**: `write_clip` / `write_automation` は実行前に対象トラックの現状態(既存クリップ・既存オートメーション)をアダプタ経由で読み取り、`versions/daw-backups/` に保存する。読み取り不能な場合は書き込みを拒否する(undo 不能な操作をしない)。
2. **WriteReceipt**: 全書き込みは「何を・どこへ・undo 手順」を含むレシートを返し、actions.jsonl に記録する(UC13 受け入れ基準の拡張)。
3. **既存素材の変更禁止**: 書き込みは常に新規クリップ・新規オートメーションレーン(または明示的に空の領域)への追加とする。既存クリップの置換はユーザーの明示指定 + プレビュー提示を必須とする(安全ルール 3・6)。
4. **プレビュー経路**: Level 2(仮想 MIDI 送信)を「書き込まずに聴く」手段として位置づけ、Agent は Level 4 書き込みの前に Level 2 試聴を提案するワークフローを既定とする。

---

## 12. Phase 5: 制作コパイロット

Phase 5 は新しいランタイムではなく、**Phase 1〜4 の部品を Agent 層のオーケストレーションで束ねる段階**である。Tool 層への追加は少ない(`audio compare` / `memory compact` 程度)。

### 12.1 マルチエージェントレビュー(use-case「将来的な高度ユースケース」)

- リポジトリの `.claude/agents/` に役割別レビュアーを定義する: `arrangement-reviewer`(アレンジ・余白・セクション対比)、`mix-reviewer`(帯域衝突・ダイナミクス)、`master-reviewer`(ラウドネス・トランジェント・配信基準)。
- 各レビュアーは read-only(analyze 系ツールのみ)で並列実行し、所見を `reviews/<date>-<role>.md` に出力する。統合判断と優先度付けはメインの Agent が行う。
- レビュアーの入力は Data 層の成果物(plan.json / loudness.json / project-context.json)に限定する。これにより Phase 5 の品質は Phase 1〜4 の成果物スキーマの充実度に還元され、独自の状態を持たない。

### 12.2 Production Memory(プロジェクト全体の制作メモリ)

§2.3 の原則どおりファイルで持つ。2 スコープに分ける。

| スコープ | 置き場所 | 内容 |
|---|---|---|
| プロジェクト | `<song>/memory/production-notes.md` | この曲での確定事項・却下案・ユーザーの好みの発現(「サビ前は空ける」等) |
| ユーザー横断 | `~/.sora/preferences.md` | 曲をまたぐ嗜好(ジャンル既定、ヒューマナイズ強度、ラウドネス好み)。プロジェクトへは読み取り専用で参照 |

- 更新フロー: decision-log.md(UC15)への追記が一次記録。`sora memory compact` が decision-log の未反映エントリを抽出し、**要約と昇格判断(プロジェクト固有か横断嗜好か)は Agent が行う**。ここでも機械処理と判断の分担(§5)を守る。
- Memory はプロンプトへ全文注入せず、Agent が必要時に読む。肥大化対策として compact 時に古いエントリの統合を促す。

### 12.3 A/B 自動バウンス比較

UC11(A/B バリエーション)+ Phase 4 render の合成ワークフロー:

1. Agent が Variant A/B の Plan を生成(既存機能)
2. `write_clip` + `render_stem` で両方をバウンス(Level 5)
3. `sora audio compare` で差分を数値化
4. Agent が `variant-comparison.md` に「数値差 + 音楽的解釈 + 推奨」を書く

Level 5 未満の環境では手順 2 をユーザーの手動レンダリングに置き換え、残りは同一に動く(段階的深化の維持)。

### 12.4 Phase 5 の受け入れ基準

- North Star リクエスト(vision.md 記載の複合依頼)が、1 回の対話セッションで「解析 → 2 リフ案 → ドラム/ベース連携 → トーン提案 → レビュー所見 → レビュー可能ファイル一式」まで到達する。
- マルチエージェントレビューの所見が、単一 Agent の所見に対して指摘の重複が少なく観点が分離していることを、サンプル楽曲セットで人間評価する。
- Production Memory 導入後、過去に却下した提案を Sora が繰り返さないことをシナリオテストで確認する。

---

## 13. 非機能要件

| 項目 | 要件 |
|---|---|
| 再現性 | 同一入力(IR + Profile + seed)→ バイト同一の `.mid`。CI の golden file テストで担保 |
| 非破壊性 | ユーザー由来ファイル(imports/, manuals/)への書き込み禁止をツール層で強制。Sora 生成物も上書きせず新パス + snapshot。DAW 書き込みは §11.4 の規約に従う |
| 性能 | `midi compile`/`inspect` は 1 秒未満、`audio analyze` は 5 分楽曲で 30 秒未満(Agent の対話テンポを壊さない)。DAW アダプタ操作はタイムアウト 5 秒でエラー(ハングしない) |
| オフライン動作 | Tool 層はネットワーク不要(DAW アダプタの OSC もローカルホスト限定)。Agent 層(Claude)のみオンライン依存 |
| プライバシー | マニュアル PDF・音声・Profile・Memory はローカル保存のみ。ツールが外部送信する経路を持たない |
| エラー設計 | §6 に従う。検証エラーは全件列挙・JSON Pointer 付き・修正ヒント付きで、Agent が 1 往復で自己修正できる形式にする |
| 可観測性 | tracing による構造化ログ。level 2+ の実行系操作は actions.jsonl に全記録(§8) |
| テスト | スキーマ検証 / コンパイラ golden file(insta スナップショット + バイト列比較)/ decompile→compile ラウンドトリップ / ErrorReport の安定性(code の後方互換)/ 実 DAW インポート手動チェックリスト / DAW アダプタはモック + REAPER 実機スモーク |
| 配布 | macOS(arm64/x86_64)・Windows・Linux 向けに単一バイナリを GitHub Releases + cargo-dist で配布。ランタイム依存ゼロ |

---

## 14. リポジトリ構成案

Cargo workspace 構成。コア(型・コンパイラ)を lib クレートに分離し、CLI / MCP から共有する。

```text
sora/
├── CLAUDE.md                  # Agent 層: Sora の振る舞い・安全ルール・ワークフロー定義
├── .claude/agents/            # Phase 5: 役割別レビュアー定義
├── docs/                      # vision / journey-map / use-case / 本書
├── schemas/                   # JSON Schema(`sora schema dump` で Rust 型から生成、CI でドリフト検査)
├── Cargo.toml                 # workspace ルート
├── crates/
│   ├── sora-core/             # serde 型(profile / context / plan / automation)、MIDI コンパイラ/デコンパイラ、解析、CoreError
│   ├── sora-audio/            # symphonia デコード + ebur128 / realfft 解析、compare
│   ├── sora-daw/              # DawAdapter トレイト + reaper / ableton / generic アダプタ(Phase 4)
│   ├── sora-cli/              # clap エントリポイント(バイナリ名 `sora`)、anyhow + ErrorReport 出力
│   └── sora-mcp/              # rmcp サーバー(Phase 3〜、`sora mcp serve` から起動)
├── references/                # ジャンル別ターゲット値
├── tests/
│   └── golden/                # 期待 .mid バイト列 + insta スナップショット
└── examples/                  # サンプル Profile とベースライン MIDI(E2E デモ用)
```

ユーザーの楽曲プロジェクトは別ディレクトリ(`sora init` が雛形生成):

```text
my-song/
├── sora.config.json
├── project-context.json
├── devices/        ├── manuals/      ├── imports/
├── exports/        ├── analysis/     ├── tone/
├── versions/       ├── logs/         ├── memory/
├── reviews/        └── decision-log.md
```

---

## 15. 実装マイルストーン

### Milestone 1: MVP コアループ(journey-map「MVP ジャーニー推奨」/ Phase 1)

実装: スキーマ 3 種(Profile / Context / Plan)+ 検証 3 層(§4.6)、CoreError/ErrorReport(§6)、`midi inspect/analyze/compile`、`profile validate`、`schema dump --check` の CI、Heavier7Strings のサンプル Profile、CLAUDE.md ワークフロー。

受け入れ: ベースライン MIDI 入力 → リフ Plan 生成 → コンパイル → 実 DAW にインポートし、パームミュート/ピッキングハーモニクスのキースイッチが意図通り発音する。フィードバック 1 往復で v2 が生成され、v1 が versions/ に残る。**不正な Plan を与えたとき、Agent が ErrorReport のみを頼りに 1 往復で修正できる**。

### Milestone 2: Device Profile パイプライン + マルチ楽器(Phase 2)

実装: マニュアル読解 → Profile 起草の Agent ワークフロー(CLAUDE.md)、MODO BASS/MODO DRUM/AmpliTube/Ozone の Profile、`midi decompile`(UC5)、drum_map コンパイル、`version snapshot`。

受け入れ: Agent が PDF マニュアルから起草した Profile が `profile validate` を通り、confidence が正しくマークされる。ギター+ドラム+ベースの 3 パートが同一 Context から整合的に生成される(UC3/UC4 の受け入れ基準)。

### Milestone 3: オーディオ解析 + トーン/マスタリングプラン(Phase 1〜2 完成)

実装: `audio analyze`、genre-targets、Automation Plan スキーマ(手動適用ドキュメント生成まで)、tone/master プラン生成ワークフロー(UC7〜10)。

受け入れ: ラフミックスから優先度付きレポートが出る。Ozone プランに LUFS/true peak 目標が明記され settings.json が構造化されている。

### Milestone 4: MCP 化 + 仮想 MIDI(Phase 3)

実装: `mcp serve`、control level ゲート(§2.4)、`midi send`/`panic`、actions.jsonl(tracing JSON レイヤ)、`doctor`。

受け入れ: Claude Code から MCP ツール経由で UC1 が完結する。送信の異常中断後にノートが鳴り続けない。全操作がログに残る。level 不足の呼び出しが正しく拒否され、有効化手順が案内される。

### Milestone 5: DAW 統合(Phase 4)

実装: `sora-daw` クレート(DawAdapter + capabilities)、REAPER 参照アダプタ(read/transport/clip/automation/render)、Generic アダプタ、`automation apply`、書き込み前バックアップ + WriteReceipt(§11.4)、`audio compare`。

受け入れ: UC13(トランスポート制御)と UC14 が REAPER で動く。`write_clip` 実行後、DAW 側 undo または daw-backups から完全に復元できる。アダプタ非対応操作がファイルベース経路へ明示的にフォールバックする。

### Milestone 6: 制作コパイロット(Phase 5)

実装: 役割別レビュアー(`.claude/agents/`)、Production Memory + `memory compact`、A/B 自動バウンス比較ワークフロー、North Star シナリオの E2E テスト。

受け入れ: §12.4 の 3 基準。

---

## 16. リスクと未決事項

| # | 事項 | 影響 | 対応方針 |
|---|---|---|---|
| 1 | **キースイッチ情報の実機差**: マニュアル記載と実プラグインの挙動(バージョン差・ユーザー設定)が食い違う | 生成 MIDI が無音/誤奏法になり MVP 価値が崩れる | Profile に `confidence` と検証手順を持たせ、初回は「検証用 MIDI」(全奏法を 1 音ずつ鳴らす .mid)を生成してユーザーに確認してもらうワークフローを Milestone 1 に含める |
| 2 | **オクターブ表記の不統一**(C3=60 問題) | キースイッチが 1〜2 オクターブずれる | Profile の `octave_convention` 必須化 + 検証用 MIDI で吸収 |
| 3 | **対象 DAW の確定**: journey-map の DAW 例のスクリプティング API・MIDI インポート慣習が未調査 | Phase 4 のアダプタ実装対象に影響 | アダプタ抽象(§11)は REAPER 参照実装で検証し、ユーザーの実 DAW は Milestone 5 前に調査タスクを切る。Generic アダプタが常にフォールバックになる |
| 4 | マニュアル PDF の著作権 | Profile の共有・同梱可否 | Profile はユーザーローカル生成物とし、リポジトリには実在プラグインの完全 Profile を同梱しない(examples はダミー or 実測ベースの最小構成) |
| 5 | 音楽的品質(リフが「音楽的に関連している」か)は自動テスト不能 | 受け入れ基準が主観依存 | examples/ に基準ベースラインと「良い出力」の対を蓄積し、リグレッションは人間試聴 + Plan の構造的チェック(音域・密度・ベースとのリズム相関係数)で近似 |
| 6 | Ozone/AmpliTube のプリセット/設定の自動適用経路(ファイル形式が非公開) | `suggest_plugin_settings` が手動適用止まりになる範囲 | Phase 3 までは「手動適用可能な具体性」(UC7 受け入れ基準)で十分。Phase 4 では DAW 側のジェネリックなプラグインパラメータオートメーション(§4.5 の `automation_target`)経由を優先し、ネイティブプリセット書き込みは調査後に判断 |
| 7 | **ヘッドレス VST ホスティング**(DAW なしのレンダリング)の実現性 | A/B 自動バウンス(§12.3)の自動化度 | R&D 項目としてクリティカルパスから外す。Phase 4 の DAW render で代替し、DAW 非接続時は手動レンダリング手順に縮退する |
| 8 | AbletonOSC 等サードパーティブリッジのメンテナンス状況 | Ableton アダプタの持続性 | アダプタは capabilities 申告制なので、ブリッジ劣化時は該当ケイパビリティを落として縮退運用できる。特定ブリッジへのコア依存を持たない |

---

## 17. 本書が意図的に決めていないこと

- ユーザーの実 DAW 向けアダプタの具体設計(調査タスク完了後に §11.2 へ追記)
- Device Profile のコミュニティ共有・配布形態(著作権整理後)
- ヘッドレス VST ホスティングの採否(R&D 結果待ち、§16 リスク 7)
- Phase 5 レビュアーのプロンプト詳細(Milestone 5 の成果物スキーマ確定後に設計)
