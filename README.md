# bellmux

Attention queue for tmux: notify when async work needs you, then jump back to the pane that needs it.

## What it is

A pane-addressed notification queue. Any command that can run a shell line on completion pushes a notification keyed by its tmux `pane_id`; bellmux tracks what's still waiting on you and gives you one primitive to deal with it — jump to the next pending pane.

Claude Code hooks were the original driver (every turn-end and tool-permission prompt is pushed), but the queue is producer-agnostic. Builds, test runs, deploys, CI watchers, other coding agents — anything with a pane and a shell line all fit.

## What it does for you

bellmux surfaces pending notifications and lets you cycle back to the tmux sessions and panes that raised them:

- `bellmux status` reports whether anything is pending.
  - Poll it from tmux's `status-interval` to flip the status bar colour, or to drive any other visual effect while notifications are pending:
    ```sh
    # --- bellmux status preset: fullbar ---
    # Flip the whole status bar to a notify colour while notifications are pending.
    # Two explicit colours: @bellmux-status-normal / @bellmux-status-notify.
    # Border is left untouched (tmux re-renders borders only on focus/layout
    # events, so a conditional border style would lag behind status).
    # Requires tmux >= 2.9 for #{?#(...),T,F} conditional in styles.
    set -g status-interval 2
    set -g @bellmux-status-normal 'bg=green fg=black'
    set -g @bellmux-status-notify 'bg=colour208 fg=black'
    set -g status-style '#{?#(bellmux status),#{@bellmux-status-notify},#{@bellmux-status-normal}}'
    set -g status-right '#(bellmux status --format="{n}: {latest_message}") | %H:%M '
    ```
- `bellmux next` and `bellmux prev` walk pending notifications in order and print a `pane_id`.
  - Bind a tmux key to feed the output into `switch-client -t`, and you can jump to the session:pane that produced the notification:
    ```sh
    # --- bellmux keybindings ---
    # Jump to the next pending notification in cycle order (does NOT ack).
    # First press enters at the newest pending pane, subsequent presses walk older.
    # `bellmux next` appends ` wrapped` when the cycle completes or only one
    # pane is pending — we surface that via display-message.
    # Dead panes are pruned by the pane-died hook (tmux-hook preset), so no
    # explicit fallback needed here.
    bind-key a run-shell '
      read -r pane tag <<<"$(bellmux next)"
      if [ -z "$pane" ]; then
        tmux display-message "No pending notifications"
        exit 0
      fi
      tmux switch-client -t "$pane"
      if [ "$tag" = wrapped ]; then
        tmux display-message "Cycled through all pending notifications."
      fi
    '
    ```
- `bellmux bell` writes BEL (`\x07`) to every login tty of the current user.
  - The "bell" in bellmux — alerts reach you even from another tmux session or another terminal tab. Pair it with an OS sound command for richer audio:
    ```sh
    # macOS
    afplay /System/Library/Sounds/Glass.aiff
    # Linux
    ...
    # WSL2
    ...
    ```
- `bellmux push --pane-id <%N>` records a notification against a pane (stdin accepts optional JSON with a `message` field).
  - Wire it into Claude Code hooks so every turn-end and tool-permission prompt fires a notification automatically. Merge into `~/.claude/settings.json`:
    ```json
    {
      "hooks": {
        "Notification": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux push --kind notification --pane-id \"$TMUX_PANE\" && bellmux bell"}]
        }],
        "Stop": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux push --kind stop --pane-id \"$TMUX_PANE\" && bellmux bell"}]
        }]
      }
    }
    ```
  - Or append it to any long-running command in a tmux pane (`$TMUX_PANE` is set automatically by tmux) so completion fires a notification + bell:
    ```sh
    make build; bellmux push --kind stop --pane-id "$TMUX_PANE" && bellmux bell
    gh run watch $run_id; echo '{"message":"CI done"}' \
      | bellmux push --kind notification --pane-id "$TMUX_PANE" && bellmux bell
    ```
- `bellmux ack-pane --pane-id <%N>` and `bellmux ack-all` clear pending notifications for one pane or everything.
  - Wire them into Claude Code hooks to auto-ack whenever the user resumes the session. `UserPromptSubmit` fires the moment a new prompt is typed; `PostToolUse` is the only reliable signal that the user approved a permission dialog (`PreToolUse` fires *before* the dialog, and Claude Code fires no hook at all on "Deny"):
    ```json
    {
      "hooks": {
        "UserPromptSubmit": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""}]
        }],
        "PostToolUse": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""}]
        }]
      }
    }
    ```
  - For manual ack, bind a tmux key. `tmux refresh-client -S` makes the status bar flip back immediately instead of waiting for the next `status-interval` poll:
    ```sh
    bind-key A run-shell 'bellmux ack-pane --pane-id "#{pane_id}" && tmux refresh-client -S'
    bind-key X run-shell 'bellmux ack-all && tmux refresh-client -S'
    ```

## Requirements

- Rust 1.74+ (for `cargo build`)
- tmux 3.2+ recommended (the `fullbar` status preset uses `#{?#(...),T,F}` conditional styles, available since 2.9)

Claude Code and Codex are optional — they are first-party integrations, but bellmux runs without either.

## Install

```sh
cargo build --release
ln -s "$(pwd)/target/release/bellmux" ~/.local/bin/bellmux   # or copy into any $PATH dir
```

## Quick start: Claude Code

bellmux ships with first-party hooks for Claude Code:

```sh
bellmux init --preset claude-hooks    # → ~/.claude/settings.json
bellmux init --preset fullbar         # → ~/.tmux.conf  (or widget / overlay / dot)
bellmux init --preset keybinds        # → ~/.tmux.conf
```

Then `tmux source-file ~/.tmux.conf` and restart Claude Code.

## Quick start: Codex

bellmux also ships with first-party hooks for Codex CLI:

```sh
bellmux init --preset codex-hooks     # → ~/.codex/hooks.json
bellmux init --preset fullbar         # → ~/.tmux.conf  (or widget / overlay / dot)
bellmux init --preset keybinds        # → ~/.tmux.conf
```

Then `tmux source-file ~/.tmux.conf`, restart Codex, and review/trust the new hooks from `/hooks`.

## Using with other tools

`bellmux push` is a plain CLI — any shell line inside a tmux pane can drive it. `$TMUX_PANE` is set automatically in every tmux pane's environment:

```sh
# Notify when a long-running task finishes in this pane
make build; bellmux push --kind stop --pane-id "$TMUX_PANE" && bellmux bell

# Send a custom message (surfaced in the popup and list output)
gh run watch $run_id; echo '{"message":"CI done"}' \
  | bellmux push --kind notification --pane-id "$TMUX_PANE" && bellmux bell
```

`push` only records; `bell` rings the terminal. Connect them with `&&` when you want both — or drop `bell` if the status-bar colour flip is enough. Acknowledgement is `prefix+A` by default, or `bellmux ack-pane --pane-id %N` programmatically.

## Default keybindings

After applying the `keybinds` preset:

| Binding | Action |
|---|---|
| `prefix + a` | Jump to the next pending pane (cycle older). Shows a "cycled all" message on wraparound. |
| `prefix + b` | Jump to the previous pending pane (cycle newer). |
| `prefix + A` | Ack (clear) notifications for the current pane. |
| `prefix + X` | Ack everything everywhere. |
| `prefix + N` | Open a popup listing all pending notifications. |

## CLI overview

```
bellmux push       --kind <notification|stop> --pane-id <%N>   # record a notification (stdin: optional JSON with `message`)
bellmux ack-pane   --pane-id <%N>                              # acknowledge all notifications for a pane
bellmux ack-all                                                # acknowledge everything
bellmux prune-pane --pane-id <%N>                              # alias of ack-pane (used by pane-died hook)
bellmux status     [--format <tpl>]                            # status string for tmux status-right
bellmux list       [--tsv | --json]                            # list pending notifications
bellmux next                                                   # advance the cycle cursor toward older pending panes
bellmux prev                                                   # retreat the cursor toward newer pending panes
bellmux bell                                                   # ring BEL on every login tty of the current user
bellmux init       [--preset <name>]                           # print setup snippets
```

## Snippets

`bellmux init` prints every snippet at once. Or pick one:

```sh
bellmux init --preset fullbar         # tmux status-bar style (full-bar recolor)
bellmux init --preset widget          # tmux status-bar style (right-side widget)
bellmux init --preset overlay         # tmux status-bar style (non-destructive overlay)
bellmux init --preset dot             # tmux status-bar style (single dot)
bellmux init --preset keybinds        # tmux key bindings (jump / ack)
bellmux init --preset popup-enriched  # tmux popup with session:window.pane enrichment
bellmux init --preset tmux-hook       # tmux pane-died → prune hook
bellmux init --preset claude-hooks    # Claude Code hooks for ~/.claude/settings.json
bellmux init --preset codex-hooks     # Codex hooks for ~/.codex/hooks.json
```

## How it works

Single Rust binary, no daemon. State lives in a small SQLite file that's statically linked into the binary — nothing to install beyond bellmux itself and tmux. See [`CLAUDE.md`](CLAUDE.md) for the implementation overview and [`DESIGN.md`](DESIGN.md) for the rationale, trade-offs, and history of the design decisions (especially the tmux statusbar/border experiments).

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT License ([`LICENSE-MIT`](LICENSE-MIT))

at your option.

---

# bellmux（日本語）

tmux 向け attention queue — 通知して、戻るべきペインへ案内する。

## これは何か

tmux session, pane と紐付けられた通知キューです。

通知が欲しいタイミングで bellmux のコマンドを実行すると、`pane_id` をキーに通知を push できます。

bellmux は「未対応のペインがまだ残っているか」を記録し、「次の未対応ペインへ飛ぶ」という基本的な機能を提供します。

Claude Code との協調動作が最初のモチベーション（ターン完了 / ツール許可要求のたびに push）でしたが、bellmux そのものは通知元に依存しません。

ビルド、テスト、deploy、CI watcher、他のコーディングエージェント — pane/session 情報と bellmux コマンド実行ができれば、様々なコマンドと組み合わせられます。

## 何をしてくれるか

未対応イベントの存在を可視化し、イベント発行元の tmux session, pane を巡回できます：

- `bellmux status` コマンドで通知の有無を検出します
  - tmux の status-interval でこのコマンドの出力を監視することで、ステータスバーを指定色に切り替える等、視覚効果をカスタマイズ可能です
      ```sh
      # --- bellmux status preset: fullbar ---
      # Flip the whole status bar to a notify colour while notifications are pending.
      # Two explicit colours: @bellmux-status-normal / @bellmux-status-notify.
      # Border is left untouched (tmux re-renders borders only on focus/layout
      # events, so a conditional border style would lag behind status).
      # Requires tmux >= 2.9 for #{?#(...),T,F} conditional in styles.
      set -g status-interval 2
      set -g @bellmux-status-normal 'bg=green fg=black'
      set -g @bellmux-status-notify 'bg=colour208 fg=black'
      set -g status-style '#{?#(bellmux status),#{@bellmux-status-notify},#{@bellmux-status-normal}}'
      set -g status-right '#(bellmux status --format="{n}: {latest_message}") | %H:%M '
      ```
- `bellmux next`, `bellmux prev` コマンドで未対応の通知を順に取得します
  - tmux キーバインドでこのコマンドの出力を受け取り、「未対応の通知元 tmux session:pane に移動」することができます
    ```sh
    # --- bellmux keybindings ---
    # Jump to the next pending notification in cycle order (does NOT ack).
    # First press enters at the newest pending pane, subsequent presses walk older.
    # `bellmux next` appends ` wrapped` when the cycle completes or only one
    # pane is pending — we surface that via display-message.
    # Dead panes are pruned by the pane-died hook (tmux-hook preset), so no
    # explicit fallback needed here.
    bind-key a run-shell '
      read -r pane tag <<<"$(bellmux next)"
      if [ -z "$pane" ]; then
        tmux display-message "No pending notifications"
        exit 0
      fi
      tmux switch-client -t "$pane"
      if [ "$tag" = wrapped ]; then
        tmux display-message "Cycled through all pending notifications."
      fi
    '
    ```
- `bellmux bell` コマンドを通知が必要なタイミングで呼ぶことで、現在ログイン中の tty 全てにベルを鳴らします
  - 名前の由来にもなっている機能で、別 tmux セッションや別ターミナルタブで作業していてもベルが鳴りますが、OS の音再生コマンドを使用するのがおすすめです
    ```sh
    # macOS
    afplay /System/Library/Sounds/Glass.aiff
    # Linux
    ...
    # WSL2
    ...
    ```
- `bellmux push --pane-id <%N>` コマンドで通知を記録します（stdin: 任意の JSON、`message` field を抽出）
  - Claude Code の hook と組み合わせると、ターン完了やツール許可要求のたびに自動で push + bell が発火します。`~/.claude/settings.json` の内容にマージしてください：
    ```json
    {
      "hooks": {
        "Notification": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux push --kind notification --pane-id \"$TMUX_PANE\" && bellmux bell"}]
        }],
        "Stop": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux push --kind stop --pane-id \"$TMUX_PANE\" && bellmux bell"}]
        }]
      }
    }
    ```
  - 任意の長時間コマンドに連結すれば、完了時に通知 + bell を飛ばせます（`$TMUX_PANE` は tmux が自動で入れる環境変数）：
    ```sh
    make build; bellmux push --kind stop --pane-id "$TMUX_PANE" && bellmux bell
    gh run watch $run_id; echo '{"message":"CI done"}' \
      | bellmux push --kind notification --pane-id "$TMUX_PANE" && bellmux bell
    ```
- `bellmux ack-pane --pane-id <%N>` / `bellmux ack-all` コマンドで通知を消去します
  - Claude Code の hook から呼ぶと、ユーザーが操作を再開した瞬間に自動 ack できます。`UserPromptSubmit` は新しいプロンプトが送信されたタイミング、`PostToolUse` はツール許可ダイアログで "Allow" が押された唯一の確定シグナルです（`PreToolUse` はダイアログ *前* に発火し、"Deny" の場合はそもそも hook が鳴らない）：
    ```json
    {
      "hooks": {
        "UserPromptSubmit": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""}]
        }],
        "PostToolUse": [{
          "matcher": "",
          "hooks": [{"type": "command", "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""}]
        }]
      }
    }
    ```
  - 手動 ack は tmux のキーバインドで。`tmux refresh-client -S` を付けるとステータスバーの色戻りが次の `status-interval` を待たず即時になります：
    ```sh
    bind-key A run-shell 'bellmux ack-pane --pane-id "#{pane_id}" && tmux refresh-client -S'
    bind-key X run-shell 'bellmux ack-all && tmux refresh-client -S'
    ```

## 動作要件

- Rust 1.74+（`cargo build` 用）
- tmux 3.2+ 推奨（`fullbar` プリセットが `#{?#(...),T,F}` 条件式スタイルを使用、必要バージョンは 2.9 以降）

Claude Code / Codex は optional です。first-party integration はありますが、bellmux はどちらが無くても動きます。

## インストール

```sh
cargo build --release
ln -s "$(pwd)/target/release/bellmux" ~/.local/bin/bellmux   # PATH 上の任意のディレクトリでも可
```

## クイックスタート：Claude Code

bellmux には Claude Code 用の first-party hook が同梱されています：

```sh
bellmux init --preset claude-hooks    # → ~/.claude/settings.json
bellmux init --preset fullbar         # → ~/.tmux.conf  （widget / overlay / dot でも可）
bellmux init --preset keybinds        # → ~/.tmux.conf
```

適用後、`tmux source-file ~/.tmux.conf` と Claude Code 再起動。

## クイックスタート：Codex

bellmux には Codex CLI 用の first-party hook も同梱されています：

```sh
bellmux init --preset codex-hooks     # → ~/.codex/hooks.json
bellmux init --preset fullbar         # → ~/.tmux.conf  （widget / overlay / dot でも可）
bellmux init --preset keybinds        # → ~/.tmux.conf
```

適用後、`tmux source-file ~/.tmux.conf` と Codex 再起動。Codex 側で `/hooks` から新しい hook を review / trust してください。

## 他のツールと組み合わせる

`bellmux push` は素の CLI です。tmux ペインの中から shell 一行で呼び出せます。`$TMUX_PANE` は tmux ペインの環境変数として自動で入っています：

```sh
# 長時間タスクの完了通知
make build; bellmux push --kind stop --pane-id "$TMUX_PANE" && bellmux bell

# カスタムメッセージ付き（popup や list 出力に反映される）
gh run watch $run_id; echo '{"message":"CI done"}' \
  | bellmux push --kind notification --pane-id "$TMUX_PANE" && bellmux bell
```

`push` は記録のみ、`bell` はベルを鳴らすだけ。両方欲しいときは `&&` で繋ぐ、ステータスバー色変化だけで十分なら `bell` を省く、で制御できます。ack はデフォルトで `prefix+A`、あるいは `bellmux ack-pane --pane-id %N` でスクリプト化できます。

## デフォルトキーバインド

`keybinds` プリセット適用後：

| キー | 動作 |
|---|---|
| `prefix + a` | 次の未対応ペインへジャンプ（古い方向へ巡回）。一周時に「全通知を一周しました」と表示 |
| `prefix + b` | 前の未対応ペインへジャンプ（新しい方向へ巡回） |
| `prefix + A` | 現在ペインの通知を全て ack（クリア） |
| `prefix + X` | 全 ack |
| `prefix + N` | 未対応通知一覧 popup |

## CLI 一覧

```
bellmux push       --kind <notification|stop> --pane-id <%N>   # 通知を記録（stdin: 任意の JSON、`message` field を抽出）
bellmux ack-pane   --pane-id <%N>                              # 指定ペインの通知を全 ack
bellmux ack-all                                                # 全 ack
bellmux prune-pane --pane-id <%N>                              # ack-pane の別名（pane-died hook 用）
bellmux status     [--format <tpl>]                            # tmux status-right 用ステータス文字列
bellmux list       [--tsv | --json]                            # 未対応通知一覧
bellmux next                                                   # サイクル cursor を古い方向へ進める
bellmux prev                                                   # cursor を新しい方向へ戻す
bellmux bell                                                   # 自分のログイン tty 全てに BEL 送信
bellmux init       [--preset <name>]                           # セットアップスニペット出力
```

## スニペット

`bellmux init` で全スニペットを一括出力。個別に取りたい場合：

```sh
bellmux init --preset fullbar         # ステータスバー全体を 2 色フリップ
bellmux init --preset widget          # 右端の小さなウィジェット
bellmux init --preset overlay         # 上流色を破壊しない overlay
bellmux init --preset dot             # 単一ドット
bellmux init --preset keybinds        # ジャンプ / ack のキーバインド
bellmux init --preset popup-enriched  # session:window.pane title で enrich された popup
bellmux init --preset tmux-hook       # pane-died → prune の tmux hook
bellmux init --preset claude-hooks    # Claude Code hook（~/.claude/settings.json 用）
bellmux init --preset codex-hooks     # Codex hook（~/.codex/hooks.json 用）
```

## 仕組み

Rust 単一バイナリ、常駐プロセスなし。状態は小さな SQLite ファイルに持ちますが、SQLite 自身はバイナリに静的リンクされているため、bellmux と tmux 以外にインストールするものはありません。実装の俯瞰は [`CLAUDE.md`](CLAUDE.md)、設計判断の根拠・トレードオフ・試行錯誤の経緯（特に tmux statusbar / border の挙動回り）は [`DESIGN.md`](DESIGN.md) を参照してください。

## ライセンス

Apache License 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE)) または MIT License ([`LICENSE-MIT`](LICENSE-MIT)) のデュアルライセンス。お好みの方をお選びください。
