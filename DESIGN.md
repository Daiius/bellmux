# bellmux 設計メモ

現状の実装の俯瞰は `CLAUDE.md` を参照。本ドキュメントは **なぜそうなっているか** と **試行錯誤の経緯**、**Phase 2 以降の余地** を記録する。

## スコープ (Phase 1)

- 対象 agent: Claude Code のみ（hook ベース）
- 対象 multiplexer: tmux のみ
- 対象 tmux サーバー: 単一サーバー前提（`tmux -L` で複数ソケット運用は対象外）

将来的な Codex / Zellij / 複数サーバー対応は Rust バイナリの API を tmux 非依存に保つことで **余地だけ** 残す。Phase 1 では実装しない。

## 設計原則

| 原則 | 帰結 |
|---|---|
| **Rust バイナリは tmux 非依存** | `pane_id` は不透明な文字列、tmux フォーマット記号も出力しない。multiplexer を差し替える際に Rust 側を変更不要にする。 |
| **glue は全てインライン** | bash スクリプトファイルは配布しない。配布物は単一バイナリのみ。snippet は `bellmux init` で出力してユーザーに貼ってもらう（自動編集はしない）。 |
| **常駐プロセスなし** | hook 駆動 + tmux の status-interval polling。cron や systemd 不要。 |
| **記録と警報を分離** | DB 記録は常に行う、警報（statusbar 視覚変化）は passive poll。 |
| **Ack = 応答 or 明示削除** | UserPromptSubmit hook で自動 ack、`prefix + a` で手動 ack。jump は ack しない（見に行っただけかもしれないため）。 |
| **DELETE on ack** | スキーマ最小、history 保持なし、cleanup 不要。 |
| **enrich は表示時** | DB には `pane_id` と最小情報のみ保存、`session:window.pane title` 等は表示時に tmux へ問い合わせる（死亡ペインの自動検出も兼ねる）。 |

## 通知セマンティクス

通知 = **「ユーザー応答が必要」** な状態。ack（DELETE）された時点で「応答済み or 不要」。履歴は残さない。

Claude hook との対応：

| Hook | 意味 | 動作 |
|---|---|---|
| Notification | 権限要求 / 60秒アイドル | `push --kind notification` |
| Stop | ターン完了 | `push --kind stop` |
| UserPromptSubmit | ユーザーが応答 | `ack-pane` |

加えて `prefix + a`（現在ペイン）/ `prefix + X`（全体）で手動 ack。

## tmux statusbar / border: 試行錯誤の経緯

当初の `fullbar` preset は **statusbar と pane-active-border の両方** を通知色にフリップする設計だった。実装時に以下の問題が順次発覚し、最終的に **border フリップは諦めて statusbar のみ 2 色明示フリップ** に落ち着いた。

### 1. 上流スタイルの保存 (snapshot 方式) の脆さ

ユーザーが既に `status-style` をカスタムしている前提で、通知色 → 通常色の戻し先を保つため、初回ロード時に `run-shell` で `@bellmux-status-style-normal` に退避する snapshot パターンを採用した。

→ 複数回 `tmux source-file ~/.tmux.conf` した際、2 回目の snapshot が **既に条件式化された値** を「通常値」として捕捉し、自己参照ループ（`@bellmux-status-style-normal = #{?...,T,#{@bellmux-status-style-normal}}`）に陥る事故が発生。idempotency ガードを入れれば回避できるが、根本的に脆い。

### 2. border の更新タイミング

`status-style` 内の `#(bellmux status)` は `status-interval` (2s) で再評価されるが、`pane-active-border-style` 内の `#(...)` は **border の再描画イベント** (focus 切替、resize、layout 変更) でしか更新されない。

通知発生と同時に両方を flip させるため Claude hook 末尾に `&& tmux refresh-client && tmux set -g pane-active-border-style "$(tmux show -gv pane-active-border-style)"` を追加したが、

- `-S` の有無で挙動が変わる（`-S` は status-line のみ refresh、border は動かない）
- hook チェーンが増えるほど race condition が出やすい（statusbar と border の色が瞬間的にズレる）
- Claude hook / tmux ack bind / 手動 source-file の 3 パスで微妙に挙動が違い、デバッグが困難

### 3. 結論

- **border フリップは廃止**。`pane-active-border-style` は tmux デフォルトに戻す。
- statusbar は `@bellmux-status-normal` / `@bellmux-status-notify` の 2 つの user option を明示宣言し、条件式で切り替える。snapshot ロジック廃止。
- Claude hook は `bellmux ...` 呼び出しのみ（refresh は status-interval poll に任せる）。
- 手動 ack bind は `tmux refresh-client -S` で即時反映（border は動かさないので `-S` で十分）。

## セキュリティ脅威モデル

### 守る

- **SQL injection**: parameter binding 必須
- **pane_id injection**: `^%[0-9]+$` で検証（`validate::pane_id`）
- **JSON injection**: `serde_json::Value` で安全に parse、`message` field のみ抽出
- **TSV / 表示層 injection**: `sanitize_message` で tab / CR / LF を空白置換

### 禁止

- 通知 message を eval / shell コマンドとして実行しない
- `run-shell` に DB 由来の文字列を直接渡さない（`pane_id` は検証済み正規表現に合致するもののみ）

## エラーコード

| Code | 意味 |
|---|---|
| 0 | 正常終了。stdin が空 / 非 JSON でも `message=NULL` で INSERT する（寛容 parse）。 |
| 1 | 実行時エラー（DB ロック超過、IO エラー、disk full 等）。stderr に warn。 |
| 2 | 引数エラー（`pane_id` 不正形式、未知の `--kind` 値等）。 |

push 失敗時の影響範囲: Claude hook は `&&` 連結を使わない素朴構成なので、push 失敗でも Claude Code の動作は止まらない。次の status-interval poll で state は整合する。

## Phase 2 以降の余地

設計上は対応の余地があるが、Phase 1 では実装しない：

| 項目 | 備考 |
|---|---|
| Codex 等他 agent 対応 | hook が無いので polling daemon が別途必要 |
| Zellij / screen 対応 | snippet 追加のみ、Rust 側は無変更のはず |
| 複数 tmux サーバー対応 | `(socket_path, pane_id)` 複合キー化が必要 |
| `pane-focus-in` hook 連携 | `focus-events on` 必須、ターミナルアプリレベルのフォーカス検出 |
| Popup での fzf 等 interactive 選択 | fzf 依存が必要 |
| macOS native 通知 (alerter) | 視覚警報で十分なら不要 |
| 古い通知の自動 cleanup | DELETE-on-ack で自然に小さく保たれるため不要 |
| jq ベースの `list --json` wrapper | TSV で代替可能 |

## 未確定事項

- **macOS の DB パス**: `dirs::state_dir()` が macOS で `None` を返すため `data_local_dir()` にフォールバックし、実体は `~/Library/Application Support/bellmux/` に落ちる。設計上は XDG の `~/.local/state/` を想定していたので、将来どちらに統一するか判断が必要。
- **`fullbar` の tmux バージョン要件**: `status-style` に `#{?#(...),T,F}` の条件式を使うので tmux >= 2.9 が必要。README に明記要。
- **hook 実行時間**: 設計上は < 100ms 想定。実測で問題が出たら CLI を非同期化する余地あり。
