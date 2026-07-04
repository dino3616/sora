# Example: Heavier7Strings リフ生成(MVP コアループ)

journey-map の「MVP ジャーニー」を再現する最小の例。ベースラインに合うモダンメタルの
ギターリフを Part Plan として設計し、決定論的に `.mid` へコンパイルする。

## 構成

- `sora.config.json` — 環境設定(Studio One 5 / Heavier7Strings、control level 1)
- `devices/heavier7strings.profile.json` — Device Profile のテンプレート
  （**keyswitch は `unverified`。実機確認前提**）
- `exports/guitar-riff-v1.plan.json` — レビュー可能な Part Plan(IR)

## 手順

プロジェクトルートでバイナリをビルド:

```bash
cargo build --release
SORA=../../target/release/sora   # examples/heavier7strings-riff から見たパス
```

1. **Profile を検証**

   ```bash
   $SORA profile validate devices/heavier7strings.profile.json
   ```

   `unverified_articulations` に全奏法が挙がる。次のステップで実機確認する。

2. **検証用 MIDI を生成 → Studio One で実機確認**

   ```bash
   $SORA profile verify-midi devices/heavier7strings.profile.json
   ```

   出力された `devices/heavier7strings.verify.mid` を Studio One に読み込み、
   Heavier7Strings で再生。各 `bar` で期待どおりの奏法が鳴るか確認し、正しければ
   Profile の該当 `confidence` を `verified` に更新する（誤っていればキースイッチの
   ノート番号か `octave_convention` を修正）。

3. **リフをコンパイル**

   ```bash
   $SORA midi compile exports/guitar-riff-v1.plan.json
   ```

   `exports/guitar-riff-v1.mid` が生成される。Studio One にインポートして試聴する。
   Profile が未検証のうちは `UNVERIFIED_ARTICULATION` 警告が出る（想定どおり）。

4. **フィードバック → v2**

   Plan JSON を編集（例: ノート密度を下げてボーカルの余白を増やす）して別名の
   `guitar-riff-v2.plan.json` にし、再コンパイル。`sora version snapshot v1` で
   `versions/v1/` に旧版を退避してから進めると差分をレビューしやすい。

## ポイント

- **Plan がレビュー単位**。`.mid` は Plan から常に再生成できるため、JSON を読めば
  「何を意図した音か」がわかる（`design_notes` と各ノートの `articulation`）。
- **決定論的**。同じ Plan + Profile + `humanize.seed` からは常にバイト同一の `.mid`。
- Profile の `confidence` が `unverified` の間はコンパイルレポートに警告が出る。
  実機確認して `verified` にすると警告は消える。
