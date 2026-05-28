# bellmux 設計メモ

現状の実装の俯瞰は `CLAUDE.md` を参照。本ドキュメントは **なぜそうなっているか** と **試行錯誤の経緯**、**Phase 2 以降の余地** を記録する。

## スコープ (Phase 1)

- 対象 agent: Claude Code / Codex（hook ベース）
- 対象 multiplexer: tmux のみ
- 対象 tmux サーバー: 単一サーバー前提（`tmux -L` で複数ソケット運用は対象外）

将来的な Zellij / 複数サーバー対応は Rust バイナリの API を tmux 非依存に保つことで **余地だけ** 残す。Phase 1 では実装しない。

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
| Notification | 権限要求 / 60秒アイドル | `push --kind notification && bellmux bell` |
| Stop | ターン完了 | `push --kind stop && bellmux bell` |
| UserPromptSubmit | ユーザーが応答 | `ack-pane` |
| PostToolUse | ツール実行完了（成功） | `ack-pane` |
| PostToolUseFailure | ツール実行完了（失敗） | `ack-pane` |
| SessionEnd | セッション終了（/clear, logout, 終了等） | `ack-pane` |

加えて `prefix + a`（現在ペイン）/ `prefix + X`（全体）で手動 ack。

SessionEnd でも ack する: `/clear`・logout・Claude の終了でセッションが終わってもペインは生き残ることがあり、その間 Stop 等の通知が残留する。`pane-died` hook はペインが実際に閉じた時しか発火しないため取りこぼす。SessionEnd で締めることで「セッションが終わった ≡ そのペインの未応答通知は無効」を表現する。

Codex hook との対応：

| Hook | 意味 | 動作 |
|---|---|---|
| PermissionRequest | 承認要求の直前 | `push --kind notification && bellmux bell` |
| Stop | ターン完了 | `push --kind stop && bellmux bell` |
| UserPromptSubmit | ユーザーが応答 | `ack-pane` |
| PostToolUse | ツール実行完了 | `ack-pane` |
| SessionStart(startup/resume/clear) | セッション開始・再開・clear 後 | `ack-pane` |

Codex には現時点で Claude Code の SessionEnd 相当 hook がないため、終了時ではなく次回の SessionStart で stale 通知を掃除する。挙動差は「終了後から次回起動まで stale 通知が残るかどうか」。ユーザーがそのペインに戻って Codex を起動・再開した時点で消えるため、実運用上は SessionEnd cleanup にかなり近い。一方で、終了直後から statusbar を戻したい Claude Code では SessionEnd が使えるならそちらを使う。

### ベルコマンドの分離

`bellmux push` は DB 記録のみに専念し、ベルを鳴らさない。Hook 側で `&& bellmux bell` を連結することで、**ユーザーは後段を自由に差し替えられる**: `afplay /System/Library/Sounds/Ping.aiff`、`terminal-notifier -message ...`、`osascript -e 'display notification ...'` など。push が suppress 判定した場合は **exit 3** で抜けるため、`&&` は自然に後段を skip する。従来の「push が内部で bell を鳴らす」方式は、「glue は全て snippet 埋め込み」の設計原則に反し、かつカスタマイズ性を潰していた。

### `notification_type` による dispatch

新しい Claude Code は Notification の payload に `notification_type` を含める（例: `"permission_prompt"`）。`cmd_push` は：

1. `notification_type` が present なら: surface 対象は `permission_prompt`（権限ダイアログ）と `elicitation_dialog`（MCP サーバーがツール実行中にユーザー入力を要求）の 2 つ。どちらも「ペインがユーザー応答を待っている」状態。それ以外（idle ping, `auth_success`, `elicitation_complete` 等）は suppress（exit 3）。
2. `notification_type` が absent（古い Claude Code）なら: 従来の message 文字列マッチ（"waiting for your input"）にフォールバック。

方針は allowlist: 未知の type は保守的に suppress。新しい surface-worthy type が Claude Code 側に追加された場合のみ手当てする（`elicitation_dialog` は MCP 連携時にペイン応答待ちを取りこぼしていたため後から追加した）。

### ダイアログ応答と ack の関係

Permission dialog の応答に直接対応するフックは **Claude Code が提供していない**（実測確認済み）。

- **"Allow" クリック**: PreToolUse は既にダイアログ前に発火済み、PostToolUse がツール実行完了で発火する。**PostToolUse が "Allow" の確定シグナル**。
- **"Deny" クリック**: フックは一切発火しない。これは上流の hook gap であり、bellmux 側で解決不能。

そのため PostToolUse を ack トリガとして追加した。副作用として Claude がツール連続実行中に pending notification が消えるが、**「Claude が能動的に動いている ≡ ユーザー応答を待っていない」** と解釈できるため意図と整合する。

なお Claude Code はツール完了を**成功（PostToolUse）/ 失敗（PostToolUseFailure）の 2 イベントに分割**する。当初 PostToolUse のみを ack トリガにしていたが、これだと "Allow" 直後にツールが失敗した場合（Bash の非ゼロ終了、grep のヒット 0、Edit の old_string 不一致など失敗は日常的）に PostToolUseFailure だけが発火して通知が残る。「ツールが終わった」ことに成功も失敗も無いため、**両イベントで ack** する。

"Deny" 応答後に古い notification が残る問題は、実運用上「拒否した直後にユーザーが新しいプロンプトを入力することが多く、UserPromptSubmit で自然に ack される」ため許容。Claude Code 側で "ツール拒否時にフック発火する" 機能が将来入れば、そこで拾える。手動 ack (`prefix + A`) も常に利用可能。

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
| 3 | `push` のみ: Notification が suppress 対象だった（idle ping 等）。DB 変更なし。Hook snippet の `bellmux push ... && bellmux bell` で後段の bell を自然に skip するための signal。 |

`push` 成功（exit 0）後に hook は `&& bellmux bell` で bell を鳴らす。`push` 失敗（1/2）や suppress（3）では `&&` が短絡して bell は鳴らず、DB と bell が必ず同期する。

## Phase 2 以降の余地

設計上は対応の余地があるが、Phase 1 では実装しない：

| 項目 | 備考 |
|---|---|
| 他 agent 対応 | hook / notify などのイベント面があれば snippet 追加で対応。イベント面が無い場合のみ polling daemon が必要 |
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
