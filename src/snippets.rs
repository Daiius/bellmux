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
set -g status-interval 2
set -g status-right '#[bg=colour208,fg=black]#(bellmux status)#[default] %H:%M '
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
set -g status-interval 2
set -g @bellmux-status-normal 'bg=green fg=black'
set -g @bellmux-status-notify 'bg=colour208 fg=black'
set -g status-style '#{?#(bellmux status),#{@bellmux-status-notify},#{@bellmux-status-normal}}'

set -g status-right '#(bellmux status --format="{n}: {latest_message}") | %H:%M '
"##;

pub const OVERLAY: &str = r##"# --- bellmux status preset: overlay ---
# Non-destructive: leaves your existing status-style / status-bg untouched.
# When notifications are pending, an orange block with "{n}: {latest_message}"
# appears in status-right; otherwise nothing extra is shown.
set -g status-interval 2
set -g status-right '#[#{?#(bellmux status),bg=colour208 fg=black bold,}]#(bellmux status --format=" {n}: {latest_message} ")#[default] %H:%M '
"##;

pub const DOT: &str = r##"# --- bellmux status preset: dot ---
# Minimal coloured dot.
set -g status-interval 2
set -g status-right '#[fg=colour208,bold]#(bellmux status --format="●") #[default]%H:%M '
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
bind-key a run-shell 'read -r pane tag <<<"$(bellmux next)"; if [ -n "$pane" ]; then tmux switch-client -t "$pane" 2>/dev/null || { bellmux prune-pane --pane-id "$pane"; tmux display-message "Pane no longer exists, pruned."; }; if [ "$tag" = wrapped ]; then tmux display-message "Cycled through all pending notifications."; fi; fi'

# Jump to the previous pending notification (opposite direction).
bind-key b run-shell 'read -r pane tag <<<"$(bellmux prev)"; if [ -n "$pane" ]; then tmux switch-client -t "$pane" 2>/dev/null || { bellmux prune-pane --pane-id "$pane"; tmux display-message "Pane no longer exists, pruned."; }; if [ "$tag" = wrapped ]; then tmux display-message "Cycled through all pending notifications."; fi; fi'

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
#   `bellmux push ... && bellmux bell` fires BEL on every login tty after a
#   successful push. `bellmux push` exits 3 when a notification is suppressed
#   (e.g. idle ping), so `&&` naturally skips the bell in that case. Replace
#   `bellmux bell` with anything: `afplay /System/Library/Sounds/Ping.aiff`,
#   `osascript -e 'display notification "..."'`, `terminal-notifier ...`, etc.
#
# Ack policy:
#   - UserPromptSubmit: user typed a new prompt — clear pending.
#   - PostToolUse: a tool just finished. This is the only reliable signal that
#     the user responded "Allow" to a permission dialog (PreToolUse fires
#     *before* the dialog; Claude Code fires no hook at all on "Deny"). Acking
#     here also clears any pending notification for the pane whenever Claude
#     is actively running tools, which matches the "Claude is working, don't
#     nag me" intent.
{
  "hooks": {
    "Notification": [{
      "matcher": "",
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
    }]
  }
}
"##;

pub fn all() -> String {
    let header = "# Add the snippets below to ~/.tmux.conf\n# (and the Claude Code hooks block to ~/.claude/settings.json).\n# Then: `tmux source-file ~/.tmux.conf` and restart any running Claude Code sessions.\n\n";
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
        _ => None,
    }
}
