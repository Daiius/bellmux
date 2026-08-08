# bellmux

Coding-agent hook, tmux, SQLite の 3 点を結ぶ最小限の通知レイヤ。単一 Rust バイナリ、常駐プロセスなし。

設計判断の背景・Phase 2 案は `DESIGN.md` を参照。

## ファイル構成

| File | 役割 |
|---|---|
| `src/main.rs` | CLI エントリ（clap derive）。サブコマンドを `cmd_*` にディスパッチ、`push` 用 stdin JSON パース（top-level `message` のみ抽出、表示用）。 |
| `src/db.rs` | SQLite open (WAL + busy_timeout=3s)、`notifications` / `meta` / `holds` テーブルの CRUD、cursor accessor、hold accessor（`set_hold` / `clear_hold` / `clear_all_holds`）、`ordered_panes` / `next_pane` / `prev_pane`、`StatusSnapshot`、`relative_time`、`sanitize_message`、DB パス解決。 |
| `src/format.rs` | `status --format` のテンプレートエンジン。`{n}` / `{latest_message}` / `{latest_pane}`。`n==0` なら空文字列を返す（tmux 条件式の false 判定に使う）。 |
| `src/validate.rs` | `pane_id` 検証（不透明キーとして `[A-Za-z0-9%_:./-]`・非空・64 文字以内。tmux `%5` / zellij `terminal_5`・`5` 等を許容、空白・`;`・制御文字は拒否）。 |
| `src/snippets.rs` | 埋め込みスニペット（`bellmux init` 出力元）。preset: `widget` / `fullbar` / `overlay` / `dot` / `popup-simple` / `popup-enriched` / `keybinds` / `tmux-hook` / `claude-hooks` / `codex-hooks`。 |

## データフロー

```
Claude Code hook (Notification / Stop / UserPromptSubmit / PostToolUse / PostToolUseFailure / SessionEnd)
or Codex hook (PermissionRequest / Stop / UserPromptSubmit / PostToolUse / SessionStart)
  → inline command: bellmux push && bellmux bell  |  bellmux ack-pane [&& bellmux hold]
  → INSERT / DELETE (SQLite, WAL)
        ↑ poll every status-interval (2s)   ← hold 中のペインは read 時に除外
        │
  tmux #(bellmux status ...) in status-right / status-style
```

Border style は意図的にフリップしない。tmux は border を focus/layout イベントでしか再描画しないため、条件式でフリップさせると statusbar とズレる。詳しくは `DESIGN.md` の試行錯誤の項。

## CLI

```
bellmux push       --kind <notification|stop> --pane-id <%N>    # stdin: 任意の JSON（top-level `message` のみ抽出、非 JSON でも OK）。記録のみ、bell は鳴らさない（呼び出し側で `&& bellmux bell` を連結）。受け取った通知は常に記録する（surface 対象の選別は hook matcher が担うため push 側に suppress 判定は無い）
bellmux ack-pane   --pane-id <%N>                               # そのペインの通知を全 DELETE
bellmux ack-all                                                  # 全通知を DELETE
bellmux prune-pane --pane-id <%N>                                # ack-pane と同じ動作、pane-died hook 用の別名
bellmux hold       --pane-id <%N> [--ttl <secs>]                 # そのペインを self-driving としてマーク。lease 既定 1800 秒（上限 86400）。既存 hold があれば期限を上書き更新
bellmux unhold     --pane-id <%N>                                # hold を解除。ack-pane / ack-all / prune-pane も解除する
bellmux status     [--format <tpl>] [--only-pane <%N>]           # 未対応 0 なら常に空文字列。hold 中のペインは数えない。--only-pane でそのペインだけに絞る（pending 無ければ空＝「今いるペインが待ちか」を statusbar で判定可能）
bellmux list       [--tsv | --json]                              # デフォルトは人間可読。hold 中の行は human 出力で `kind [held]`、`--json` で `"held": true`（`--tsv` は 4 列固定のまま）
bellmux next       [--current <%N>]                              # サイクル cursor を 1 つ古い方向へ進めて返す。cursor 無ければ最新。一周時は ` wrapped` を付ける。--current 指定時、移動先が現在ペインなら 1 つ先へスキップ（他に pending があれば）。現在ペインが唯一の pending ならそのまま返し ` wrapped` を付ける
bellmux prev       [--current <%N>]                              # cursor を 1 つ新しい方向へ戻して返す。cursor 無ければ最古。--current の挙動は next と対称
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
  CREATE TABLE holds (
    pane_id    TEXT PRIMARY KEY,
    expires_at TEXT NOT NULL   -- RFC3339 UTC。now_iso8601() と同一書式なので文字列比較で時系列比較できる
  );
  ```
- Ack = `DELETE FROM notifications WHERE pane_id = ?`。`acked_at` カラムは持たない（history は保持しない）。
- 全テーブルは `CREATE TABLE IF NOT EXISTS` を open ごとに流すので、既存 DB には `holds` が自動で足される（マイグレーション不要）。

## Hold（self-driving ペイン）

**Stop は「ターンが終わった」であって「ユーザーが必要」ではない。** エージェントがバックグラウンドコマンドを起動してターンを終え待機に入ると、その待機の間ずっとペインが「あなた待ち」として表示される。完了→自動再開→次の待機、で毎周これが起きる。hold はこのズレを吸収するための状態。

- **意味**: 「このペインは自分で起動した機械仕掛けを待っている（＝ユーザー待ちではない）」。
- **read 時フィルタである**。`push` は従来通り**必ず記録する**（「push は受け取った通知を必ず記録する」不変条件は不変）。hold が抑えるのは *surface* だけ。
- 対象は `status`（`--only-pane` 含む）と `ordered_panes`（→ `next` / `prev`）。`list` は**除外しない**：抑制中の通知が一覧からも消えると「statusbar が静か」と「キューが空」を区別できなくなるため、`held` フラグを付けて出す。
- **lease（TTL）で自動失効する**。hold が漏れたときに倒れる方向は「通知が遅れる」であって「通知が消える」ではない。既定 1800 秒。
- **`ack-pane` / `ack-all` / `prune-pane` が hold を解放する**。ack は「ユーザーが介入した」か「ペインが実作業に戻った」のどちらかを意味し、どちらでも self-driving ではなくなる。これが「hold が、その根拠となった待機より長生きしない」ことを保証する主要経路であり、TTL はあくまで backstop。
- cursor が hold されたペインを指していた場合は `ordered_panes` に現れないので、既存の stale cursor 処理（先頭から再入）がそのまま効く。

## Cursor（`next` / `prev` 用）

サイクル用の cursor を `meta` テーブルに `key='cursor'` で 1 行保存。挙動の不変条件：

- ペイン順序は `MIN(id)` per pane の DESC（「最初にキューに入った順」の逆、FIFO 互換。再通知でペイン位置は shuffle しない）。
- `push` は cursor に触らない。`ack-pane` / `prune-pane` が cursor のペインを消したときのみ cursor も null に。`ack-all` でも null に。
- `next` / `prev` 呼出時、cursor が null または現在の pending に居なければ entry（`next`=最新、`prev`=最古）。有効なら 1 つ進めて wrap。
- 観測的には pending 非空の間 `next` / `prev` は常に有効な pane_id を返す。
- `--current <%N>`（keybind が `#{pane_id}` を渡す）指定時、cursor 計算結果が現在ペインに一致したら 1 ステップだけ余分に進めて現在ペインへの no-op ジャンプを避ける（pane_id はユニークなのでスキップは高々 1 回）。現在ペインが唯一の pending の場合のみスキップせずそのペインを返し `wrapped=true` にする（行き先が無い＝一周の合図）。`--current` 無し（None）なら従来挙動と完全一致。

## tmux / Claude 連携

`bellmux init --preset=<name>` で各スニペットを出力。`init` 単体で全 preset + Claude hooks の一括ダンプ。

- **statusbar preset**: `widget`（右端の小さな塊）/ `fullbar`（bar 全体をフリップ）/ `overlay`（上流色を破壊しない）/ `dot`（単一文字）。pending 有無で色を出すのに加え、`#(bellmux status --only-pane #{pane_id} --format here)` で「現ペインが待ちか」を判定し `🔔` バッジで示す（他ペインのみ pending とは区別。全 bar を別色にフリップするのは目に痛いので色ではなくバッジで表現）。tmux は `#()` 内の `#{pane_id}` を client ごとに展開するので、active pane が pending 集合に含まれるときだけ probe が非空になる
- **popup preset**: `popup-simple`（`list | less`）/ `popup-enriched`（TSV + `tmux display-message` で `session:window.pane title` に enrich）
- **keybinds**: `prefix+a` `next` ジャンプ（最頻動作なので小文字）、`prefix+b` `prev` ジャンプ（逆方向）、`prefix+A` 現在ペイン ack、`prefix+X` 全 ack。ack 系は `tmux refresh-client -S` で即時反映
- **tmux-hook**: `pane-died` → `prune-pane`
- **claude-hooks**: `~/.claude/settings.json` に貼る JSON。Notification は matcher を `permission_prompt|elicitation_dialog` に絞り（surface 対象の選別は hook matcher が担う。idle ping 等は matcher 不一致で hook 自体が発火しない）、Stop と共に `bellmux push ... && bellmux bell`、UserPromptSubmit / **PostToolUse** / **PostToolUseFailure** / **SessionEnd** は `bellmux ack-pane ...`。`push` は記録のみで bell を鳴らさず、成功時のみ後段が走る。この分離で後段を `afplay` / `terminal-notifier` / `osascript` 等のカスタム通知手段に差し替えられる。PostToolUse ack は permission dialog で "Allow" を押した後の唯一の確定シグナル（PreToolUse はダイアログ前に発火、"Deny" はフックなし）。Claude Code はツール完了を成功＝PostToolUse / 失敗＝PostToolUseFailure の 2 イベントに分けるため**両方**で ack する（"Allow" 直後にツールが失敗すると PostToolUse は発火せず通知が残るため）。SessionEnd ack は `/clear`・logout・Claude 終了でペインが生き残るケースの通知ゴーストを防ぐ（`pane-died` はペインが実際に閉じた時しか発火しないため）。全コマンドの先頭に `[ -n "$TMUX_PANE" ] || exit 0;` ガードを置き、tmux 外で起動したセッションでは hook を静かに no-op にする（空 `--pane-id` での `invalid pane_id` エラーがターンごとに出るのを防ぐ）。ガードを bellmux 本体でなく snippet 側に置くのは、pane_id 検証は境界で厳格に保ちつつ「通知すべきペインが無い」判断は glue の責務だという切り分けによる。**PostToolUse は ack に加えて hold も張る**: payload の `tool_name == "Bash" && tool_input.run_in_background == true` を `jq -e` で判定し、真なら `bellmux hold`。ack が先に走って旧 hold を落とし、その直後に「今起動した待機」に対する hold を張り直すので、1 コマンド内で順序が確定する（同一イベントに hook を 2 本並べると実行順が保証されない）。`jq` が無い環境では hold 段を丸ごとスキップして hold 導入前の挙動に落ちる。判定を文字列 grep でなく `jq` にしているのは、`command` 引数の中にたまたま `"run_in_background":true` を含むツール呼び出し（まさにこの機能の開発中に起きる）で誤爆させないため。
- **codex-hooks**: `~/.codex/hooks.json` に貼る JSON。PermissionRequest / Stop は固定の短い `{"message": ...}` を pipe してから `bellmux push ... && bellmux bell`、UserPromptSubmit / PostToolUse / SessionStart(startup|resume|clear) は `bellmux ack-pane ...`。Codex は command hook に入力プロンプト等を含み得る JSON を stdin で渡すため、その payload は bellmux に読ませない。Codex には現時点で Claude Code の SessionEnd 相当 hook がないため、SessionStart で stale 通知を掃除する。claude-hooks と同じく全コマンド先頭に `[ -n "$TMUX_PANE" ] || exit 0;` ガードを置く。

statusbar の refresh は tmux の status-interval poll に任せる（素朴・stable）。bell は push 同期、statusbar は次のポーリングなので最大 status-interval 秒のズレはあり得る（許容）。`bellmux bell` は tmux 非依存で全クライアントの outer tty に直接 BEL を送るため、別セッションで作業中でも気付ける

`fullbar` は `@bellmux-status-normal` / `@bellmux-status-notify` の 2 つの user option を tmux.conf 先頭で明示宣言する方式。上流値を snapshot 取得する方式は多重 `source-file` で自己参照ループのリスクがあり廃止した（`DESIGN.md` 参照）。

## 不変条件

- **Rust バイナリは tmux 非依存**: `pane_id` は不透明な文字列キー、tmux フォーマット記号は出力しない。
- **glue は全て snippet 埋め込み**: bash スクリプトファイルは配布しない。
- **入力検証は境界で**: `pane_id` は不透明キーとして `[A-Za-z0-9%_:./-]`・非空・64 文字以内に限定（multiplexer 非依存。tmux `%5` も zellij `terminal_5`・`5` も通す）、SQL は常に parameter binding、`message` は全制御文字（tab/CR/LF に加え ESC/BEL 等）を空白置換し、tmux status bar / `list` 出力へのターミナルエスケープシーケンス注入を防ぐ（`sanitize_message`）。
- **未対応 0 なら status 出力は空**: `format::render` は `n==0` で template に関わらず空文字列を返す → tmux 条件式 `#{?#(bellmux status),T,F}` の F 側が選ばれ、statusbar が通常色に戻る。
- **抑制は read 時のみ、記録は必ず行う**: hold は `status` / `next` / `prev` から除外するだけで、`push` は常に INSERT する。抑制は必ず失効時刻を持ち、失効すれば表面化する（fail-visible）。「通知を出さない」判断を書き込み側に持たせない。
- **エージェントのイベント体系を知るのは snippet だけ**: `tool_name` や `run_in_background` の解釈は hook snippet の `jq` 側に閉じる。`hold` の CLI は「このペインを surface するな」しか知らない。

## 依存クレート

`Cargo.toml`:

- `rusqlite` (bundled) — SQLite を静的リンク
- `clap` (derive) — CLI パース
- `serde_json` — Claude payload 解析
- `chrono` — RFC3339 UTC
- `anyhow` — CLI エラー
- `dirs` — XDG / macOS ディレクトリ解決

release profile: `opt-level=3`, `lto=thin`, `strip=symbols`。
