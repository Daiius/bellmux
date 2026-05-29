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
# Dead panes are pruned by the pane-died hook (see tmux-hook preset), so
# we don't handle that case explicitly here.
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

# Jump to the previous pending notification (opposite direction).
bind-key b run-shell '
  read -r pane tag <<<"$(bellmux prev)"
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
{
  "hooks": {
    "Notification": [{
      "matcher": "permission_prompt|elicitation_dialog",
      "hooks": [{
        "type": "command",
        "command": "bellmux push --kind notification --pane-id \"$TMUX_PANE\" && bellmux bell"
      }]
    }],
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "bellmux push --kind stop --pane-id \"$TMUX_PANE\" && bellmux bell"
      }]
    }],
    "UserPromptSubmit": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }],
    "PostToolUse": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }],
    "PostToolUseFailure": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""
      }]
    }],
    "SessionEnd": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\""
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
        "command": "printf '%s' '{\"message\":\"Codex needs approval\"}' | bellmux push --kind notification --pane-id \"$TMUX_PANE\" && bellmux bell",
        "statusMessage": "Recording bellmux approval notification"
      }]
    }],
    "Stop": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "printf '%s' '{\"message\":\"Codex turn complete\"}' | bellmux push --kind stop --pane-id \"$TMUX_PANE\" && bellmux bell",
        "statusMessage": "Recording bellmux turn notification"
      }]
    }],
    "UserPromptSubmit": [{
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\"",
        "statusMessage": "Clearing bellmux notification"
      }]
    }],
    "PostToolUse": [{
      "matcher": "",
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\"",
        "statusMessage": "Clearing bellmux tool notification"
      }]
    }],
    "SessionStart": [{
      "matcher": "startup|resume|clear",
      "hooks": [{
        "type": "command",
        "command": "bellmux ack-pane --pane-id \"$TMUX_PANE\"",
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
