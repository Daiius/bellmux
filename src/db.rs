use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags};
use std::path::PathBuf;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS notifications (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at  TEXT NOT NULL,
  pane_id     TEXT NOT NULL,
  kind        TEXT NOT NULL,
  message     TEXT
);
CREATE INDEX IF NOT EXISTS idx_pane ON notifications(pane_id);
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

const META_CURSOR: &str = "cursor";

pub fn db_path() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("BELLMUX_DB_PATH") {
        return Ok(PathBuf::from(custom));
    }
    let state = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .context("could not resolve XDG_STATE_HOME or fallback")?;
    Ok(state.join("bellmux").join("notifications.db"))
}

pub fn open() -> Result<Connection> {
    let path = db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("failed to open database {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(3000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

pub fn now_iso8601() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Replace tab/CR/LF with a single space so TSV output and column layout stay sane.
pub fn sanitize_message(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\t' | '\r' | '\n' => ' ',
            other => other,
        })
        .collect()
}

#[derive(Debug)]
pub struct Notification {
    pub id: i64,
    pub created_at: String,
    pub pane_id: String,
    pub kind: String,
    pub message: Option<String>,
}

#[derive(Debug, Default)]
pub struct StatusSnapshot {
    pub n: usize,
    pub latest_message: Option<String>,
    pub latest_pane: Option<String>,
    pub latest_kind: Option<String>,
}

pub fn insert(
    conn: &Connection,
    created_at: &str,
    pane_id: &str,
    kind: &str,
    message: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO notifications (created_at, pane_id, kind, message) VALUES (?1, ?2, ?3, ?4)",
        params![created_at, pane_id, kind, message],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_pane(conn: &Connection, pane_id: &str) -> Result<usize> {
    let n = conn.execute(
        "DELETE FROM notifications WHERE pane_id = ?1",
        params![pane_id],
    )?;
    if get_cursor(conn)?.as_deref() == Some(pane_id) {
        clear_cursor(conn)?;
    }
    Ok(n)
}

pub fn delete_all(conn: &Connection) -> Result<usize> {
    let n = conn.execute("DELETE FROM notifications", [])?;
    clear_cursor(conn)?;
    Ok(n)
}

pub fn get_cursor(conn: &Connection) -> Result<Option<String>> {
    let r = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![META_CURSOR],
            |r| r.get::<_, String>(0),
        )
        .ok();
    Ok(r)
}

pub fn set_cursor(conn: &Connection, pane_id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![META_CURSOR, pane_id],
    )?;
    Ok(())
}

pub fn clear_cursor(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM meta WHERE key = ?1", params![META_CURSOR])?;
    Ok(())
}

/// Snapshot of pending notifications.
///
/// `only_pane = Some(p)` restricts the snapshot to a single pane: `n` becomes
/// 0 or 1 and the "latest" fields describe that pane. This is what lets the
/// status bar answer "is the pane I'm currently in waiting on me?" — combined
/// with the `n == 0 → empty string` rule in `format::render`, a per-pane probe
/// prints nothing unless that exact pane is pending.
pub fn status_snapshot(conn: &Connection, only_pane: Option<&str>) -> Result<StatusSnapshot> {
    let n: i64 = match only_pane {
        Some(p) => conn.query_row(
            "SELECT COUNT(DISTINCT pane_id) FROM notifications WHERE pane_id = ?1",
            params![p],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(DISTINCT pane_id) FROM notifications",
            [],
            |r| r.get(0),
        )?,
    };
    if n == 0 {
        return Ok(StatusSnapshot::default());
    }
    let map_latest =
        |r: &rusqlite::Row| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
    let (latest_message, latest_pane, latest_kind): (Option<String>, Option<String>, Option<String>) =
        match only_pane {
            Some(p) => conn.query_row(
                "SELECT message, pane_id, kind FROM notifications WHERE pane_id = ?1 ORDER BY id DESC LIMIT 1",
                params![p],
                map_latest,
            )?,
            None => conn.query_row(
                "SELECT message, pane_id, kind FROM notifications ORDER BY id DESC LIMIT 1",
                [],
                map_latest,
            )?,
        };
    Ok(StatusSnapshot {
        n: n as usize,
        latest_message,
        latest_pane,
        latest_kind,
    })
}

pub fn list_all(conn: &Connection) -> Result<Vec<Notification>> {
    let mut stmt = conn.prepare(
        "SELECT id, created_at, pane_id, kind, message FROM notifications ORDER BY created_at ASC, id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Notification {
                id: r.get(0)?,
                created_at: r.get(1)?,
                pane_id: r.get(2)?,
                kind: r.get(3)?,
                message: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Panes ordered newest-first by MIN(id) per pane.
/// "Newest" = most-recently-entered-the-queue. Re-notifications on an existing
/// pane do NOT promote it; its position is pinned to its first notification.
pub fn ordered_panes(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT pane_id FROM notifications GROUP BY pane_id ORDER BY MIN(id) DESC",
    )?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One step of the cycle: the new cursor pane, plus a `wrapped` flag that
/// signals "you've been through all pending" — true when the advance crossed
/// the cycle boundary (from bottom to top for next, top to bottom for prev),
/// or when there's only one pending pane and the cursor was already there.
/// The flag is always false on the very first advance from a null cursor.
#[derive(Debug, PartialEq, Eq)]
pub struct CycleStep {
    pub pane: String,
    pub wrapped: bool,
}

/// Advance cursor toward older panes. Empty pending → None.
/// Null cursor (or cursor pointing to a pane no longer pending) → entry at newest (top).
/// Valid cursor → (position + 1) % len, wrapping oldest → newest.
pub fn next_pane(conn: &Connection) -> Result<Option<CycleStep>> {
    let panes = ordered_panes(conn)?;
    if panes.is_empty() {
        return Ok(None);
    }
    let cursor = get_cursor(conn)?;
    let len = panes.len();
    let (new_cursor, wrapped) = match cursor.as_deref().and_then(|c| panes.iter().position(|p| p == c)) {
        Some(idx) => {
            let new_idx = (idx + 1) % len;
            (panes[new_idx].clone(), new_idx == 0)
        }
        None => (panes[0].clone(), false),
    };
    set_cursor(conn, &new_cursor)?;
    Ok(Some(CycleStep { pane: new_cursor, wrapped }))
}

/// Retreat cursor toward newer panes. Symmetric to next_pane.
/// Null/invalid cursor → entry at oldest (bottom).
pub fn prev_pane(conn: &Connection) -> Result<Option<CycleStep>> {
    let panes = ordered_panes(conn)?;
    if panes.is_empty() {
        return Ok(None);
    }
    let cursor = get_cursor(conn)?;
    let len = panes.len();
    let (new_cursor, wrapped) = match cursor.as_deref().and_then(|c| panes.iter().position(|p| p == c)) {
        Some(idx) => {
            let new_idx = (idx + len - 1) % len;
            (panes[new_idx].clone(), new_idx == len - 1)
        }
        None => (panes[len - 1].clone(), false),
    };
    set_cursor(conn, &new_cursor)?;
    Ok(Some(CycleStep { pane: new_cursor, wrapped }))
}

/// Render relative-time-ago string ("2m ago", "10s ago").
pub fn relative_time(created_at: &str, now: DateTime<Utc>) -> String {
    let parsed: Option<DateTime<Utc>> = DateTime::parse_from_rfc3339(created_at)
        .ok()
        .map(|d| d.with_timezone(&Utc));
    let parsed = match parsed {
        Some(p) => p,
        None => return created_at.to_string(),
    };
    let elapsed = now.signed_duration_since(parsed);
    let secs = elapsed.num_seconds();
    if secs < 0 {
        return "future".to_string();
    }
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    #[test]
    fn insert_and_status() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        insert(
            &conn,
            "2026-04-20T10:30:01Z",
            "%5",
            "notification",
            Some("hi"),
        )
        .unwrap();
        insert(&conn, "2026-04-20T10:30:02Z", "%7", "stop", None).unwrap();
        let snap = status_snapshot(&conn, None).unwrap();
        assert_eq!(snap.n, 2);
        assert_eq!(snap.latest_pane.as_deref(), Some("%7"));
    }

    #[test]
    fn status_only_pane_filters() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        insert(&conn, "2026-04-20T10:30:01Z", "%5", "notification", Some("hi")).unwrap();
        insert(&conn, "2026-04-20T10:30:02Z", "%7", "stop", None).unwrap();
        // Pending pane → n==1, latest is that pane's most recent row.
        let here = status_snapshot(&conn, Some("%5")).unwrap();
        assert_eq!(here.n, 1);
        assert_eq!(here.latest_pane.as_deref(), Some("%5"));
        assert_eq!(here.latest_message.as_deref(), Some("hi"));
        // Pane with no pending → empty snapshot (drives the empty status string).
        let absent = status_snapshot(&conn, Some("%99")).unwrap();
        assert_eq!(absent.n, 0);
        assert!(absent.latest_pane.is_none());
    }

    #[test]
    fn delete_pane_only() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        insert(&conn, "2026-04-20T10:30:01Z", "%7", "stop", None).unwrap();
        let n = delete_pane(&conn, "%5").unwrap();
        assert_eq!(n, 1);
        let snap = status_snapshot(&conn, None).unwrap();
        assert_eq!(snap.n, 1);
        assert_eq!(snap.latest_pane.as_deref(), Some("%7"));
    }

    #[test]
    fn empty_status() {
        let conn = mem();
        let snap = status_snapshot(&conn, None).unwrap();
        assert_eq!(snap.n, 0);
        assert!(snap.latest_pane.is_none());
    }

    #[test]
    fn sanitize_strips_separators() {
        assert_eq!(sanitize_message("a\tb\nc\rd"), "a b c d");
        assert_eq!(sanitize_message("plain"), "plain");
    }

    #[test]
    fn relative_time_formats() {
        let now: DateTime<Utc> = "2026-04-20T10:31:00Z".parse().unwrap();
        assert_eq!(relative_time("2026-04-20T10:30:55Z", now), "5s ago");
        assert_eq!(relative_time("2026-04-20T10:29:00Z", now), "2m ago");
        assert_eq!(relative_time("2026-04-20T08:31:00Z", now), "2h ago");
    }

    fn seed(conn: &Connection, pane_ids: &[&str]) {
        for (i, p) in pane_ids.iter().enumerate() {
            let ts = format!("2026-04-20T10:00:{:02}Z", i);
            insert(conn, &ts, p, "stop", None).unwrap();
        }
    }

    #[test]
    fn ordered_panes_by_min_id_desc() {
        let conn = mem();
        // Insertion order: A, B, C, then re-notify A. MIN(id) per pane: A=1, B=2, C=3.
        // Newest-first (DESC by MIN(id)): C, B, A. Re-notify on A must NOT move it.
        seed(&conn, &["%A", "%B", "%C", "%A"]);
        let panes = ordered_panes(&conn).unwrap();
        assert_eq!(panes, vec!["%C", "%B", "%A"]);
    }

    fn step(pane: &str, wrapped: bool) -> CycleStep {
        CycleStep { pane: pane.to_string(), wrapped }
    }

    #[test]
    fn next_null_cursor_enters_at_top() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A. First next → C (top), not wrapped.
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%C", false)));
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%C"));
    }

    #[test]
    fn prev_null_cursor_enters_at_bottom() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A. First prev → A (bottom), not wrapped.
        assert_eq!(prev_pane(&conn).unwrap(), Some(step("%A", false)));
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%A"));
    }

    #[test]
    fn next_walks_toward_older_and_wraps() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A.
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%C", false)));
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%B", false)));
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%A", false)));
        // wrap
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%C", true)));
    }

    #[test]
    fn prev_walks_toward_newer_and_wraps() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A. First prev enters at A, then B, C, wrap to A.
        assert_eq!(prev_pane(&conn).unwrap(), Some(step("%A", false)));
        assert_eq!(prev_pane(&conn).unwrap(), Some(step("%B", false)));
        assert_eq!(prev_pane(&conn).unwrap(), Some(step("%C", false)));
        // wrap
        assert_eq!(prev_pane(&conn).unwrap(), Some(step("%A", true)));
    }

    #[test]
    fn single_pane_cycles_report_wrap_after_first_advance() {
        let conn = mem();
        seed(&conn, &["%A"]);
        // First next enters at top (not wrap).
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%A", false)));
        // Second next: len=1, new_idx = 0 = entry, prior cursor valid → wrapped.
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%A", true)));
        // Same for prev direction.
        assert_eq!(prev_pane(&conn).unwrap(), Some(step("%A", true)));
    }

    #[test]
    fn empty_pending_returns_none() {
        let conn = mem();
        assert_eq!(next_pane(&conn).unwrap(), None);
        assert_eq!(prev_pane(&conn).unwrap(), None);
        assert_eq!(get_cursor(&conn).unwrap(), None);
    }

    #[test]
    fn ack_pane_clears_cursor_if_pointing_there() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn).unwrap(); // cursor=%B (top: order B, A)
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%B"));
        delete_pane(&conn, "%B").unwrap();
        assert_eq!(get_cursor(&conn).unwrap(), None);
    }

    #[test]
    fn ack_other_pane_keeps_cursor() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn).unwrap(); // cursor=%B
        delete_pane(&conn, "%A").unwrap();
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%B"));
    }

    #[test]
    fn ack_all_clears_cursor() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn).unwrap();
        delete_all(&conn).unwrap();
        assert_eq!(get_cursor(&conn).unwrap(), None);
    }

    #[test]
    fn push_does_not_reset_cursor() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn).unwrap(); // order B, A; cursor=%B
        next_pane(&conn).unwrap(); // cursor=%A
        // New push to %C — cursor must stay on %A.
        insert(&conn, "2026-04-20T10:00:10Z", "%C", "stop", None).unwrap();
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%A"));
        // Ordering: MIN(id) per pane A=1, B=2, C=10. DESC: C, B, A.
        // next from A wraps to C (top).
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%C", true)));
    }

    #[test]
    fn stale_cursor_re_enters_at_top_for_next() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        set_cursor(&conn, "%GHOST").unwrap();
        // Cursor pane not in pending → treat as null → next enters at top (not wrap).
        assert_eq!(next_pane(&conn).unwrap(), Some(step("%B", false)));
    }
}
