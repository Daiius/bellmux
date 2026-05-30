mod db;
mod format;
mod snippets;
mod validate;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::process::Command;

const ABOUT: &str = "Minimal notification layer bridging coding-agent hooks, tmux, and SQLite.";

#[derive(Parser)]
#[command(name = "bellmux", version, about = ABOUT)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record a notification. Reads optional JSON payload from stdin.
    Push {
        #[arg(long)]
        kind: Kind,
        #[arg(long = "pane-id")]
        pane_id: String,
    },
    /// Acknowledge (delete) all notifications for the given pane.
    AckPane {
        #[arg(long = "pane-id")]
        pane_id: String,
    },
    /// Acknowledge (delete) every pending notification.
    AckAll,
    /// Drop notifications belonging to a pane that no longer exists.
    PrunePane {
        #[arg(long = "pane-id")]
        pane_id: String,
    },
    /// Print a status string suited for tmux status bars.
    Status {
        /// Template with {n}, {latest_message}, {latest_pane}. Empty string is
        /// printed when nothing is pending.
        #[arg(long, default_value = "AGENT:{n}")]
        format: String,
        /// Restrict the snapshot to a single pane. Output is empty unless that
        /// pane has a pending notification — lets the status bar distinguish
        /// "the pane I'm in is waiting" from "some other pane is".
        #[arg(long = "only-pane")]
        only_pane: Option<String>,
    },
    /// List pending notifications.
    List {
        #[arg(long, conflicts_with = "json")]
        tsv: bool,
        #[arg(long)]
        json: bool,
    },
    /// Advance the cycle cursor toward older pending panes and print the new cursor's pane_id.
    /// Entry (no cursor) → newest pane; wraps oldest → newest.
    Next {
        /// The pane you are currently in (pass tmux's #{pane_id}). When the
        /// target would be this same pane, skip to the next pending pane instead
        /// so the jump is never a no-op. If it is the only pending pane it is
        /// returned with the ` wrapped` tag.
        #[arg(long = "current")]
        current: Option<String>,
    },
    /// Retreat the cycle cursor toward newer pending panes and print the new cursor's pane_id.
    /// Entry (no cursor) → oldest pane; wraps newest → oldest.
    Prev {
        /// Same skip-the-pane-you're-on behaviour as `next --current`.
        #[arg(long = "current")]
        current: Option<String>,
    },
    /// Write BEL (\x07) to every login tty of the current user (via `who`).
    /// Reaches the outer terminal regardless of tmux session/client topology.
    /// Best-effort: silently skips ttys we cannot open.
    Bell,
    /// Print setup snippets for tmux/coding-agent hooks.
    Init {
        /// One of: widget, fullbar, overlay, dot, popup-simple,
        /// popup-enriched, keybinds, tmux-hook, claude-hooks, codex-hooks.
        /// Omit to print everything.
        #[arg(long)]
        preset: Option<String>,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum Kind {
    Notification,
    Stop,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Notification => "notification",
            Kind::Stop => "stop",
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("bellmux: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Cmd::Push { kind, pane_id } => cmd_push(&pane_id, kind),
        Cmd::AckPane { pane_id } => cmd_ack_pane(&pane_id),
        Cmd::AckAll => cmd_ack_all(),
        Cmd::PrunePane { pane_id } => cmd_prune_pane(&pane_id),
        Cmd::Status { format, only_pane } => cmd_status(&format, only_pane.as_deref()),
        Cmd::List { tsv, json } => cmd_list(tsv, json),
        Cmd::Next { current } => cmd_next(current.as_deref()),
        Cmd::Prev { current } => cmd_prev(current.as_deref()),
        Cmd::Bell => cmd_bell(),
        Cmd::Init { preset } => cmd_init(preset.as_deref()),
    }
}

fn cmd_push(pane_id: &str, kind: Kind) -> Result<()> {
    validate::pane_id(pane_id)?;
    let message = read_stdin_message();
    let conn = db::open()?;
    db::insert(
        &conn,
        &db::now_iso8601(),
        pane_id,
        kind.as_str(),
        message.as_deref(),
    )?;
    Ok(())
}

fn cmd_ack_pane(pane_id: &str) -> Result<()> {
    validate::pane_id(pane_id)?;
    let conn = db::open()?;
    db::delete_pane(&conn, pane_id)?;
    Ok(())
}

fn cmd_ack_all() -> Result<()> {
    let conn = db::open()?;
    db::delete_all(&conn)?;
    Ok(())
}

fn cmd_prune_pane(pane_id: &str) -> Result<()> {
    validate::pane_id(pane_id)?;
    let conn = db::open()?;
    db::delete_pane(&conn, pane_id)?;
    Ok(())
}

fn cmd_status(template: &str, only_pane: Option<&str>) -> Result<()> {
    if let Some(pane) = only_pane {
        validate::pane_id(pane)?;
    }
    let conn = db::open()?;
    let snap = db::status_snapshot(&conn, only_pane)?;
    let out = format::render(template, &snap);
    if !out.is_empty() {
        // No trailing newline: tmux #(...) substitution adds whitespace itself.
        print!("{out}");
    }
    Ok(())
}

fn cmd_list(tsv: bool, json: bool) -> Result<()> {
    let conn = db::open()?;
    let rows = db::list_all(&conn)?;
    if json {
        print_list_json(&rows);
    } else if tsv {
        print_list_tsv(&rows);
    } else {
        print_list_human(&rows);
    }
    Ok(())
}

fn cmd_next(current: Option<&str>) -> Result<()> {
    if let Some(pane) = current {
        validate::pane_id(pane)?;
    }
    let conn = db::open()?;
    if let Some(step) = db::next_pane(&conn, current)? {
        print_cycle_step(&step);
    }
    Ok(())
}

fn cmd_prev(current: Option<&str>) -> Result<()> {
    if let Some(pane) = current {
        validate::pane_id(pane)?;
    }
    let conn = db::open()?;
    if let Some(step) = db::prev_pane(&conn, current)? {
        print_cycle_step(&step);
    }
    Ok(())
}

/// One line: `pane_id` plus optional ` wrapped` tag. The keybind consumes
/// the first whitespace-separated field as the pane and checks for a second
/// field to decide whether to display a "cycled through all" message.
fn print_cycle_step(step: &db::CycleStep) {
    if step.wrapped {
        println!("{} wrapped", step.pane);
    } else {
        println!("{}", step.pane);
    }
}

fn cmd_bell() -> Result<()> {
    ring_bell();
    Ok(())
}

fn ring_bell() {
    for tty in user_login_ttys() {
        if let Ok(mut f) = OpenOptions::new().write(true).open(&tty) {
            let _ = f.write_all(b"\x07");
        }
    }
}

/// Enumerate `/dev/<tty>` paths from `who` for the current `$USER`, deduped.
/// `who` is POSIX and reads the same utmpx DB libc::getutxent would; shelling
/// out keeps the binary free of FFI for a one-off best-effort path.
fn user_login_ttys() -> Vec<String> {
    let user = match std::env::var("USER") {
        Ok(u) if !u.is_empty() => u,
        _ => return Vec::new(),
    };
    let output = match Command::new("who").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let mut ttys: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let line_user = it.next()?;
            if line_user != user {
                return None;
            }
            let tty = it.next()?;
            Some(format!("/dev/{tty}"))
        })
        .collect();
    ttys.sort();
    ttys.dedup();
    ttys
}

fn cmd_init(preset: Option<&str>) -> Result<()> {
    let body = match preset {
        None => snippets::all(),
        Some(name) => snippets::by_name(name)
            .ok_or_else(|| anyhow!("unknown preset: {name}"))?
            .to_string(),
    };
    print!("{body}");
    Ok(())
}

/// Extract the optional top-level `message` string from stdin JSON, for
/// display in the status bar and `list`. Returns None for empty or non-JSON
/// stdin. bellmux interprets no other field: which events are worth surfacing
/// is decided by the hook matcher, not here.
fn read_stdin_message() -> Option<String> {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&buf).ok()?;
    value
        .get("message")
        .and_then(|v| v.as_str())
        .map(db::sanitize_message)
}

fn print_list_human(rows: &[db::Notification]) {
    let now = chrono::Utc::now();
    for r in rows {
        let when = db::relative_time(&r.created_at, now);
        let msg = r.message.as_deref().unwrap_or("");
        println!("{:<6} {:<10} {:<14} {}", r.pane_id, when, r.kind, msg);
    }
}

fn print_list_tsv(rows: &[db::Notification]) {
    for r in rows {
        let msg = r.message.as_deref().unwrap_or("");
        println!("{}\t{}\t{}\t{}", r.pane_id, r.created_at, r.kind, msg);
    }
}

fn print_list_json(rows: &[db::Notification]) {
    let arr: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "pane_id": r.pane_id,
                "created_at": r.created_at,
                "kind": r.kind,
                "message": r.message,
            })
        })
        .collect();
    println!("{}", serde_json::Value::Array(arr));
}
