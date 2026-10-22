//! Adds each ride's XP and Drops to its Strava activity description, the way the earlier
//! zwift-stats tool did. It shares that tool's Strava app settings and sign-in
//! (~/.config/zwift-stats), its marker line, and its block format, so whichever of the two gets to
//! a ride first writes the block and the other leaves it alone.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use duckdb::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MARKER: &str = "--- Zwift Stats ---";
const API: &str = "https://www.strava.com/api/v3";
const TOKEN_URL: &str = "https://www.strava.com/oauth/token";
/// Must match the Authorization Callback Domain (localhost) of the Strava API app
const CALLBACK_PORT: u16 = 8888;
/// New rides are posted automatically within this many days; older ones only from the ride page
pub const AUTO_DAYS: i64 = 3;
/// Strava can take a few minutes to receive Zwift's upload; give up on a ride after this many tries
const MAX_ATTEMPTS: i64 = 12;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}

fn config_dir() -> Result<PathBuf> {
    Ok(dirs::home_dir().context("no home folder")?.join(".config/zwift-stats"))
}

fn read_json<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    serde_json::from_slice(&fs::read(config_dir().ok()?.join(name)).ok()?).ok()
}

/// Writes next to the target and renames, so a crash or the other tool never sees half a file.
fn write_json<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".{name}.tmp"));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&tmp)?.write_all(&serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, dir.join(name))?;
    Ok(())
}

pub fn connected() -> bool {
    read_json::<AppConfig>("config.json").is_some() && read_json::<Tokens>("tokens.json").is_some()
}

pub fn disconnect() -> Result<()> {
    if let Ok(path) = config_dir().map(|d| d.join("tokens.json")) {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?)
}

async fn token_request(client: &reqwest::Client, body: Value) -> Result<Tokens> {
    let resp = client.post(TOKEN_URL).json(&body).send().await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Strava sign-in failed ({status}): {}", v.get("message").and_then(Value::as_str).unwrap_or(""));
    }
    let tokens = Tokens {
        access_token: v["access_token"].as_str().context("no access token")?.to_string(),
        refresh_token: v["refresh_token"].as_str().context("no refresh token")?.to_string(),
        expires_at: v["expires_at"].as_i64().context("no expiry")?,
    };
    write_json("tokens.json", &tokens)?;
    Ok(tokens)
}

async fn access_token(client: &reqwest::Client) -> Result<String> {
    let config: AppConfig = read_json("config.json").context("Strava isn't connected")?;
    let tokens: Tokens = read_json("tokens.json").context("Strava isn't connected")?;
    if chrono::Utc::now().timestamp() < tokens.expires_at - 300 {
        return Ok(tokens.access_token);
    }
    let fresh = token_request(
        client,
        serde_json::json!({
            "client_id": config.client_id, "client_secret": config.client_secret,
            "grant_type": "refresh_token", "refresh_token": tokens.refresh_token,
        }),
    )
    .await?;
    Ok(fresh.access_token)
}

/// Saves the Strava API app's details and returns the page to approve access on. The approval
/// comes back to a one-off listener on localhost:8888, closed after one request or 5 minutes.
pub async fn begin_connect(config: AppConfig) -> Result<String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .with_context(|| format!("port {CALLBACK_PORT} is in use; quit whatever uses it and try again"))?;
    write_json("config.json", &config)?;
    let redirect = format!("http://localhost:{CALLBACK_PORT}/callback");
    let url = format!(
        "https://www.strava.com/oauth/authorize?client_id={}&redirect_uri={}&response_type=code&scope=activity:read_all,activity:write&approval_prompt=auto",
        config.client_id,
        urlencode(&redirect)
    );
    tokio::spawn(async move {
        match tokio::time::timeout(Duration::from_secs(300), receive_code(listener)).await {
            Ok(Ok(code)) => {
                let result = token_request(
                    &match client() {
                        Ok(c) => c,
                        Err(e) => return eprintln!("Strava: {e:#}"),
                    },
                    serde_json::json!({
                        "client_id": config.client_id, "client_secret": config.client_secret,
                        "code": code, "grant_type": "authorization_code",
                    }),
                )
                .await;
                if let Err(e) = result {
                    eprintln!("Strava: {e:#}");
                }
            }
            Ok(Err(e)) => eprintln!("Strava: {e:#}"),
            Err(_) => eprintln!("Strava: gave up waiting for approval"),
        }
    });
    Ok(url)
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn receive_code(listener: tokio::net::TcpListener) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut sock, _) = listener.accept().await?;
    let mut buf = vec![0u8; 8192];
    let n = sock.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let target = request.split_whitespace().nth(1).unwrap_or_default();
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
    let param = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(str::to_string);
    let (title, result) = match (param("code"), param("error")) {
        (Some(code), _) => ("Strava is connected. You can close this tab and go back to Zwift Stats.", Ok(code)),
        (_, Some(e)) => ("Strava access wasn't granted. You can close this tab.", Err(anyhow::anyhow!("denied: {e}"))),
        _ => ("Something went wrong. Try connecting again from Zwift Stats.", Err(anyhow::anyhow!("no code"))),
    };
    let body = format!("<!doctype html><meta charset=utf-8><body style=\"font:16px -apple-system,sans-serif;padding:40px\"><p>{title}</p>");
    let _ = sock
        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
        .await;
    result
}

/// A ride waiting to be posted, gathered from the database before any network call.
pub struct Pending {
    pub start_utc: String,
    pub xp: i64,
    pub drops: i64,
    pub duration_s: f64,
    pub estimated: bool,
    pub best_xp_per_hour: bool,
    pub best_drops_per_hour: bool,
}

/// Rides to post: exact earnings from the last few days not yet done (or, with `only`, that one
/// ride whatever its age or source).
pub fn pending(conn: &Connection, only: Option<&str>) -> Result<Vec<Pending>> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(AUTO_DAYS)).format("%Y-%m-%d %H:%M:%S").to_string();
    let filter = match only {
        Some(_) => "r.start_utc = CAST(?1 AS TIMESTAMP)",
        None => "r.earnings_source = 'log' AND r.start_utc >= CAST(?1 AS TIMESTAMP)
                 AND (p.start_utc IS NULL OR (p.status = 'not_found' AND p.attempts < ?2))",
    };
    let sql = format!(
        "SELECT strftime(r.start_utc, '%Y-%m-%d %H:%M:%S'), r.xp, r.drops, r.duration_s, r.earnings_source = 'api',
            r.xp_per_hour > coalesce((SELECT max(o.xp_per_hour) FROM rides o
                WHERE o.start_utc < r.start_utc AND o.duration_s >= 600 AND o.earnings_source <> 'api'), 0),
            r.drops_per_hour > coalesce((SELECT max(o.drops_per_hour) FROM rides o
                WHERE o.start_utc < r.start_utc AND o.duration_s >= 600 AND o.earnings_source <> 'api'), 0)
         FROM rides r LEFT JOIN strava_posts p ON p.start_utc = r.start_utc
         WHERE r.xp IS NOT NULL AND r.drops IS NOT NULL AND r.duration_s >= 60 AND {filter}
         ORDER BY r.start_utc"
    );
    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &duckdb::Row| {
        Ok(Pending {
            start_utc: r.get(0)?,
            xp: r.get(1)?,
            drops: r.get(2)?,
            duration_s: r.get(3)?,
            estimated: r.get(4)?,
            best_xp_per_hour: r.get::<_, Option<bool>>(5)?.unwrap_or(false),
            best_drops_per_hour: r.get::<_, Option<bool>>(6)?.unwrap_or(false),
        })
    };
    let rows = match only {
        Some(start) => stmt.query_map(params![start], map)?.collect::<Result<Vec<_>, _>>()?,
        None => stmt.query_map(params![cutoff, MAX_ATTEMPTS], map)?.collect::<Result<Vec<_>, _>>()?,
    };
    Ok(rows)
}

fn num(n: f64) -> String {
    let s = format!("{:.0}", n.round().abs());
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0.0 { format!("-{out}") } else { out }
}

/// The same block the earlier tool wrote, so the two read alike on Strava.
pub fn block(p: &Pending) -> String {
    let hours = p.duration_s / 3600.0;
    let trophy = |b: bool| if b { "🏆 " } else { "" };
    let approx = if p.estimated { "≈" } else { "" };
    let mut lines = vec![
        MARKER.to_string(),
        format!("XP: {approx}{}  ({}{}/hr)", num(p.xp as f64), trophy(p.best_xp_per_hour), num(p.xp as f64 / hours)),
        format!("Drops: {approx}{}  ({}{}/hr)", num(p.drops as f64), trophy(p.best_drops_per_hour), num(p.drops as f64 / hours)),
    ];
    if p.estimated {
        lines.push("(estimated from the change in Zwift totals)".to_string());
    }
    lines.join("\n")
}

pub enum Outcome {
    Posted(i64),
    AlreadyThere(i64),
    NotFound,
}

/// Finds the Strava activity Zwift uploaded for this ride and appends the block to its description.
pub async fn post(p: &Pending) -> Result<Outcome> {
    let client = client()?;
    let token = access_token(&client).await?;
    let start = chrono::NaiveDateTime::parse_from_str(&p.start_utc, "%Y-%m-%d %H:%M:%S")?.and_utc().timestamp();
    let list: Vec<Value> = client
        .get(format!("{API}/athlete/activities?after={}&before={}&per_page=10", start - 300, start + 300))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let rides: Vec<i64> = list
        .iter()
        .filter(|a| a["sport_type"].as_str() == Some("VirtualRide") || a["type"].as_str() == Some("VirtualRide"))
        .filter_map(|a| a["id"].as_i64())
        .collect();
    let mut chosen = None;
    for id in &rides {
        let full: Value = client.get(format!("{API}/activities/{id}")).bearer_auth(&token).send().await?.error_for_status()?.json().await?;
        let is_zwift = full["external_id"].as_str().is_some_and(|e| e.starts_with("zwift-activity-"));
        if is_zwift || rides.len() == 1 {
            chosen = Some((*id, full));
            break;
        }
    }
    let Some((id, full)) = chosen else { return Ok(Outcome::NotFound) };
    let existing = full["description"].as_str().unwrap_or("").trim().to_string();
    if existing.contains(MARKER) {
        return Ok(Outcome::AlreadyThere(id));
    }
    let description = if existing.is_empty() { block(p) } else { format!("{existing}\n\n{}", block(p)) };
    client
        .put(format!("{API}/activities/{id}"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "description": description }))
        .send()
        .await?
        .error_for_status()?;
    Ok(Outcome::Posted(id))
}

pub fn record(conn: &Connection, start_utc: &str, outcome: &Result<Outcome>) -> Result<()> {
    let (status, id, error) = match outcome {
        Ok(Outcome::Posted(id)) => ("posted", Some(*id), None),
        Ok(Outcome::AlreadyThere(id)) => ("already_there", Some(*id), None),
        Ok(Outcome::NotFound) => ("not_found", None, None),
        Err(e) => ("error", None, Some(format!("{e:#}"))),
    };
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO strava_posts VALUES (CAST(?1 AS TIMESTAMP), ?2, ?3, CAST(?4 AS TIMESTAMP), 1, ?5)
         ON CONFLICT (start_utc) DO UPDATE SET status = excluded.status, strava_id = excluded.strava_id,
            updated_at = excluded.updated_at, attempts = strava_posts.attempts + 1, error = excluded.error",
        params![start_utc, status, id, now, error],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_matches_the_earlier_tool() {
        let p = Pending {
            start_utc: String::new(), xp: 1234, drops: 45678, duration_s: 3600.0 * 1.5,
            estimated: false, best_xp_per_hour: true, best_drops_per_hour: false,
        };
        assert_eq!(block(&p), "--- Zwift Stats ---\nXP: 1,234  (🏆 823/hr)\nDrops: 45,678  (30,452/hr)");
    }

    #[test]
    fn pending_picks_recent_unposted_log_rides() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        conn.execute_batch(crate::db::RIDES_VIEW).unwrap();
        let recent = (chrono::Utc::now() - chrono::Duration::hours(2)).format("%Y-%m-%d %H:%M:%S").to_string();
        let old = (chrono::Utc::now() - chrono::Duration::days(10)).format("%Y-%m-%d %H:%M:%S").to_string();
        for (start, xp) in [(&old, 500), (&recent, 900)] {
            conn.execute(
                "INSERT INTO activities (start_utc, end_utc, xp, drops, distance_m) VALUES
                 (CAST(?1 AS TIMESTAMP), CAST(?1 AS TIMESTAMP) + INTERVAL 1 HOUR, ?2, 1000, 30000)",
                params![start, xp],
            )
            .unwrap();
        }
        let p = pending(&conn, None).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].xp, 900);
        assert!(p[0].best_xp_per_hour);
        record(&conn, &p[0].start_utc, &Ok(Outcome::Posted(42))).unwrap();
        assert!(pending(&conn, None).unwrap().is_empty());
        // The ride page can still post an old ride
        assert_eq!(pending(&conn, Some(&old)).unwrap().len(), 1);
    }
}
