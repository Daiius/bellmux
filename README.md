# bellmux

Minimal notification layer that bridges Claude Code hooks, tmux, and SQLite. Single Rust binary, no daemon.

When a Claude Code session in a tmux pane needs your attention (turn finished, tool permission requested), bellmux records it and surfaces it via:

- A coloured tmux status bar that lights up while anything is pending
- A terminal bell delivered to your active login ttys (so it reaches you across tmux sessions and terminal tabs)
- Keybindings to cycle through every pane that's waiting on you

## Status

Pre-1.0, personal project. Phase 1 scope: single tmux server, Claude Code only. The CLI surface and snippet output may change. Issues and PRs welcome but not actively solicited.

## Requirements

- Rust 1.74+ (for `cargo build`)
- tmux 3.2+ recommended (the `fullbar` status preset uses `#{?#(...),T,F}` conditional styles, available since 2.9)
- Claude Code CLI for the hook integration

## Install

```sh
cargo build --release
ln -s "$(pwd)/target/release/bellmux" ~/.local/bin/bellmux   # or copy into any $PATH dir
```

## Setup

Print all the snippets bellmux ships with:

```sh
bellmux init
```

Or pick one preset at a time:

```sh
bellmux init --preset fullbar         # tmux status-bar style (full-bar recolor)
bellmux init --preset widget          # tmux status-bar style (right-side widget)
bellmux init --preset overlay         # tmux status-bar style (non-destructive overlay)
bellmux init --preset dot             # tmux status-bar style (single dot)
bellmux init --preset keybinds        # tmux key bindings (jump / ack)
bellmux init --preset popup-enriched  # tmux popup with session:window.pane enrichment
bellmux init --preset tmux-hook       # tmux pane-died → prune hook
bellmux init --preset claude-hooks    # Claude Code hooks for ~/.claude/settings.json
```

Paste the relevant blocks into `~/.tmux.conf` (then `tmux source-file ~/.tmux.conf`) and `~/.claude/settings.json`. Restart Claude Code so it picks up the new hooks.

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

## How it works

See [`CLAUDE.md`](CLAUDE.md) for the implementation overview and [`DESIGN.md`](DESIGN.md) for the rationale, trade-offs, and history of the design decisions (especially the tmux statusbar/border experiments).

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT License ([`LICENSE-MIT`](LICENSE-MIT))

at your option.

---

# bellmux（日本語）

Claude Code hook、tmux、SQLite の 3 点を結ぶ最小構成の通知レイヤ。Rust 単一バイナリ、常駐プロセスなし。

tmux のペインで動いている Claude Code が応答を待ち始めた瞬間（ターン完了、ツール権限要求）を、bellmux が捕捉して以下で可視化します：

- 未対応がある間 tmux ステータスバーが指定色に切り替わる
- ユーザーの現在ログイン中の tty 全てにベルを鳴らす（別 tmux セッションや別ターミナルタブで作業していても気付ける）
- 未対応ペインを順に巡回するキーバインド

## 開発状況

Pre-1.0、個人プロジェクトです。Phase 1 のスコープは「単一 tmux サーバー / Claude Code のみ」。CLI と snippet 出力は変わる可能性あり。Issue / PR は歓迎しますが積極的な募集はしていません。

## 動作要件

- Rust 1.74+（`cargo build` 用）
- tmux 3.2+ 推奨（`fullbar` プリセットが `#{?#(...),T,F}` 条件式スタイルを使用、必要バージョンは 2.9 以降）
- Claude Code CLI（hook 連携用）

## インストール

```sh
cargo build --release
ln -s "$(pwd)/target/release/bellmux" ~/.local/bin/bellmux   # PATH 上の任意のディレクトリでも可
```

## セットアップ

同梱の全スニペットを出力：

```sh
bellmux init
```

プリセット単体で取り出すこともできます：

```sh
bellmux init --preset fullbar         # ステータスバー全体を 2 色フリップ
bellmux init --preset widget          # 右端の小さなウィジェット
bellmux init --preset overlay         # 上流色を破壊しない overlay
bellmux init --preset dot             # 単一ドット
bellmux init --preset keybinds        # ジャンプ / ack のキーバインド
bellmux init --preset popup-enriched  # session:window.pane title で enrich された popup
bellmux init --preset tmux-hook       # pane-died → prune の tmux hook
bellmux init --preset claude-hooks    # Claude Code hook（~/.claude/settings.json 用）
```

該当ブロックを `~/.tmux.conf` に貼って `tmux source-file ~/.tmux.conf`、Claude Code 用は `~/.claude/settings.json` に貼って Claude Code を再起動してください。

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

## 仕組み

実装の俯瞰は [`CLAUDE.md`](CLAUDE.md)、設計判断の根拠・トレードオフ・試行錯誤の経緯（特に tmux statusbar / border の挙動回り）は [`DESIGN.md`](DESIGN.md) を参照してください。

## ライセンス

Apache License 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE)) または MIT License ([`LICENSE-MIT`](LICENSE-MIT)) のデュアルライセンス。お好みの方をお選びください。
