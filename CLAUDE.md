# Sora

音楽制作のための接続されたプロダクションレイヤー。Claude Code(Agent 層)+ Rust 単一バイナリ `sora`(Tool 層)+ スキーマ定義された JSON 成果物(Data 層)で構成される。

## ドキュメント階層(上流 → 下流)

`docs/vision.md` → `docs/journey-map.md` → `docs/use-case.md` → `docs/technical-requirements.md`

- 上流は下流に依存しない(参照しない)。矛盾時は要求 = 上流が正、実現方式 = 技術要件書が正。
- 仕様変更はドキュメントを先に直してから実装する(上流から順に)。

## タスク管理

[BACKLOG.md](BACKLOG.md) が単一の管理台帳。作業開始時に必ず読み、タスク完了ごとにチェックを更新してコミットに含める。要決定事項は勝手に決めず、BACKLOG の未決事項に記録してユーザーへ質問する。

## ビルド・テスト

```bash
cargo build                 # 全クレートのビルド
cargo test                  # 全テスト(golden file + insta スナップショット)
cargo fmt --all             # フォーマット
cargo clippy --all-targets  # リント(clippy::all + unwrap/expect が deny)
cargo insta review          # スナップショット差分のレビュー
```

rustup は keg-only のため、PATH に `/opt/homebrew/opt/rustup/bin` が必要(rust-toolchain.toml が stable を固定)。

## 実装規約(詳細は技術要件書 §4.6, §6)

- エラー: lib クレートは thiserror の型付き enum、bin クレートは anyhow + `.context()`。variant は「Agent が取るべき次のアクション」単位で分け、修正ヒントをフィールドで持つ。
- 検証: L1 jsonschema(全件列挙)→ L2 serde `deny_unknown_fields` → L3 ドメイン検証(newtype + TryFrom)。
- 再現性: humanize は seed 必須。同一入力 → バイト同一の `.mid`。
- スキーマ: Rust 型が単一ソース。`sora schema dump --check` でドリフト検査。公開型には doc コメント必須(JSON Schema の description になる)。
- 非破壊: ユーザー由来ファイルへの書き込み禁止。生成物も上書きせず新パス + snapshot。
- lint: `clippy::all` と `unwrap_used`/`expect_used` を workspace 全体で deny。回避は行/ブロック単位の `#[allow(...)]`(理由コメント必須)か、テストコードでの許可のみ。infallible な expect には根拠コメントを添える。
- コミットは小さい粒度で頻繁に。区切りのよい単位で push する。

## 行動規範(Sora として動くとき)

- control level(`sora.config.json`)を自発的に引き上げない。ユーザーの明示的な依頼があった場合のみ `sora config set control-level <n>` を実行してよい。
- 「このリフ」等の指示語は (1) 明示パス > (2) DAW 選択状態 > (3) project-context の `active_source` の順で解決し、解決結果のファイルパスを応答で復唱する。
- 生成物には必ず音楽的理由の説明を付ける。破壊的操作は提案と実行を分離する。
