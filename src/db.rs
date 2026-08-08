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
CREATE TABLE IF NOT EXISTS holds (
  pane_id    TEXT PRIMARY KEY,
  expires_at TEXT NOT NULL
);
"#;

const META_CURSOR: &str = "cursor";

/// SQL predicate: "pane `n.pane_id` is NOT currently held". Takes one bound
/// parameter, the current time as RFC3339 UTC (same format as `expires_at`, so
/// plain string comparison is chronological).
const NOT_HELD: &str =
    "NOT EXISTS (SELECT 1 FROM holds h WHERE h.pane_id = n.pane_id AND h.expires_at > ?)";

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

/// Replace any control character (tab/CR/LF as well as ESC/BEL/other C0/C1 and
/// DEL) with a single space so TSV output and column layout stay sane and the
/// message cannot smuggle terminal escape sequences into the tmux status bar or
/// `list` output.
pub fn sanitize_message(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[derive(Debug)]
pub struct Notification {
    pub id: i64,
    pub created_at: String,
    pub pane_id: String,
    pub kind: String,
    pub message: Option<String>,
    /// The pane is under an unexpired hold, so `status` is not advertising it.
    /// `list` still shows the row — a held notification must be findable, or a
    /// silent status bar and a non-empty queue cannot be told apart.
    pub held: bool,
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

/// Ack a pane. This also releases any hold on it: an ack means either the user
/// intervened here or the pane started doing real work again, and in both cases
/// "self-driving, don't advertise me" no longer holds. It is what keeps a hold
/// from outliving the wait that justified it — the hook that acks on
/// PostToolUse re-places the hold only when the tool it just ran started more
/// background work.
pub fn delete_pane(conn: &Connection, pane_id: &str) -> Result<usize> {
    let n = conn.execute(
        "DELETE FROM notifications WHERE pane_id = ?1",
        params![pane_id],
    )?;
    clear_hold(conn, pane_id)?;
    if get_cursor(conn)?.as_deref() == Some(pane_id) {
        clear_cursor(conn)?;
    }
    Ok(n)
}

pub fn delete_all(conn: &Connection) -> Result<usize> {
    let n = conn.execute("DELETE FROM notifications", [])?;
    clear_all_holds(conn)?;
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

/// RFC3339 UTC timestamp `secs` seconds from now — the expiry of a hold lease.
pub fn expiry_iso8601(secs: i64) -> String {
    (Utc::now() + chrono::Duration::seconds(secs)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Place (or refresh) a hold on a pane until `expires_at`.
///
/// A hold means "this pane is self-driving": it is waiting on machinery it
/// started itself (a backgrounded command, a spawned agent), not on the user.
/// Holds are a *read-time* filter — `push` still records every notification it
/// is handed, the hold only stops that pane from being advertised. When the
/// lease expires the suppressed notification reappears, so the failure mode of
/// a leaked hold is a late notification, never a lost one.
pub fn set_hold(conn: &Connection, pane_id: &str, expires_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO holds (pane_id, expires_at) VALUES (?1, ?2)
         ON CONFLICT(pane_id) DO UPDATE SET expires_at = excluded.expires_at",
        params![pane_id, expires_at],
    )?;
    prune_expired_holds(conn)?;
    Ok(())
}

pub fn clear_hold(conn: &Connection, pane_id: &str) -> Result<usize> {
    let n = conn.execute("DELETE FROM holds WHERE pane_id = ?1", params![pane_id])?;
    Ok(n)
}

pub fn clear_all_holds(conn: &Connection) -> Result<usize> {
    let n = conn.execute("DELETE FROM holds", [])?;
    Ok(n)
}

/// Drop lapsed leases. Correctness never depends on this (every read compares
/// `expires_at` against now); it just keeps the table from accumulating rows
/// for panes that are never held again. Called from the write path only, so
/// the 2-second status poll stays read-only.
fn prune_expired_holds(conn: &Connection) -> Result<usize> {
    let n = conn.execute(
        "DELETE FROM holds WHERE expires_at <= ?1",
        params![now_iso8601()],
    )?;
    Ok(n)
}

/// Snapshot of pending notifications, excluding panes that are currently held.
///
/// `only_pane = Some(p)` restricts the snapshot to a single pane: `n` becomes
/// 0 or 1 and the "latest" fields describe that pane. This is what lets the
/// status bar answer "is the pane I'm currently in waiting on me?" — combined
/// with the `n == 0 → empty string` rule in `format::render`, a per-pane probe
/// prints nothing unless that exact pane is pending.
///
/// Held panes are filtered out here rather than at `push` time so the record
/// survives the hold; see `set_hold`.
pub fn status_snapshot(conn: &Connection, only_pane: Option<&str>) -> Result<StatusSnapshot> {
    let mut binds: Vec<String> = vec![now_iso8601()];
    let pane_clause = match only_pane {
        Some(p) => {
            binds.push(p.to_string());
            " AND n.pane_id = ?"
        }
        None => "",
    };
    let n: i64 = conn.query_row(
        &format!("SELECT COUNT(DISTINCT n.pane_id) FROM notifications n WHERE {NOT_HELD}{pane_clause}"),
        rusqlite::params_from_iter(binds.iter()),
        |r| r.get(0),
    )?;
    if n == 0 {
        return Ok(StatusSnapshot::default());
    }
    let (latest_message, latest_pane, latest_kind): (Option<String>, Option<String>, Option<String>) =
        conn.query_row(
            &format!(
                "SELECT n.message, n.pane_id, n.kind FROM notifications n
                 WHERE {NOT_HELD}{pane_clause} ORDER BY n.id DESC LIMIT 1"
            ),
            rusqlite::params_from_iter(binds.iter()),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
    Ok(StatusSnapshot {
        n: n as usize,
        latest_message,
        latest_pane,
        latest_kind,
    })
}

/// Every pending notification, held or not. Unlike `status_snapshot` this does
/// not filter — `list` is the "show me everything" view — but each row carries
/// the pane's hold state so the caller can mark it.
pub fn list_all(conn: &Connection) -> Result<Vec<Notification>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT n.id, n.created_at, n.pane_id, n.kind, n.message, NOT ({NOT_HELD})
         FROM notifications n ORDER BY n.created_at ASC, n.id ASC"
    ))?;
    let rows = stmt
        .query_map(params![now_iso8601()], |r| {
            Ok(Notification {
                id: r.get(0)?,
                created_at: r.get(1)?,
                pane_id: r.get(2)?,
                kind: r.get(3)?,
                message: r.get(4)?,
                held: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Panes ordered newest-first by MIN(id) per pane, excluding held panes.
/// "Newest" = most-recently-entered-the-queue. Re-notifications on an existing
/// pane do NOT promote it; its position is pinned to its first notification.
///
/// Held panes are dropped so `next` / `prev` never strand the user in a pane
/// that the status bar is not even advertising. A cursor left pointing at a
/// pane that has since been held is simply not found in this list, which the
/// existing stale-cursor path already treats as "re-enter at the top".
pub fn ordered_panes(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT n.pane_id FROM notifications n WHERE {NOT_HELD}
         GROUP BY n.pane_id ORDER BY MIN(n.id) DESC"
    ))?;
    let rows = stmt
        .query_map(params![now_iso8601()], |r| r.get::<_, String>(0))?
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
///
/// `current = Some(p)` is the pane the user is sitting in: don't strand them by
/// returning the pane they're already on. If the computed target is `p` and
/// another pane is pending, step once more (pane_id is unique per group, so `p`
/// appears at most once → one extra step always clears it). If `p` is the only
/// pending pane, return it but flag `wrapped` — there is nowhere else to go,
/// which is exactly the "you've seen everything" signal.
pub fn next_pane(conn: &Connection, current: Option<&str>) -> Result<Option<CycleStep>> {
    let panes = ordered_panes(conn)?;
    if panes.is_empty() {
        return Ok(None);
    }
    let cursor = get_cursor(conn)?;
    let len = panes.len();
    let (mut idx, mut wrapped) =
        match cursor.as_deref().and_then(|c| panes.iter().position(|p| p == c)) {
            Some(i) => {
                let new_idx = (i + 1) % len;
                (new_idx, new_idx == 0)
            }
            None => (0, false),
        };
    if current == Some(panes[idx].as_str()) {
        if len > 1 {
            let new_idx = (idx + 1) % len;
            wrapped |= new_idx == 0;
            idx = new_idx;
        } else {
            wrapped = true;
        }
    }
    set_cursor(conn, &panes[idx])?;
    Ok(Some(CycleStep { pane: panes[idx].clone(), wrapped }))
}

/// Retreat cursor toward newer panes. Symmetric to next_pane.
/// Null/invalid cursor → entry at oldest (bottom).
/// `current` has the same skip-the-pane-you're-on semantics as next_pane.
pub fn prev_pane(conn: &Connection, current: Option<&str>) -> Result<Option<CycleStep>> {
    let panes = ordered_panes(conn)?;
    if panes.is_empty() {
        return Ok(None);
    }
    let cursor = get_cursor(conn)?;
    let len = panes.len();
    let (mut idx, mut wrapped) =
        match cursor.as_deref().and_then(|c| panes.iter().position(|p| p == c)) {
            Some(i) => {
                let new_idx = (i + len - 1) % len;
                (new_idx, new_idx == len - 1)
            }
            None => (len - 1, false),
        };
    if current == Some(panes[idx].as_str()) {
        if len > 1 {
            let new_idx = (idx + len - 1) % len;
            wrapped |= new_idx == len - 1;
            idx = new_idx;
        } else {
            wrapped = true;
        }
    }
    set_cursor(conn, &panes[idx])?;
    Ok(Some(CycleStep { pane: panes[idx].clone(), wrapped }))
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
    fn sanitize_strips_terminal_escapes() {
        // ESC-based CSI/OSC sequences and BEL must not survive into terminal output.
        assert_eq!(sanitize_message("\u{1b}[31mred\u{1b}[0m"), " [31mred [0m");
        assert_eq!(sanitize_message("a\u{7}b"), "a b");
        assert_eq!(sanitize_message("\u{1b}]0;title\u{7}"), " ]0;title ");
        // Non-control unicode is preserved.
        assert_eq!(sanitize_message("🔔 ok"), "🔔 ok");
    }

    const FUTURE: &str = "2099-01-01T00:00:00Z";
    const PAST: &str = "2000-01-01T00:00:00Z";

    #[test]
    fn hold_hides_pane_from_status_but_keeps_the_row() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        insert(&conn, "2026-04-20T10:30:01Z", "%7", "stop", None).unwrap();
        set_hold(&conn, "%5", FUTURE).unwrap();
        // Global status counts only the unheld pane...
        let snap = status_snapshot(&conn, None).unwrap();
        assert_eq!(snap.n, 1);
        assert_eq!(snap.latest_pane.as_deref(), Some("%7"));
        // ...the per-pane probe for the held pane goes empty (drives the "is the
        // pane I'm in waiting on me?" badge back off)...
        assert_eq!(status_snapshot(&conn, Some("%5")).unwrap().n, 0);
        // ...but the notification itself is still recorded.
        let rows = list_all(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().find(|r| r.pane_id == "%5").unwrap().held);
        assert!(!rows.iter().find(|r| r.pane_id == "%7").unwrap().held);
    }

    #[test]
    fn lapsed_hold_lets_the_notification_resurface() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        // Insert directly: set_hold prunes anything already expired.
        conn.execute(
            "INSERT INTO holds (pane_id, expires_at) VALUES (?1, ?2)",
            params!["%5", PAST],
        )
        .unwrap();
        // A leaked hold must fail toward a late notification, never a lost one.
        assert_eq!(status_snapshot(&conn, None).unwrap().n, 1);
        assert!(!list_all(&conn).unwrap()[0].held);
    }

    #[test]
    fn set_hold_refreshes_the_lease() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        set_hold(&conn, "%5", "2030-01-01T00:00:00Z").unwrap();
        set_hold(&conn, "%5", FUTURE).unwrap();
        let expires: String = conn
            .query_row("SELECT expires_at FROM holds WHERE pane_id = '%5'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(expires, FUTURE);
    }

    #[test]
    fn held_panes_drop_out_of_the_cycle() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]); // order C, B, A
        set_hold(&conn, "%C", FUTURE).unwrap();
        assert_eq!(ordered_panes(&conn).unwrap(), vec!["%B", "%A"]);
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%B", false)));
    }

    #[test]
    fn cycle_reports_empty_when_every_pending_pane_is_held() {
        let conn = mem();
        seed(&conn, &["%A"]);
        set_hold(&conn, "%A", FUTURE).unwrap();
        assert_eq!(next_pane(&conn, None).unwrap(), None);
        assert_eq!(prev_pane(&conn, None).unwrap(), None);
    }

    #[test]
    fn ack_pane_releases_the_hold() {
        let conn = mem();
        insert(&conn, "2026-04-20T10:30:00Z", "%5", "stop", None).unwrap();
        set_hold(&conn, "%5", FUTURE).unwrap();
        delete_pane(&conn, "%5").unwrap();
        assert_eq!(clear_hold(&conn, "%5").unwrap(), 0, "hold should already be gone");
        // A notification pushed after the ack is advertised again immediately.
        insert(&conn, "2026-04-20T10:31:00Z", "%5", "stop", None).unwrap();
        assert_eq!(status_snapshot(&conn, None).unwrap().n, 1);
    }

    #[test]
    fn ack_all_releases_every_hold() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        set_hold(&conn, "%A", FUTURE).unwrap();
        set_hold(&conn, "%B", FUTURE).unwrap();
        delete_all(&conn).unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM holds", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn hold_without_pending_notifications_is_harmless() {
        let conn = mem();
        set_hold(&conn, "%Z", FUTURE).unwrap();
        assert_eq!(status_snapshot(&conn, None).unwrap().n, 0);
        // The hold still applies to a notification that arrives later — this is
        // the ordering the Stop hook relies on (hold placed before the Stop).
        insert(&conn, "2026-04-20T10:30:00Z", "%Z", "stop", None).unwrap();
        assert_eq!(status_snapshot(&conn, None).unwrap().n, 0);
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
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%C", false)));
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%C"));
    }

    #[test]
    fn prev_null_cursor_enters_at_bottom() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A. First prev → A (bottom), not wrapped.
        assert_eq!(prev_pane(&conn, None).unwrap(), Some(step("%A", false)));
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%A"));
    }

    #[test]
    fn next_walks_toward_older_and_wraps() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A.
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%C", false)));
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%B", false)));
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%A", false)));
        // wrap
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%C", true)));
    }

    #[test]
    fn prev_walks_toward_newer_and_wraps() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]);
        // Order: C, B, A. First prev enters at A, then B, C, wrap to A.
        assert_eq!(prev_pane(&conn, None).unwrap(), Some(step("%A", false)));
        assert_eq!(prev_pane(&conn, None).unwrap(), Some(step("%B", false)));
        assert_eq!(prev_pane(&conn, None).unwrap(), Some(step("%C", false)));
        // wrap
        assert_eq!(prev_pane(&conn, None).unwrap(), Some(step("%A", true)));
    }

    #[test]
    fn single_pane_cycles_report_wrap_after_first_advance() {
        let conn = mem();
        seed(&conn, &["%A"]);
        // First next enters at top (not wrap).
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%A", false)));
        // Second next: len=1, new_idx = 0 = entry, prior cursor valid → wrapped.
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%A", true)));
        // Same for prev direction.
        assert_eq!(prev_pane(&conn, None).unwrap(), Some(step("%A", true)));
    }

    #[test]
    fn empty_pending_returns_none() {
        let conn = mem();
        assert_eq!(next_pane(&conn, None).unwrap(), None);
        assert_eq!(prev_pane(&conn, None).unwrap(), None);
        assert_eq!(get_cursor(&conn).unwrap(), None);
    }

    #[test]
    fn ack_pane_clears_cursor_if_pointing_there() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn, None).unwrap(); // cursor=%B (top: order B, A)
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%B"));
        delete_pane(&conn, "%B").unwrap();
        assert_eq!(get_cursor(&conn).unwrap(), None);
    }

    #[test]
    fn ack_other_pane_keeps_cursor() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn, None).unwrap(); // cursor=%B
        delete_pane(&conn, "%A").unwrap();
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%B"));
    }

    #[test]
    fn ack_all_clears_cursor() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn, None).unwrap();
        delete_all(&conn).unwrap();
        assert_eq!(get_cursor(&conn).unwrap(), None);
    }

    #[test]
    fn push_does_not_reset_cursor() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        next_pane(&conn, None).unwrap(); // order B, A; cursor=%B
        next_pane(&conn, None).unwrap(); // cursor=%A
        // New push to %C — cursor must stay on %A.
        insert(&conn, "2026-04-20T10:00:10Z", "%C", "stop", None).unwrap();
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%A"));
        // Ordering: MIN(id) per pane A=1, B=2, C=10. DESC: C, B, A.
        // next from A wraps to C (top).
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%C", true)));
    }

    #[test]
    fn stale_cursor_re_enters_at_top_for_next() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]);
        set_cursor(&conn, "%GHOST").unwrap();
        // Cursor pane not in pending → treat as null → next enters at top (not wrap).
        assert_eq!(next_pane(&conn, None).unwrap(), Some(step("%B", false)));
    }

    #[test]
    fn next_skips_current_pane_on_entry() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]); // order C, B, A
        // User sits in the newest pending pane C; entry would land on C → skip to B.
        assert_eq!(next_pane(&conn, Some("%C")).unwrap(), Some(step("%B", false)));
        assert_eq!(get_cursor(&conn).unwrap().as_deref(), Some("%B"));
    }

    #[test]
    fn next_skips_current_pane_mid_cycle_carrying_wrap() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]); // order C, B, A
        set_cursor(&conn, "%A").unwrap(); // advance from oldest wraps to top C
        // Wrap target is C but the user is in C → skip to B; wrap flag is kept.
        assert_eq!(next_pane(&conn, Some("%C")).unwrap(), Some(step("%B", true)));
    }

    #[test]
    fn next_current_not_pending_no_skip() {
        let conn = mem();
        seed(&conn, &["%A", "%B"]); // order B, A
        // User is in a pane with no pending notification → normal entry at top.
        assert_eq!(next_pane(&conn, Some("%Z")).unwrap(), Some(step("%B", false)));
    }

    #[test]
    fn next_single_current_pane_reports_wrapped() {
        let conn = mem();
        seed(&conn, &["%A"]);
        // The only pending pane is the one the user is in → return it, flag wrapped.
        assert_eq!(next_pane(&conn, Some("%A")).unwrap(), Some(step("%A", true)));
    }

    #[test]
    fn prev_skips_current_pane_on_entry() {
        let conn = mem();
        seed(&conn, &["%A", "%B", "%C"]); // order C, B, A; prev entry = oldest A
        // User in oldest pending pane A; entry would land on A → skip to B.
        assert_eq!(prev_pane(&conn, Some("%A")).unwrap(), Some(step("%B", false)));
    }

    #[test]
    fn prev_single_current_pane_reports_wrapped() {
        let conn = mem();
        seed(&conn, &["%A"]);
        assert_eq!(prev_pane(&conn, Some("%A")).unwrap(), Some(step("%A", true)));
    }
}
