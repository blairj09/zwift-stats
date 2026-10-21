use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use duckdb::{Connection, params};
use sha2::{Digest, Sha256};

use crate::fit;
use crate::logparse::{self, LogActivity};

pub struct Paths {
    pub zwift_dir: PathBuf,
    pub data_dir: PathBuf,
    pub legacy_db: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub logs_archived: usize,
    pub logs_total: usize,
    pub activities: usize,
    pub fit_new: usize,
    pub fit_failed: usize,
    pub fit_total: usize,
    pub legacy_rides: usize,
}

/// `2026-10-15T13:58:38Z` → `2026-10-15 13:58:38`, which DuckDB casts to TIMESTAMP (UTC)
fn ts(s: &str) -> String {
    s.trim_end_matches('Z').replacen('T', " ", 1)
}

pub fn run(conn: &Connection, paths: &Paths) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    let archive = paths.data_dir.join("logs");
    fs::create_dir_all(&archive)?;

    report.logs_archived = archive_logs(conn, &paths.zwift_dir.join("Logs"), &archive)?;
    let (logs_total, activities) = import_activities(conn, &archive)?;
    report.logs_total = logs_total;
    report.activities = activities;

    let (fit_new, fit_failed) = import_fit_files(conn, &paths.zwift_dir.join("Activities"))?;
    report.fit_new = fit_new;
    report.fit_failed = fit_failed;
    report.fit_total =
        conn.query_row("SELECT count(*) FROM fit_files WHERE start_utc IS NOT NULL", [], |r| r.get::<_, i64>(0))? as usize;
    link_unmatched_activities(conn)?;

    if let Some(legacy) = &paths.legacy_db {
        if legacy.exists() {
            report.legacy_rides = import_legacy(conn, legacy)?;
        }
    }
    Ok(report)
}

/// Copies each game log into the archive, named by its hash. Zwift keeps only its last ten
/// logs, so the archive is what lets XP history outlive Zwift's rotation.
fn archive_logs(conn: &Connection, logs_dir: &Path, archive: &Path) -> Result<usize> {
    if !logs_dir.exists() {
        eprintln!("Zwift logs folder not found: {}", logs_dir.display());
        return Ok(0);
    }
    let mut added = 0;
    for entry in fs::read_dir(logs_dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if !(name.starts_with("Log") && name.ends_with(".txt")) {
            continue;
        }
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let sha = hex::encode(Sha256::digest(&bytes));
        let dest = archive.join(format!("{sha}.txt"));
        if dest.exists() {
            continue;
        }
        fs::write(&dest, &bytes)?;
        let first_line = String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let started = first_line.split("Log Time: ").nth(1).unwrap_or_default().to_string();
        conn.execute(
            "INSERT OR REPLACE INTO log_files (sha256, original_name, log_started, bytes) VALUES (?, ?, ?, ?)",
            params![sha, name, started, bytes.len() as i64],
        )?;
        added += 1;
    }
    Ok(added)
}

fn import_activities(conn: &Connection, archive: &Path) -> Result<(usize, usize)> {
    let mut by_start: BTreeMap<String, LogActivity> = BTreeMap::new();
    let mut logs = 0;
    for entry in fs::read_dir(archive)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        logs += 1;
        // Logs can contain stray binary bytes; lossy decoding keeps the text we need
        let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
        for act in logparse::parse_log(&text) {
            let keep = by_start.get(&act.start_utc).is_none_or(|prev| act.xp >= prev.xp);
            if keep {
                by_start.insert(act.start_utc.clone(), act);
            }
        }
    }

    conn.execute_batch("BEGIN TRANSACTION")?;
    for a in by_start.values() {
        conn.execute(
            "INSERT OR REPLACE INTO activities VALUES (
                CAST(? AS TIMESTAMP), CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                ts(&a.start_utc), ts(&a.end_utc), a.sport, a.kind, a.route, a.title, a.world, a.world_id,
                a.fit_file, a.distance_m, a.elevation_m, a.calories, a.xp, a.drops, a.start_level,
                a.end_level, a.start_total_xp, a.end_total_xp, a.start_drops, a.end_drops,
                a.streak_weeks, a.week_ride_count, a.bonus_xp, a.event_name, a.ride_ons_received,
                a.ride_ons_given, a.game_version
            ],
        )?;
        conn.execute("DELETE FROM xp_sources WHERE start_utc = CAST(? AS TIMESTAMP)", params![ts(&a.start_utc)])?;
        // Some XP (level-ups, bonuses) is granted without an [XP] line; keep the books balanced
        let itemized: i64 = a.xp_by_source.values().sum();
        let unitemized = (a.xp - itemized > 0).then(|| ("UNITEMIZED".to_string(), a.xp - itemized));
        for (source, xp) in a.xp_by_source.iter().map(|(k, v)| (k.clone(), *v)).chain(unitemized) {
            conn.execute(
                "INSERT INTO xp_sources VALUES (CAST(? AS TIMESTAMP), ?, ?)",
                params![ts(&a.start_utc), source, xp],
            )?;
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok((logs, by_start.len()))
}

fn import_fit_files(conn: &Connection, activities_dir: &Path) -> Result<(usize, usize)> {
    if !activities_dir.exists() {
        eprintln!("Zwift activities folder not found: {}", activities_dir.display());
        return Ok((0, 0));
    }
    let mut known: BTreeMap<String, i64> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT fit_file, file_mtime FROM fit_files")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (f, m) = row?;
            known.insert(f, m);
        }
    }

    let (mut added, mut failed) = (0, 0);
    conn.execute_batch("BEGIN TRANSACTION")?;
    for entry in fs::read_dir(activities_dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if !name.ends_with(".fit") || name == "inProgressActivity.fit" {
            continue;
        }
        let mtime = fs::metadata(&path)?
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if known.get(&name) == Some(&mtime) {
            continue;
        }
        match fit::read_summary(&path) {
            Ok(None) => {
                // Remember empty files too, so they aren't re-read on every import
                conn.execute(
                    "INSERT OR REPLACE INTO fit_files (fit_file, file_mtime) VALUES (?, ?)",
                    params![name, mtime],
                )?;
            }
            Ok(Some(s)) => {
                conn.execute(
                    "INSERT OR REPLACE INTO fit_files VALUES (?, CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        name, ts(&s.start_utc), s.elapsed_s, s.moving_s, s.distance_m, s.ascent_m,
                        s.calories, s.avg_power, s.max_power, s.normalized_power, s.avg_hr, s.max_hr,
                        s.avg_cadence, s.sport, mtime
                    ],
                )?;
                added += 1;
            }
            Err(e) => {
                eprintln!("  skipped {name}: {e}");
                failed += 1;
            }
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok((added, failed))
}

/// Activities whose save line was missing from the log get the FIT file that started within 3 minutes.
fn link_unmatched_activities(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        UPDATE activities a SET fit_file = m.fit_file
        FROM (
            SELECT a.start_utc, arg_min(f.fit_file, abs(epoch(f.start_utc) - epoch(a.start_utc))) AS fit_file
            FROM activities a
            JOIN fit_files f ON abs(epoch(f.start_utc) - epoch(a.start_utc)) <= 180
            WHERE a.fit_file IS NULL OR a.fit_file NOT IN (SELECT fit_file FROM fit_files)
            GROUP BY a.start_utc
        ) m
        WHERE a.start_utc = m.start_utc;
        "#,
    )?;
    Ok(())
}

fn import_legacy(conn: &Connection, legacy: &Path) -> Result<usize> {
    let sqlite = rusqlite::Connection::open_with_flags(legacy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_name = sqlite
        .prepare("SELECT name FROM pragma_table_info('rides') WHERE name = 'ride_name'")?
        .exists([])?;
    let sql = format!(
        "SELECT fit_file, xp, drops, duration_seconds, strava_activity_id, {} FROM rides",
        if has_name { "ride_name" } else { "NULL" }
    );
    let mut stmt = sqlite.prepare(&sql)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<f64>>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut n = 0;
    conn.execute_batch("BEGIN TRANSACTION")?;
    for row in rows {
        let (fit_file, xp, drops, dur, strava, name) = row?;
        conn.execute(
            "INSERT OR REPLACE INTO legacy_rides VALUES (?, ?, ?, ?, ?, ?)",
            params![fit_file, xp, drops, dur, strava, name],
        )?;
        n += 1;
    }
    conn.execute_batch("COMMIT")?;
    Ok(n)
}
