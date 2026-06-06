use anyhow::{bail, Result};

/// Maximum accepted pane_id length. Generous — real multiplexer ids are short
/// (tmux `%12`, zellij `terminal_3`); the bound just caps pathological input.
const MAX_LEN: usize = 64;

/// Validate a pane_id as an opaque, multiplexer-agnostic key.
///
/// bellmux treats pane_id as a plain string and never interprets it, so this is
/// a boundary sanity check, not a tmux-format check: accept anything that is
/// safe to thread through TSV output, the status bar, and shell-quoted snippet
/// args. SQL is always parameter-bound, so injection isn't the concern here —
/// the allow-list exists to keep whitespace, separators, and control characters
/// out of those display/shell contexts.
///
/// Allowed: `[A-Za-z0-9%_:./-]`, non-empty, at most `MAX_LEN` chars. Covers
/// tmux (`%5`), zellij (`5` / `terminal_5` / `plugin_2`), and composite keys
/// like `session:1.2`. Rejects whitespace, `;`, control chars, and over-long
/// input.
pub fn pane_id(s: &str) -> Result<()> {
    let ok = !s.is_empty()
        && s.len() <= MAX_LEN
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '%' | '_' | ':' | '.' | '/' | '-'));
    if !ok {
        bail!("invalid pane_id: expected non-empty [A-Za-z0-9%_:./-]{{1,{MAX_LEN}}}, got {s:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        // tmux
        assert!(pane_id("%0").is_ok());
        assert!(pane_id("%12345").is_ok());
        // zellij: bare integer, terminal_N, plugin_N
        assert!(pane_id("5").is_ok());
        assert!(pane_id("terminal_5").is_ok());
        assert!(pane_id("plugin_2").is_ok());
        // composite keys (session:window.pane style)
        assert!(pane_id("session:1.2").is_ok());
    }

    #[test]
    fn rejects_invalid() {
        assert!(pane_id("").is_err()); // empty
        assert!(pane_id("%5;DROP TABLE notifications;--").is_err()); // ';' and space
        assert!(pane_id("a b").is_err()); // space
        assert!(pane_id("pane\tid").is_err()); // tab
        assert!(pane_id("line\nbreak").is_err()); // newline
        assert!(pane_id(&"x".repeat(MAX_LEN + 1)).is_err()); // too long
    }
}
