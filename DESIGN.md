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
| Notification (matcher: `permission_prompt\|elicitation_dialog`) | 権限要求 / MCP 入力要求 | `push --kind notification && bellmux bell` |
| Stop | ターン完了 | `push --kind stop && bellmux bell` |
| UserPromptSubmit | ユーザーが応答 | `ack-pane` |
| PostToolUse | ツール実行完了（成功） | `ack-pane`、`run_in_background: true` の Bash なら続けて `hold` |
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

`bellmux push` は DB 記録のみに専念し、ベルを鳴らさない。Hook 側で `&& bellmux bell` を連結することで、**ユーザーは後段を自由に差し替えられる**: `afplay /System/Library/Sounds/Ping.aiff`、`terminal-notifier -message ...`、`osascript -e 'display notification ...'` など。push は受け取った通知を必ず記録し、成功時のみ後段が走る（surface 対象の選別は後述の通り hook matcher が担うので、push 側に suppress 判定は無い）。従来の「push が内部で bell を鳴らす」方式は、「glue は全て snippet 埋め込み」の設計原則に反し、かつカスタマイズ性を潰していた。

### surface 対象の選別は hook matcher が担う

Claude Code の `Notification` は `notification_type` に対して matcher が効く（`permission_prompt` / `idle_prompt` / `auth_success` / `elicitation_dialog` / `elicitation_complete` / `elicitation_response`）。そこで snippet 側で matcher を `permission_prompt|elicitation_dialog` に絞り、**surface すべき種別の選別を hook 設定に寄せた**。この 2 つはどちらも「ペインがユーザー応答を待っている」状態を表す（前者は権限ダイアログ、後者は MCP サーバーがツール実行中に入力を要求）。idle ping や `auth_success` 等は matcher に一致しないため、そもそも hook が発火せず bellmux は呼ばれない。

これにより `cmd_push` は **coding agent のイベント体系を一切知らない**: 渡された通知をそのまま記録するだけ。以前は `cmd_push` が payload の `notification_type` を読んで allowlist 判定し、外れたら exit 3 で抜けていたが、その分岐・文字列フォールバック・`notification_type` パースは全て撤去した。surface 対象を増やしたいときは matcher に `|` 区切りで type を足すだけで、Rust 側は無変更で済む。

トレードオフ: matcher は `notification_type` フィールドに対して効くため、このフィールドを出さない**旧 Claude Code は何も surface されない**（旧来の "waiting for your input" 文字列フォールバックは廃止した）。現行 Claude Code を前提とする。

同じ理由で Codex preset の `PermissionRequest` / `Stop` も固定 JSON を pipe するだけで、bellmux に payload を読ませない。「bellmux は agent のイベントを知らない」という方針で両者は揃っている。

### ダイアログ応答と ack の関係

Permission dialog の応答に直接対応するフックは **Claude Code が提供していない**（実測確認済み）。

- **"Allow" クリック**: PreToolUse は既にダイアログ前に発火済み、PostToolUse がツール実行完了で発火する。**PostToolUse が "Allow" の確定シグナル**。
- **"Deny" クリック**: フックは一切発火しない。これは上流の hook gap であり、bellmux 側で解決不能。

そのため PostToolUse を ack トリガとして追加した。副作用として Claude がツール連続実行中に pending notification が消えるが、**「Claude が能動的に動いている ≡ ユーザー応答を待っていない」** と解釈できるため意図と整合する。

なお Claude Code はツール完了を**成功（PostToolUse）/ 失敗（PostToolUseFailure）の 2 イベントに分割**する。当初 PostToolUse のみを ack トリガにしていたが、これだと "Allow" 直後にツールが失敗した場合（Bash の非ゼロ終了、grep のヒット 0、Edit の old_string 不一致など失敗は日常的）に PostToolUseFailure だけが発火して通知が残る。「ツールが終わった」ことに成功も失敗も無いため、**両イベントで ack** する。

"Deny" 応答後に古い notification が残る問題は、実運用上「拒否した直後にユーザーが新しいプロンプトを入力することが多く、UserPromptSubmit で自然に ack される」ため許容。Claude Code 側で "ツール拒否時にフック発火する" 機能が将来入れば、そこで拾える。手動 ack (`prefix + A`) も常に利用可能。

### self-driving ペインと hold

**Stop は「ターン境界」であって「ユーザー入力待ち」ではない。** bellmux は前者を後者の代理として使っているので、エージェントが自分自身を再起動するループでズレる。

具体例（`Daiius/oculibis` のレビュー待機レシピ: `run_in_background` で 30 秒 × 40 回ポーリング）：

1. 待機ループを起動してターン終了 → **Stop → push** → **20 分間ずっと「あなた待ち」表示**
2. ループ完了 → 自動再開 → 修正 → 再度待機 → **Stop → push** → また同じ
3. 全部終わって最終報告 → **Stop → push** → これだけが正しい

1 と 2 は誤りだが、3 は残したい。「Stop を通知源から外す」では 3 を失う。

#### 実測: どのイベントが何を知っているか

診断 hook を仕込んで 1 ペイン分のイベント列を実測した（`~/.claude/settings.json` は全セッション共通なので、`TMUX_PANE` と `session_id` でフィルタしないと他エージェントのイベントが混ざる）。

```
05:40:01  PostToolUse  tool=Bash  run_in_background=true   ← 起動の時点で発火する
05:40:07  Stop                                             ← 誤通知
05:41:07  Notification  notification_type=idle_prompt      ← Stop の 60 秒後
05:41:16  UserPromptSubmit                                 ← task 完了による自動再開
```

わかったこと：

- **`Stop` の payload に「なぜこのターンが始まったか」は無い**（`session_id` / `transcript_path` / `last_assistant_message` 等のみ）。起動元の判別はできない。
- **`idle_prompt` は使えない**。バックグラウンドタスクが走っている最中でも Stop の 60 秒後に発火するので、「本当にユーザーが必要」を意味しない。ドキュメントの「teammate が idle になる通知」という説明も不正確で、teammate の無いセッションで発火した。
- **`TaskCreated` / `TaskCompleted` は `TaskCreate` ツール専用**で、`run_in_background` の Bash や Monitor は対象外。
- **自動再開でも `UserPromptSubmit` が発火する**（task 完了通知が user message として注入される）。したがって「UserPromptSubmit で arm して Stop で消費する（＝人間のプロンプト 1 回につき通知 1 回）」案は成立しない。そもそもこの案は上記 1 が armed 状態なので、一番痛いケースを潰せない。
- **使える手がかりは 1 つだけ**: `PostToolUse` の payload に `tool_name` と `tool_input` がそのまま来るので、**「バックグラウンド待機を起動した」ことは Stop の 1 イベント前に観測できる**。

#### 設計: read 時フィルタとしての hold

`holds(pane_id, expires_at)` に「このペインは self-driving」を記録し、`status` / `ordered_panes` が読み出し時に除外する。

**`push` は変更しない。** 「push は受け取った通知を必ず記録する」という既存の不変条件を保ったまま、抑制を surface 側だけに置く。これにより：

- 抑制中でも記録は残るので `list` で見える（`held` フラグ付き）。「statusbar が静か」と「キューが空」が区別できなくなる事態を避ける。
- **lease が切れれば自然に表面化する**。hold が漏れたときに倒れる方向が「通知が遅れる」であって「通知が消える」ではない。抑制機構としてはこの向きが必須。
- 書き込み側（hook の `push`）に「出すか出さないか」の判断を持ち込まない。surface 対象の選別を hook matcher に寄せた方針と同じ形。

hold の解放は **`ack-pane` が主経路、TTL は backstop**。ack は「ユーザーが介入した」か「ペインが実作業に戻った」を意味し、どちらでも self-driving ではなくなる。PostToolUse hook は 1 コマンド内で `ack-pane` → 条件付き `hold` の順に走るので、待機を起動したターンだけが hold を持ち越す。既定 TTL は 1800 秒 —— 現実的な自走待機を覆い、かつセッションごと落ちた場合でも同じセッション内で表面化する長さ。

#### 検討して採らなかった案

| 案 | 却下理由 |
|---|---|
| 待機ループ自身が毎周 `ack-pane` を打つ | bellmux 変更ゼロで済むが、ポーリング間隔ぶんの誤通知が残り、かつ自分で書いていない待機（組み込みツール等）には仕込めない |
| Stop を通知源から外し、明示 `push` に任せる | 実装ゼロだがモデルの規律に依存し、上記 3（本当に見てほしい完了）を落とす |
| `UserPromptSubmit` で arm、`Stop` で消費 | 自動再開でも UserPromptSubmit が発火するため成立しない（実測） |
| `push` 側で hold を見て INSERT をスキップ | 記録が残らないので失効時に表面化できず、fail-visible にならない |
| `jq` を使わず payload を文字列 grep | `command` 引数の中にたまたま `"run_in_background":true` を含む呼び出しで誤爆する（この機能の開発中に実際に起こりうる） |

#### 適用範囲

hold を張るのは **`tool_name == "Bash"` かつ `tool_input.run_in_background == true`** の場合のみ。これは PostToolUse が「起動時」に発火することを実測で確認できた唯一のケースだから。サブエージェント起動など他のバックグラウンド系ツールは、PostToolUse が「完了時」に発火する可能性があり、その場合 hold は**見たい通知の方を潰す**。検証できたものだけを対象にする。

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
- **TSV / 表示層 injection**: `sanitize_message` で全制御文字（tab / CR / LF に加え ESC / BEL 等）を空白置換し、status bar / `list` へのターミナルエスケープシーケンス注入を防ぐ

### 禁止

- 通知 message を eval / shell コマンドとして実行しない
- `run-shell` に DB 由来の文字列を直接渡さない（`pane_id` は検証済み正規表現に合致するもののみ）

## エラーコード

| Code | 意味 |
|---|---|
| 0 | 正常終了。stdin が空 / 非 JSON でも `message=NULL` で INSERT する（寛容 parse）。 |
| 1 | 実行時エラー（DB ロック超過、IO エラー、disk full 等）。stderr に warn。 |
| 2 | 引数エラー（`pane_id` 不正形式、未知の `--kind` 値等）。 |

`push` 成功（exit 0）後に hook は `&& bellmux bell` で bell を鳴らす。`push` 失敗（1/2）では `&&` が短絡して bell は鳴らず、DB と bell が必ず同期する。surface 対象の選別は hook matcher が担うため、`push` 自身は suppress signal（旧 exit 3）を持たない。

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
