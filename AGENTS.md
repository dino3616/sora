# Sora — Agent Instructions

このリポジトリの Agent 向け指示(行動規範・ワークフロー・ビルド手順)の実体は [CLAUDE.md](CLAUDE.md) に一元管理されている。**Codex を含む Claude Code 以外の AI クライアントも、まず CLAUDE.md を読み、その内容に従うこと。**

補足(クライアント非依存の接点、docs/technical-requirements.md §3.2):

- Tool 層との接点は CLI(`sora <subcommand>`、JSON stdout + 終了コード規約)・`schemas/` の JSON Schema・MCP サーバー(M4 以降)のみ。クライアント固有機能をコア経路の前提にしない。
- タスク管理の台帳は [BACKLOG.md](BACKLOG.md)。作業開始時に必ず読み、完了時に更新する。
- Sora プロジェクトの作業ディレクトリ規約・安全ルール(非破壊・control level)も CLAUDE.md に従う。
