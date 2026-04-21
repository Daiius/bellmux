use anyhow::{bail, Result};

pub fn pane_id(s: &str) -> Result<()> {
    if s.len() < 2 || !s.starts_with('%') || !s[1..].chars().all(|c| c.is_ascii_digit()) {
        bail!("invalid pane_id format: expected ^%[0-9]+$, got {s:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        assert!(pane_id("%0").is_ok());
        assert!(pane_id("%5").is_ok());
        assert!(pane_id("%12345").is_ok());
    }

    #[test]
    fn rejects_invalid() {
        assert!(pane_id("").is_err());
        assert!(pane_id("%").is_err());
        assert!(pane_id("5").is_err());
        assert!(pane_id("%a").is_err());
        assert!(pane_id("%5;DROP TABLE notifications;--").is_err());
        assert!(pane_id("%-1").is_err());
    }
}
