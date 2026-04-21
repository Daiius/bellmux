use crate::db::StatusSnapshot;

/// Render a `--format` template against the current snapshot.
///
/// Supported placeholders:
///   {n}              - distinct pane count
///   {latest_message} - most recent message; falls back to kind name when null
///   {latest_pane}    - pane id of the most recent notification
///
/// Returns the empty string when the snapshot is empty (n == 0), regardless of
/// what the template requested. The status bar should disappear cleanly when
/// nothing is pending.
pub fn render(template: &str, snap: &StatusSnapshot) -> String {
    if snap.n == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        let mut closed = false;
        for c in chars.by_ref() {
            if c == '}' {
                closed = true;
                break;
            }
            name.push(c);
        }
        if !closed {
            out.push('{');
            out.push_str(&name);
            continue;
        }
        match name.as_str() {
            "n" => out.push_str(&snap.n.to_string()),
            "latest_message" => {
                let fallback = snap.latest_kind.as_deref().unwrap_or("");
                let msg = snap.latest_message.as_deref().unwrap_or(fallback);
                out.push_str(msg);
            }
            "latest_pane" => out.push_str(snap.latest_pane.as_deref().unwrap_or("")),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(n: usize, msg: Option<&str>, pane: Option<&str>, kind: Option<&str>) -> StatusSnapshot {
        StatusSnapshot {
            n,
            latest_message: msg.map(String::from),
            latest_pane: pane.map(String::from),
            latest_kind: kind.map(String::from),
        }
    }

    #[test]
    fn empty_yields_empty() {
        assert_eq!(render("AGENT:{n}", &snap(0, None, None, None)), "");
        assert_eq!(render("anything", &snap(0, None, None, None)), "");
    }

    #[test]
    fn renders_n() {
        assert_eq!(
            render("AGENT:{n}", &snap(3, None, Some("%5"), Some("stop"))),
            "AGENT:3"
        );
    }

    #[test]
    fn renders_latest_message_fallback_to_kind() {
        assert_eq!(
            render("{n}: {latest_message}", &snap(1, None, Some("%5"), Some("stop"))),
            "1: stop"
        );
        assert_eq!(
            render(
                "{n}: {latest_message}",
                &snap(1, Some("hi"), Some("%5"), Some("stop"))
            ),
            "1: hi"
        );
    }

    #[test]
    fn unknown_placeholder_passes_through() {
        assert_eq!(
            render("{n} {wat}", &snap(1, None, Some("%5"), Some("stop"))),
            "1 {wat}"
        );
    }

    #[test]
    fn unterminated_brace_passes_through() {
        assert_eq!(
            render("AGENT:{n", &snap(1, None, Some("%5"), Some("stop"))),
            "AGENT:{n"
        );
    }
}
