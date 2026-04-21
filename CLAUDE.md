# bellmux

Claude Code hook, tmux, SQLite の 3 点を結ぶ最小限の通知レイヤ。単一 Rust バイナリ、常駐プロセスなし。

設計判断の背景・Phase 2 案は `DESIGN.md` を参照。

## ファイル構成

| File | 役割 |
|---|---|
| `src/main.rs` | CLI エントリ（clap derive）。サブコマンドを `cmd_*` にディスパッチ、`push` 用 stdin JSON パース。 |
| `src/db.rs` | SQLite open (WAL + busy_timeout=3s)、`notifications` / `meta` テーブルの CRUD、cursor accessor、`ordered_panes` / `next_pane` / `prev_pane`、`StatusSnapshot`、`relative_time`、`sanitize_message`、DB パス解決。 |
| `src/format.rs` | `status --format` のテンプレートエンジン。`{n}` / `{latest_message}` / `{latest_pane}`。`n==0` なら空文字列を返す（tmux 条件式の false 判定に使う）。 |
| `src/validate.rs` | `pane_id` 検証 (`^%[0-9]+$`)。 |
| `src/snippets.rs` | 埋め込みスニペット（`bellmux init` 出力元）。preset: `widget` / `fullbar` / `overlay` / `dot` / `popup-simple` / `popup-enriched` / `keybinds` / `tmux-hook` / `claude-hooks`。 |

## データフロー

```
Claude Code hook (Notification / Stop / UserPromptSubmit)
  → inline command: bellmux push|ack-pane --pane-id $TMUX_PANE
  → INSERT / DELETE (SQLite, WAL)
        ↑ poll every status-interval (2s)
        │
  tmux #(bellmux status ...) in status-right / status-style
```

Border style は意図的にフリップしない。tmux は border を focus/layout イベントでしか再描画しないため、条件式でフリップさせると statusbar とズレる。詳しくは `DESIGN.md` の試行錯誤の項。

## CLI

```
bellmux push       --kind <notification|stop> --pane-id <%N>    # stdin: 任意の JSON（`message` field のみ抽出、非 JSON でも OK）。成功時は内部で bell も鳴る。idle notification（message に "waiting for your input" を含む）は記録もベルも skip
bellmux ack-pane   --pane-id <%N>                               # そのペインの通知を全 DELETE
bellmux ack-all                                                  # 全通知を DELETE
bellmux prune-pane --pane-id <%N>                                # ack-pane と同じ動作、pane-died hook 用の別名
bellmux status     [--format <tpl>]                              # 未対応 0 なら常に空文字列
bellmux list       [--tsv | --json]                              # デフォルトは人間可読
bellmux next                                                      # サイクル cursor を 1 つ古い方向へ進めて返す。cursor 無ければ最新。一周時は ` wrapped` を付ける
bellmux prev                                                      # cursor を 1 つ新しい方向へ戻して返す。cursor 無ければ最古
bellmux bell                                                      # `who` で取得した自分のログイン tty 全てに BEL (\x07) を書込む
bellmux init       [--preset <name>]                             # tmux/hook スニペット出力
```

## データベース

- パス: `${BELLMUX_DB_PATH:-${XDG_STATE_HOME:-~/.local/state}/bellmux/notifications.db}`
  - macOS は `dirs::state_dir()` が `None` を返すので `~/Library/Application Support/bellmux/notifications.db` にフォールバック
- PRAGMA: `journal_mode=WAL`, `busy_timeout=3000`
- スキーマ:
  ```sql
  CREATE TABLE notifications (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,  -- RFC3339 UTC
    pane_id    TEXT NOT NULL,
    kind       TEXT NOT NULL,  -- "notification" | "stop"
    message    TEXT
  );
  CREATE INDEX idx_pane ON notifications(pane_id);
  CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);  -- cursor 等
  ```
- Ack = `DELETE FROM notifications WHERE pane_id = ?`。`acked_at` カラムは持たない（history は保持しない）。

## Cursor（`next` / `prev` 用）

サイクル用の cursor を `meta` テーブルに `key='cursor'` で 1 行保存。挙動の不変条件：

- ペイン順序は `MIN(id)` per pane の DESC（「最初にキューに入った順」の逆、FIFO 互換。再通知でペイン位置は shuffle しない）。
- `push` は cursor に触らない。`ack-pane` / `prune-pane` が cursor のペインを消したときのみ cursor も null に。`ack-all` でも null に。
- `next` / `prev` 呼出時、cursor が null または現在の pending に居なければ entry（`next`=最新、`prev`=最古）。有効なら 1 つ進めて wrap。
- 観測的には pending 非空の間 `next` / `prev` は常に有効な pane_id を返す。

## tmux / Claude 連携

`bellmux init --preset=<name>` で各スニペットを出力。`init` 単体で全 preset + Claude hooks の一括ダンプ。

- **statusbar preset**: `widget`（右端の小さな塊）/ `fullbar`（bar 全体を 2 色フリップ）/ `overlay`（上流色を破壊しない）/ `dot`（単一文字）
- **popup preset**: `popup-simple`（`list | less`）/ `popup-enriched`（TSV + `tmux display-message` で `session:window.pane title` に enrich）
- **keybinds**: `prefix+a` `next` ジャンプ（最頻動作なので小文字）、`prefix+b` `prev` ジャンプ（逆方向）、`prefix+A` 現在ペイン ack、`prefix+X` 全 ack。ack 系は `tmux refresh-client -S` で即時反映
- **tmux-hook**: `pane-died` → `prune-pane`
- **claude-hooks**: `~/.claude/settings.json` に貼る JSON。Notification / Stop は `bellmux push ...`、UserPromptSubmit は `bellmux ack-pane ...`。`push` 内で bell も発火（ただし idle notification は suppress）。statusbar の refresh は tmux の status-interval poll に任せる（素朴・stable）。`bellmux bell` は tmux 非依存で全クライアントの outer tty に直接 BEL を送るため、別セッションで作業中でも気付ける

`fullbar` は `@bellmux-status-normal` / `@bellmux-status-notify` の 2 つの user option を tmux.conf 先頭で明示宣言する方式。上流値を snapshot 取得する方式は多重 `source-file` で自己参照ループのリスクがあり廃止した（`DESIGN.md` 参照）。

## 不変条件

- **Rust バイナリは tmux 非依存**: `pane_id` は不透明な文字列キー、tmux フォーマット記号は出力しない。
- **glue は全て snippet 埋め込み**: bash スクリプトファイルは配布しない。
- **入力検証は境界で**: `pane_id` は `^%[0-9]+$`、SQL は常に parameter binding、`message` は tab/CR/LF を空白置換（`sanitize_message`）。
- **未対応 0 なら status 出力は空**: `format::render` は `n==0` で template に関わらず空文字列を返す → tmux 条件式 `#{?#(bellmux status),T,F}` の F 側が選ばれ、statusbar が通常色に戻る。

## 依存クレート

`Cargo.toml`:

- `rusqlite` (bundled) — SQLite を静的リンク
- `clap` (derive) — CLI パース
- `serde_json` — Claude payload 解析
- `chrono` — RFC3339 UTC
- `anyhow` — CLI エラー
- `dirs` — XDG / macOS ディレクトリ解決

release profile: `opt-level=3`, `lto=thin`, `strip=symbols`。
