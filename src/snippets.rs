//! Embedded init snippets returned by `bellmux init`.
//!
//! Snippets are user-facing examples; they are intentionally verbose with
//! comments so a beginner can copy/paste and understand each line. Colours and
//! key bindings are illustrative — users are expected to tune them.

// NOTE: snippets contain the literal sequence `"#` (tmux format substitutions
// like `"#{pane_id}"`), which would close a normal `r#"..."#` raw string. We
// use `r##"..."##` everywhere so the snippet bodies can hold any single-`#`
// sequence safely.

pub const WIDGET: &str = r##"# --- bellmux status preset: widget ---
# Right-side widget that lights up when notifications are pending.
# A 🔔 badge appears when the pane you are currently looking at is the one
# waiting on you; the orange count covers all pending panes.
#
# `bellmux status --only-pane #{pane_id}` prints nothing unless the active pane
# (tmux expands #{pane_id} inside #() per client) has a pending notification, so
# the #{?...} condition is false otherwise.
set -g status-interval 2
set -g status-right '#{?#(bellmux status --only-pane #{pane_id} --format here),🔔 ,}#[bg=colour208 fg=black]#(bellmux status)#[default] %H:%M '
"##;

pub const FULLBAR: &str = r##"# --- bellmux status preset: fullbar ---
# Flip the whole status bar to a notify colour while notifications are pending.
# Two explicit colours: @bellmux-status-normal / @bellmux-status-notify.
# Override these to match your existing status-style.
#
# Border is intentionally left untouched: tmux only re-renders borders on
# focus/layout events, so a conditional border style lags behind status
# (the status bar polls every status-interval seconds via #(...), borders
# do not). If you want a single coherent "alert" colour, prefer the bar.
# Requires tmux >= 2.9 for #{?#(...),T,F} conditional in styles.
#
# The bar flips orange whenever something is pending. Flipping the WHOLE bar to
# a third colour for the current pane is jarring, so instead a `here 🔔` badge
# appears in status-right when the pane you are currently in is the one waiting
# on you (vs. only other panes pending). The per-pane probe `--only-pane
# #{pane_id}` prints nothing unless that exact pane is pending.
set -g status-interval 2
set -g @bellmux-status-normal 'bg=green fg=black'
set -g @bellmux-status-notify 'bg=colour208 fg=black'
set -g status-style '#{?#(bellmux status),#{@bellmux-status-notify},#{@bellmux-status-normal}}'

set -g status-right '#{?#(bellmux status --only-pane #{pane_id} --format here),here 🔔 ,}#(bellmux status --format="{n}: {latest_message}") | %H:%M '
"##;

pub const OVERLAY: &str = r##"# --- bellmux status preset: overlay ---
# Non-destructive: leaves your existing status-style / status-bg untouched.
# When notifications are pending, an orange block with "{n}: {latest_message}"
# appears in status-right; otherwise nothing extra is shown. A 🔔 badge precedes
# the block when the pane you are currently in is the one waiting on you (vs.
# only other panes pending).
set -g status-interval 2
set -g status-right '#{?#(bellmux status --only-pane #{pane_id} --format here),🔔 ,}#[#{?#(bellmux status),bg=colour208 fg=black bold,}]#(bellmux status --format=" {n}: {latest_message} ")#[default] %H:%M '
"##;

pub const DOT: &str = r##"# --- bellmux status preset: dot ---
# Minimal single glyph: 🔔 when the current pane is the one waiting on you, an
# orange ● when only other panes are pending, absent when nothing is pending.
set -g status-interval 2
set -g status-right '#[fg=colour208 bold]#{?#(bellmux status --only-pane #{pane_id} --format here),🔔,#(bellmux status --format="●")} #[default]%H:%M '
"##;

pub const POPUP_SIMPLE: &str = r##"# --- bellmux popup preset: simple ---
# Show all pending notifications. No external deps beyond tmux + less.
# Bound to prefix+N (uppercase) to avoid clashing with the default
# next-window binding on prefix+n.
bind-key N display-popup -E -w 80% -h 70% 'bellmux list | less'
"##;

pub const POPUP_ENRICHED: &str = r##"# --- bellmux popup preset: enriched ---
# Resolve each pane_id back to "session:window.pane title" via tmux.
# No jq required — uses TSV output and bash while-read.
# Bound to prefix+N (uppercase) to avoid clashing with the default
# next-window binding on prefix+n.
bind-key N display-popup -E -w 80% -h 70% "bellmux list --tsv | while IFS=\$'\t' read pane created kind msg; do info=\$(tmux display-message -t \"\$pane\" -p '#S:#I.#P #T' 2>/dev/null || echo '(dead)'); printf '%-30s %-25s %-12s %s\n' \"\$info\" \"\$created\" \"\$kind\" \"\$msg\"; done | less"
"##;

pub const KEYBINDS: &str = r##"# --- bellmux keybindings ---
# Jump to the next pending notification in cycle order (does NOT ack).
# Cursor lives in SQLite: first press enters at the newest pane, subsequent
# presses walk toward older panes, wrapping oldest → newest. ack/prune that
# removes the cursor's pane resets the cursor.
#
# `bellmux next` prints the target pane_id and appends ` wrapped` when
# the advance crossed the cycle boundary (one pass done) or only one pane
# is pending (every press revisits it). The keybind uses that tag to show
# a "cycled through all" message via display-message.
#
# `--current #{pane_id}` tells bellmux which pane you are in so it never jumps
# to the pane you are already sitting in: if another pane is pending it steps
# past you to that one; if yours is the *only* pending pane it is returned with
# the ` wrapped` tag (the switch-client below is then a harmless no-op and you
# still get the "cycled through all" message).
#
# Dead panes are pruned by the pane-died hook (see tmux-hook preset), so
# we don't handle that case explicitly here.
#
# The body is POSIX sh, not bash: tmux runs `run-shell` commands with /bin/sh,
# which is dash on Debian/Ubuntu. A bash here-string (`read -r pane tag <<<"$(...)"`)
# is a syntax error there and the binding fails with `returned 2` — it only
# appears to work on macOS, where /bin/sh is bash. `set --` splits the output
# portably; pane_id and the ` wrapped` tag contain no whitespace or glob
# characters (bellmux validates pane_id), so the unquoted substitution is safe.
bind-key a run-shell '
  set -- $(bellmux next --current "#{pane_id}")
  pane=$1
  tag=$2
  if [ -z "$pane" ]; then
    tmux display-message "No pending notifications"
    exit 0
  fi
  tmux switch-client -t "$pane"
  if [ "$tag" = wrapped ]; then
    tmux display-message "Cycled through all pending notifications."
  fi
'

# Jump to the previous pending notification (opposite direction).
bind-key b run-shell '
  set -- $(bellmux prev --current "#{pane_id}")
  pane=$1
  tag=$2
  if [ -z "$pane" ]; then
    tmux display-message "No pending notifications"
    exit 0
  fi
  tmux switch-client -t "$pane"
  if [ "$tag" = wrapped ]; then
    tmux display-message "Cycled through all pending notifications."
  fi
'

# Ack all notifications for the current pane (use when you saw it but won't reply).
bind-key A run-shell 'bellmux ack-pane --pane-id "#{pane_id}" && tmux refresh-client -S'

# Ack everything everywhere.
bind-key X run-shell 'bellmux ack-all && tmux refresh-client -S'
"##;

pub const TMUX_HOOK: &str = r##"# --- bellmux tmux hook ---
# Drop notifications for panes that have died (so jump-latest never points at a ghost).
set-hook -g pane-died 'run-shell "bellmux prune-pane --pane-id #{pane_id}"'
"##;

pub const CLAUDE_HOOKS: &str = r##"# --- bellmux Claude Code hooks ---
# Add these to ~/.claude/settings.json. If the file already has a "hooks"
# section, merge — do NOT overwrite. The $TMUX_PANE env var is set by tmux
# automatically and inherited by the Claude Code hook subprocess.
#
# Running outside tmux:
#   Every command starts with `[ -n "$TMUX_PANE" ] || exit 0;` so that a Claude
#   Code session started outside tmux exits the hook silently instead of failing
#   `bellmux: invalid pane_id ... got ""`. Running outside tmux is a normal,
#   deliberate thing to do; it should not print an error on every turn. The
#   guard lives here rather than in bellmux itself: bellmux validates pane_id at
#   its boundary (a genuinely empty --pane-id stays an error), and deciding
#   "there is no pane to notify about" is the glue's job.
#
# Customising the alert sound:
#   `bellmux push ... && bellmux bell` records the notification, then rings BEL
#   on every login tty. Replace `bellmux bell` with anything:
#   `afplay /System/Library/Sounds/Ping.aiff`,
#   `osascript -e 'display notification "..."'`, `terminal-notifier ...`, etc.
#
# Notification policy:
#   The Notification matcher picks which notification types reach bellmux. We
#   surface only `permission_prompt` (a tool permission dialog) and
#   `elicitation_dialog` (an MCP server requesting input mid-tool) — both mean
#   "this pane is waiting on you". Idle pings, auth_success, and the like never
#   match, so Claude Code never runs the hook for them and bellmux stays
#   agent-agnostic: it records whatever it is handed. Add more types to the
#   matcher (|-separated) to surface them.
#
# Ack policy:
#   - UserPromptSubmit: user typed a new prompt — clear pending.
#   - PostToolUse / PostToolUseFailure: a tool just finished. This is the only
#     reliable signal that the user responded "Allow" to a permission dialog
#     (PreToolUse fires *before* the dialog; Claude Code fires no hook at all
#     on "Deny"). Claude Code splits tool completion into two events — success
#     fires PostToolUse, failure fires PostToolUseFailure — so we ack on both;
#     otherwise a tool that fails right after "Allow" leaves the notification
#     stuck. Acking here also clears any pending notification for the pane
#     whenever Claude is actively running tools, which matches the "Claude is
#     working, don't nag me" intent.
#   - SessionEnd: the Claude session ended (/clear, logout, or exiting Claude
#     while the tmux pane lives on). Clear pending so a stale notification does
#     not ghost until the pane itself dies (the pane-died tmux hook only fires
#     when the pane actually closes).
#
# Subagent events:
#   Subagents run in the same tmux pane and fire PostToolUse /
#   PostToolUseFailure for every tool they call, with `agent_id` set in the
#   payload. They are NOT acks: a background subagent's tool call says nothing
#   about whether the user answered a dialog in the main session, and acking on
#   it would clear a real pending notification (and release a hold) while the
#   main session is still waiting. So the ack hooks skip payloads that carry
#   `agent_id`. Without jq the check cannot be made and every event acks, as
#   before.
#
# Hold policy (self-driving panes):
#   Stop means "the turn ended", which is not the same as "the user is needed".
#   When Claude starts background work (a backgrounded Bash command, a
#   background subagent, a Monitor, ...) and ends its turn to wait for it, Stop
#   fires and the pane would be advertised as waiting on you for the whole wait.
#
#   The Stop payload lists the session's background work in `background_tasks`
#   (Claude Code >= 2.1.145), e.g.
#     {"id":"...","type":"subagent","status":"running",...}
#     {"id":"...","type":"shell","status":"running","command":"sleep 45",...}
#   So the Stop hook asks one question at the moment the turn ends: is anything
#   still running? If so it places a hold BEFORE recording the stop (so the
#   status bar never sees it unheld) and does not ring the bell. The stop is
#   still recorded either way; `bellmux hold` only stops `status` and
#   `next`/`prev` from advertising the pane while the lease is alive, and the
#   notification resurfaces if the lease expires. When the background work
#   finishes, Claude resumes the session, UserPromptSubmit fires, and its
#   `ack-pane` releases the hold.
#
#   A Notification (permission prompt / elicitation) always means the user is
#   needed, even on a held pane — a background subagent asking for permission
#   is exactly that. So the Notification hook releases any hold before
#   recording, and the failure of that unhold does not block the record. The
#   cost: after you answer a subagent's dialog the pane stays flagged until the
#   main session acks (subagent tool events do not ack; see above). That errs
#   on the visible side.
#
#   A failed hold must not cost the record, so the push runs regardless of the
#   hold's exit status: the failure direction is "notified when it should not
#   have been", never "silently dropped".
#
#   `idle_prompt` is deliberately NOT used as the "user is really needed"
#   signal; the Notification matcher does not include it.
#
#   jq is optional. Without it (or on a Claude Code that predates
#   `background_tasks`) no hold is placed and every Stop notifies.
{
  "hooks": {
    "Notification": [{
      "matcher": "permission_prompt|elicitation_dialog",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; bellmux unhold --pane-id \"$TMUX_PANE\"; bellmux push --kind notification --pane-id \"$TMUX_PANE\" && bellmux bell"
      }]
    }],
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; payload=$(cat); if command -v jq >/dev/null 2>&1 && printf '%s' \"$payload\" | jq -e 'any(.background_tasks[]?; .status == \"running\")' >/dev/null 2>&1; then bellmux hold --pane-id \"$TMUX_PANE\"; printf '%s' \"$payload\" | bellmux push --kind stop --pane-id \"$TMUX_PANE\"; else printf '%s' \"$payload\" | bellmux push --kind stop --pane-id \"$TMUX_PANE\" && bellmux bell; fi"
      }]
    }],
    "UserPromptSubmit": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }],
    "PostToolUse": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; payload=$(cat); if command -v jq >/dev/null 2>&1 && printf '%s' \"$payload\" | jq -e 'has(\"agent_id\")' >/dev/null 2>&1; then exit 0; fi; bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }],
    "PostToolUseFailure": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; payload=$(cat); if command -v jq >/dev/null 2>&1 && printf '%s' \"$payload\" | jq -e 'has(\"agent_id\")' >/dev/null 2>&1; then exit 0; fi; bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }],
    "SessionEnd": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }]
  }
}
"##;

pub const CODEX_HOOKS: &str = r##"# --- bellmux Codex hooks ---
# Add the JSON object below to ~/.codex/hooks.json. If the file already
# exists, merge the top-level "hooks" object; do not overwrite unrelated hooks.
# Codex passes the hook payload on stdin. The push hooks below intentionally
# pipe a small fixed JSON object into bellmux instead, so Codex prompts and
# hook payloads are not stored in bellmux. $TMUX_PANE is inherited by hook
# subprocesses when Codex runs inside tmux.
#
# Running outside tmux:
#   Every command starts with `[ -n "$TMUX_PANE" ] || exit 0;` so a Codex
#   session started outside tmux exits the hook silently instead of failing on
#   an empty --pane-id. See the claude-hooks preset for why the guard lives in
#   the snippet rather than in bellmux.
#
# Codex hook trust:
#   Non-managed command hooks must be reviewed and trusted before they run.
#   Use /hooks in Codex after adding this file.
#
# Notification policy:
#   - PermissionRequest: Codex is about to ask for approval; surface it.
#   - Stop: assistant turn completed; surface it.
#
# Ack policy:
#   - UserPromptSubmit: user typed a new prompt; clear pending for this pane.
#   - PostToolUse: a tool completed, including non-zero Bash exits in current
#     Codex releases; clear any stale approval notification for this pane.
#   - SessionStart startup/resume/clear: Codex does not currently expose a
#     SessionEnd hook, so clear stale pane notifications when a Codex session
#     starts or resumes in the pane. This is close to SessionEnd cleanup in
#     practice: the next time the user returns to that pane, stale work is no
#     longer advertised.
{
  "hooks": {
    "PermissionRequest": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; printf '%s' '{\"message\":\"Codex needs approval\"}' | bellmux push --kind notification --pane-id \"$TMUX_PANE\" && bellmux bell",
        "statusMessage": "Recording bellmux approval notification"
      }]
    }],
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; printf '%s' '{\"message\":\"Codex turn complete\"}' | bellmux push --kind stop --pane-id \"$TMUX_PANE\" && bellmux bell",
        "statusMessage": "Recording bellmux turn notification"
      }]
    }],
    "UserPromptSubmit": [{
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; bellmux ack-pane --pane-id \"$TMUX_PANE\"",
        "statusMessage": "Clearing bellmux notification"
      }]
    }],
    "PostToolUse": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; bellmux ack-pane --pane-id \"$TMUX_PANE\"",
        "statusMessage": "Clearing bellmux tool notification"
      }]
    }],
    "SessionStart": [{
      "matcher": "startup|resume|clear",
      "hooks": [{
        "type": "command",
        "command": "[ -n \"$TMUX_PANE\" ] || exit 0; bellmux ack-pane --pane-id \"$TMUX_PANE\"",
        "statusMessage": "Clearing stale bellmux notification"
      }]
    }]
  }
}
"##;

pub fn all() -> String {
    let header = "# Add the snippets below to ~/.tmux.conf\n# (and the coding-agent hooks blocks to their user config files).\n# Then: `tmux source-file ~/.tmux.conf` and restart any running agent sessions.\n\n";
    let mut out = String::new();
    out.push_str(header);
    out.push_str(WIDGET);
    out.push('\n');
    out.push_str(FULLBAR);
    out.push('\n');
    out.push_str(OVERLAY);
    out.push('\n');
    out.push_str(DOT);
    out.push('\n');
    out.push_str(POPUP_SIMPLE);
    out.push('\n');
    out.push_str(POPUP_ENRICHED);
    out.push('\n');
    out.push_str(KEYBINDS);
    out.push('\n');
    out.push_str(TMUX_HOOK);
    out.push_str("\n# --- Claude Code hooks (paste into ~/.claude/settings.json) ---\n");
    out.push_str(CLAUDE_HOOKS);
    out.push_str("\n# --- Codex hooks (paste into ~/.codex/hooks.json) ---\n");
    out.push_str(CODEX_HOOKS);
    out
}

pub fn by_name(name: &str) -> Option<&'static str> {
    match name {
        "widget" => Some(WIDGET),
        "fullbar" => Some(FULLBAR),
        "overlay" => Some(OVERLAY),
        "dot" => Some(DOT),
        "popup-simple" => Some(POPUP_SIMPLE),
        "popup-enriched" => Some(POPUP_ENRICHED),
        "keybinds" => Some(KEYBINDS),
        "tmux-hook" => Some(TMUX_HOOK),
        "claude-hooks" => Some(CLAUDE_HOOKS),
        "codex-hooks" => Some(CODEX_HOOKS),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hook presets are a `#`-commented header followed by the JSON object
    /// the user pastes into their agent config. Strip the header and parse the
    /// rest — the commands inside are hand-escaped inside a Rust raw string,
    /// which is exactly the kind of thing that silently rots.
    fn json_body(preset: &str) -> serde_json::Value {
        let body: String = preset
            .lines()
            .skip_while(|l| l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str(&body).expect("preset body must be valid JSON")
    }

    fn command(preset: &str, event: &str) -> String {
        json_body(preset)["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("{event} hook has no command string"))
            .to_string()
    }

    #[test]
    fn hook_presets_are_valid_json() {
        json_body(CLAUDE_HOOKS);
        json_body(CODEX_HOOKS);
    }

    #[test]
    fn every_preset_is_reachable_by_name() {
        for name in [
            "widget",
            "fullbar",
            "overlay",
            "dot",
            "popup-simple",
            "popup-enriched",
            "keybinds",
            "tmux-hook",
            "claude-hooks",
            "codex-hooks",
        ] {
            assert!(by_name(name).is_some(), "{name} is not reachable");
        }
        assert!(by_name("nope").is_none());
    }

    /// Run a Claude hook command under `sh` with `bellmux` replaced by a stub
    /// that logs each invocation (subcommand plus whatever it read on stdin),
    /// and return the log lines. `None` when jq is unavailable, since the
    /// behaviour under test is the jq branch.
    fn run_claude_hook(event: &str, payload: &str) -> Option<Vec<String>> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        if Command::new("jq").arg("--version").output().is_err() {
            return None;
        }
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "bellmux-hook-test-{}-{event}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        let _ = std::fs::remove_file(&log);
        let stub = dir.join("bellmux");
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\nin=$(cat)\nprintf '%s stdin=%s\\n' \"$1\" \"$in\" >> '{}'\n",
                log.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!(
            "{}:{}",
            dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command(CLAUDE_HOOKS, event))
            .env("PATH", path)
            .env("TMUX_PANE", "%1")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success(), "{event} hook failed");
        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        Some(calls.lines().map(|l| l.to_string()).collect())
    }

    fn subcommands(calls: &[String]) -> Vec<&str> {
        calls.iter().map(|l| l.split(' ').next().unwrap()).collect()
    }

    /// A Stop with background work still running is the agent waiting on
    /// itself: hold first (so the status bar never sees the stop unheld),
    /// still record it, and do not ring the bell.
    #[test]
    fn stop_with_running_background_task_holds_then_records_without_bell() {
        let payload = r#"{"hook_event_name":"Stop","background_tasks":[{"id":"x","type":"subagent","status":"running"}]}"#;
        let Some(calls) = run_claude_hook("Stop", payload) else {
            return;
        };
        assert_eq!(subcommands(&calls), ["hold", "push"], "{calls:?}");
        assert!(
            calls[1].contains(payload),
            "push must still receive the payload: {calls:?}"
        );
    }

    /// No running background work: an ordinary stop that notifies and rings.
    #[test]
    fn stop_without_running_background_task_records_and_rings() {
        for payload in [
            r#"{"hook_event_name":"Stop","background_tasks":[]}"#,
            r#"{"hook_event_name":"Stop","background_tasks":[{"id":"x","type":"shell","status":"completed"}]}"#,
            r#"{"hook_event_name":"Stop"}"#,
        ] {
            let Some(calls) = run_claude_hook("Stop", payload) else {
                return;
            };
            assert_eq!(
                subcommands(&calls),
                ["push", "bell"],
                "{payload}: {calls:?}"
            );
        }
    }

    /// The record must not depend on the hold succeeding: a failed hold costs
    /// an unwanted notification, never a lost one.
    #[test]
    fn stop_records_even_if_hold_fails() {
        let cmd = command(CLAUDE_HOOKS, "Stop");
        let hold = cmd.find("bellmux hold").expect("Stop must place holds");
        let push = hold + cmd[hold..].find("bellmux push").expect("push after hold");
        let between = &cmd[hold..push];
        assert!(
            !between.contains("&&") && !between.contains("||"),
            "push must not be chained on the hold's exit status: {cmd}"
        );
    }

    /// A permission prompt means the user is needed even while the pane is
    /// held for background work, so the hold is released before recording.
    #[test]
    fn notification_releases_hold_then_records_and_rings() {
        let Some(calls) = run_claude_hook(
            "Notification",
            r#"{"notification_type":"permission_prompt"}"#,
        ) else {
            return;
        };
        assert_eq!(subcommands(&calls), ["unhold", "push", "bell"], "{calls:?}");
    }

    /// A subagent's tool call is not an ack of the main session: it must not
    /// clear a pending notification or release a hold.
    #[test]
    fn tool_events_from_subagents_do_not_ack() {
        for event in ["PostToolUse", "PostToolUseFailure"] {
            let sub = r#"{"hook_event_name":"PostToolUse","agent_id":"a1","agent_type":"general-purpose","tool_name":"Bash"}"#;
            let Some(calls) = run_claude_hook(event, sub) else {
                return;
            };
            assert!(
                calls.is_empty(),
                "{event} from a subagent must not call bellmux: {calls:?}"
            );

            let main = r#"{"hook_event_name":"PostToolUse","tool_name":"Bash"}"#;
            let calls = run_claude_hook(event, main).unwrap();
            assert_eq!(
                subcommands(&calls),
                ["ack-pane"],
                "{event} from the main session: {calls:?}"
            );
        }
    }

    /// Every hook command has to survive being run outside tmux, where
    /// `$TMUX_PANE` is unset and there is no pane to notify about.
    #[test]
    fn every_hook_command_guards_on_tmux_pane() {
        for preset in [CLAUDE_HOOKS, CODEX_HOOKS] {
            let hooks = json_body(preset)["hooks"].clone();
            for (event, entries) in hooks.as_object().expect("hooks must be an object") {
                for entry in entries.as_array().expect("event must hold an array") {
                    for hook in entry["hooks"].as_array().expect("entry needs hooks") {
                        let cmd = hook["command"].as_str().expect("hook needs a command");
                        assert!(
                            cmd.starts_with(r#"[ -n "$TMUX_PANE" ] || exit 0;"#),
                            "{event} command is missing the outside-tmux guard: {cmd}"
                        );
                    }
                }
            }
        }
    }
}
