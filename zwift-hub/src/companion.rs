//! What the Zwift Companion app shows, fetched from Zwift's unofficial API: your profile, your
//! Zwift activities with their Ride Ons, who you follow (and who of them is riding now), and
//! upcoming events. Zwift does not document or support this API, so any of it can break.
//!
//! Signing in exchanges your email and password for a refresh token, and only that token is
//! saved (readable by you alone). The password is never written anywhere.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use duckdb::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const API: &str = "https://us-or-rly101.zwift.com";
const TOKEN_URLS: [&str; 2] = [
    "https://secure.zwift.com/auth/realms/zwift/protocol/openid-connect/token",
    // Older Keycloak path that earlier Zwift clients used
    "https://secure.zwift.com/auth/realms/zwift/tokens/access/codes",
];
const CLIENT_ID: &str = "Zwift_Mobile_Link";
const USER_AGENT: &str = "Zwift/115 CFNetwork/758.0.2 Darwin/15.0.0";
const MAX_ACTIVITIES: usize = 2000;

#[derive(Serialize, Deserialize)]
struct SavedToken {
    refresh_token: String,
    saved_at: String,
    /// The last access token and when it expires (Unix seconds), so frequent snapshots don't
    /// need a token refresh each time
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_expires: Option<i64>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub activities: usize,
    pub followers: usize,
    pub followees: usize,
    pub events: usize,
    /// Lists that couldn't be fetched this time; the rest was still saved
    pub problems: Vec<String>,
}

fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("zwift-token.json")
}

pub fn signed_in(data_dir: &Path) -> bool {
    token_path(data_dir).exists()
}

/// The app and the background snapshot both refresh the token, and Zwift rotates the refresh
/// token each time; holding this lock around read, refresh and save keeps the newest one.
fn lock_token(data_dir: &Path) -> Result<fs::File> {
    fs::create_dir_all(data_dir)?;
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("zwift-token.lock"))?;
    lock.lock()?;
    Ok(lock)
}

/// Written to a temporary file and renamed into place, so a reader never sees half a token.
fn write_token(data_dir: &Path, token: &SavedToken) -> Result<()> {
    let path = token_path(data_dir);
    let tmp = path.with_extension("json.tmp");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(&tmp)?.write_all(&serde_json::to_vec_pretty(token)?)?;
    #[cfg(unix)]
    {
        // `mode` applies only when the file is created; tighten a leftover file too
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn save_token(data_dir: &Path, refresh_token: &str) -> Result<()> {
    let _lock = lock_token(data_dir)?;
    write_token(
        data_dir,
        &SavedToken {
            refresh_token: refresh_token.to_string(),
            saved_at: chrono::Utc::now().to_rfc3339(),
            access_token: None,
            access_expires: None,
        },
    )
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder().user_agent(USER_AGENT).build()?)
}

async fn request_token(client: &reqwest::Client, form: &[(&str, &str)]) -> Result<TokenResponse> {
    let mut last_err = None;
    for url in TOKEN_URLS {
        let resp = client.post(url).form(form).send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            last_err = Some(anyhow::anyhow!("{url} not found"));
            continue;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::BAD_REQUEST {
            let body = resp.text().await.unwrap_or_default();
            bail!("Zwift rejected the sign-in ({status}): {}", body.chars().take(200).collect::<String>());
        }
        return Ok(resp.error_for_status()?.json().await?);
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no token endpoint answered")))
}

/// Exchanges email and password for a refresh token, saves only the token, and returns the
/// signed-in rider's name.
pub async fn sign_in(data_dir: &Path, email: &str, password: &str) -> Result<String> {
    let client = client()?;
    let token = request_token(
        &client,
        &[("client_id", CLIENT_ID), ("grant_type", "password"), ("username", email.trim()), ("password", password)],
    )
    .await?;
    let refresh = token.refresh_token.context("Zwift returned no refresh token")?;
    save_token(data_dir, &refresh)?;
    let me = get(&client, &token.access_token, "/api/profiles/me").await?;
    Ok(format!(
        "{} {}",
        str_at(&me, "firstName").unwrap_or_default(),
        str_at(&me, "lastName").unwrap_or_default()
    )
    .trim()
    .to_string())
}

/// Prompts for email and password on the terminal; the explorer's Companion tab does the same.
pub async fn login(data_dir: &Path) -> Result<()> {
    print!("Zwift email: ");
    std::io::stdout().flush()?;
    let mut email = String::new();
    std::io::stdin().read_line(&mut email)?;
    let password = rpassword::prompt_password("Zwift password (not saved): ")?;
    let name = sign_in(data_dir, &email, &password).await?;
    println!(
        "Signed in as {name}. Saved a refresh token to {} (your password was not saved).",
        token_path(data_dir).display()
    );
    Ok(())
}

/// Removes the saved sign-in; returns whether there was one.
pub fn sign_out(data_dir: &Path) -> Result<bool> {
    let _lock = lock_token(data_dir)?;
    let path = token_path(data_dir);
    let existed = path.exists();
    if existed {
        fs::remove_file(&path)?;
    }
    // The totals belong to that sign-in too
    let _ = fs::remove_file(data_dir.join(SNAPSHOTS));
    Ok(existed)
}

pub fn logout(data_dir: &Path) -> Result<()> {
    if sign_out(data_dir)? {
        println!("Signed out; removed {}", token_path(data_dir).display());
    } else {
        println!("Not signed in");
    }
    Ok(())
}

async fn access_token(client: &reqwest::Client, data_dir: &Path) -> Result<String> {
    let path = token_path(data_dir);
    if !path.exists() {
        bail!("not signed in to Zwift; run `zwift-hub login`");
    }
    let _lock = lock_token(data_dir)?;
    let saved: SavedToken = serde_json::from_slice(&fs::read(&path)?)?;
    let now = chrono::Utc::now().timestamp();
    if let (Some(access), Some(expires)) = (&saved.access_token, saved.access_expires) {
        if expires > now + 60 {
            return Ok(access.clone());
        }
    }
    let token = request_token(
        client,
        &[("client_id", CLIENT_ID), ("grant_type", "refresh_token"), ("refresh_token", &saved.refresh_token)],
    )
    .await
    .context("refreshing the Zwift sign-in failed; run `zwift-hub login` again")?;
    // Zwift rotates refresh tokens; keep the newest so the sign-in doesn't lapse
    let rotated = token.refresh_token.clone().filter(|r| *r != saved.refresh_token);
    write_token(
        data_dir,
        &SavedToken {
            saved_at: if rotated.is_some() { chrono::Utc::now().to_rfc3339() } else { saved.saved_at },
            refresh_token: rotated.unwrap_or(saved.refresh_token),
            access_token: Some(token.access_token.clone()),
            access_expires: token.expires_in.map(|s| now + s),
        },
    )?;
    Ok(token.access_token)
}

async fn get(client: &reqwest::Client, access: &str, path: &str) -> Result<Value> {
    let resp = client
        .get(format!("{API}{path}"))
        .bearer_auth(access)
        // Without this the API answers in protobuf
        .header("Accept", "application/json")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("GET {path}: {status} {}", body.chars().take(300).collect::<String>().trim());
    }
    Ok(resp.json().await?)
}

const PAGE: usize = 20;

/// Your activities from every device. Zwift has served this list from two places over the
/// years; try the profile list first, then the Companion app's own feed.
async fn fetch_activities(client: &reqwest::Client, access: &str, id: i64, max: usize) -> Result<Vec<Value>> {
    let as_list = |v: Value| v.as_array().cloned().unwrap_or_default();
    let mut activities = Vec::new();
    let first = get(client, access, &format!("/api/profiles/{id}/activities?start=0&limit={PAGE}")).await;
    match first {
        Ok(page) => {
            let mut page = as_list(page);
            loop {
                let n = page.len();
                activities.extend(page);
                if n < PAGE || activities.len() >= max {
                    return Ok(activities);
                }
                page = as_list(
                    get(client, access, &format!("/api/profiles/{id}/activities?start={}&limit={PAGE}", activities.len()))
                        .await?,
                );
            }
        }
        Err(profile_err) => {
            let feed = "/api/activity-feed/feed/?includeInProgress=false&feedType=JUST_ME";
            let mut path = format!("{feed}&limit={PAGE}");
            loop {
                let page = match get(client, access, &path).await {
                    Ok(v) => as_list(v),
                    Err(e) if activities.is_empty() => bail!("{profile_err}; feed: {e}"),
                    Err(e) => return Err(e),
                };
                let n = page.len();
                let last = page.last().and_then(|a| str_at(a, "id_str").or_else(|| str_at(a, "id")));
                activities.extend(page);
                match last {
                    Some(last) if n == PAGE && activities.len() < max => {
                        path = format!("{feed}&limit={PAGE}&start_after_activity_id={last}");
                    }
                    _ => return Ok(activities),
                }
            }
        }
    }
}

/// Everything fetched in one sync, gathered before the database is touched so the write is quick.
/// A list that failed to fetch is `None` and leaves what's stored untouched.
pub struct Fetched {
    me: Value,
    activities: Option<Vec<Value>>,
    followers: Option<Vec<Value>>,
    followees: Option<Vec<Value>>,
    events: Option<Vec<Value>>,
    problems: Vec<String>,
}

/// `full` pages through every activity; otherwise only the newest page, which is all that can
/// have changed since the last sync.
pub async fn fetch(data_dir: &Path, full: bool) -> Result<Fetched> {
    let client = client()?;
    let access = access_token(&client, data_dir).await?;
    let me = get(&client, &access, "/api/profiles/me").await?;
    let id = me.get("id").and_then(Value::as_i64).context("profile has no id")?;

    let mut problems = Vec::new();
    let mut keep = |what: &str, r: Result<Vec<Value>>| match r {
        Ok(v) => Some(v),
        Err(e) => {
            problems.push(format!("{what}: {e:#}"));
            None
        }
    };
    let list = |r: Result<Value>| r.map(|v| v.as_array().cloned().unwrap_or_default());
    let activities = keep("activities", fetch_activities(&client, &access, id, if full { MAX_ACTIVITIES } else { PAGE }).await);
    let followers = keep("followers", list(get(&client, &access, &format!("/api/profiles/{id}/followers?start=0&limit=200")).await));
    let followees = keep("following", list(get(&client, &access, &format!("/api/profiles/{id}/followees?start=0&limit=200")).await));
    let events = keep("events", list(get(&client, &access, "/api/public/events/upcoming").await));
    Ok(Fetched { me, activities, followers, followees, events, problems })
}

/// Background snapshots of your totals, appended by `zwift-hub snapshot` from a LaunchAgent
/// while the app may be closed (so without the database, which the app keeps locked). A line is
/// written only when the totals or the riding state change; otherwise the file is just touched,
/// so its age says when Zwift was last asked.
const SNAPSHOTS: &str = "totals.jsonl";
/// How often to ask Zwift when you're not riding; while riding, every time the agent runs
const IDLE_CHECK: Duration = Duration::from_secs(10 * 60);

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct Snapshot {
    pub fetched_at: String,
    pub total_xp: Option<i64>,
    pub drops: Option<i64>,
    pub achievement_level: Option<i64>,
    pub riding: bool,
}

fn riding(me: &Value) -> bool {
    bool_at(me, "riding").unwrap_or(false) || me.get("currentActivityId").is_some_and(|v| !v.is_null())
}

fn last_snapshot(path: &Path) -> Option<Snapshot> {
    let mut f = fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(1024))).ok()?;
    let mut tail = String::new();
    f.read_to_string(&mut tail).ok()?;
    serde_json::from_str(tail.lines().last()?).ok()
}

/// One cheap check of your totals. Returns what was written, or `None` when it skipped (not
/// signed in, checked recently and not riding, or nothing changed).
pub async fn snapshot(data_dir: &Path) -> Result<Option<Snapshot>> {
    if !signed_in(data_dir) {
        return Ok(None);
    }
    let path = data_dir.join(SNAPSHOTS);
    let last = last_snapshot(&path);
    let age = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    if last.as_ref().is_some_and(|l| !l.riding) && age.is_some_and(|a| a < IDLE_CHECK) {
        return Ok(None);
    }
    let client = client()?;
    let access = access_token(&client, data_dir).await?;
    let me = get(&client, &access, "/api/profiles/me").await?;
    let snap = Snapshot {
        fetched_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        total_xp: int_at(&me, "totalExperiencePoints"),
        drops: int_at(&me, "totalGold"),
        achievement_level: int_at(&me, "achievementLevel"),
        riding: riding(&me),
    };
    let mut f = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let unchanged = last.as_ref().is_some_and(|l| {
        (l.total_xp, l.drops, l.achievement_level, l.riding) == (snap.total_xp, snap.drops, snap.achievement_level, snap.riding)
    });
    if unchanged {
        f.set_modified(SystemTime::now())?;
        return Ok(None);
    }
    writeln!(f, "{}", serde_json::to_string(&snap)?)?;
    Ok(Some(snap))
}

/// Adds snapshots newer than the last one stored to `zwift_profile_history`, which the rides
/// view uses to estimate what rides on other devices earned.
pub fn fold_snapshots(conn: &Connection, data_dir: &Path) -> Result<usize> {
    let Ok(text) = fs::read_to_string(data_dir.join(SNAPSHOTS)) else { return Ok(0) };
    let mut n = 0;
    for snap in text.lines().filter_map(|l| serde_json::from_str::<Snapshot>(l).ok()) {
        n += conn.execute(
            "INSERT OR IGNORE INTO zwift_profile_history VALUES (CAST(? AS TIMESTAMP), ?, ?, ?)",
            params![snap.fetched_at.trim_end_matches('Z').replace('T', " "), snap.total_xp, snap.drops, snap.achievement_level],
        )?;
    }
    Ok(n)
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| match x {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

fn num_at(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64().or_else(|| x.as_str()?.parse().ok()))
}

fn int_at(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64().or_else(|| x.as_str()?.parse().ok()))
}

fn bool_at(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

/// Zwift dates look like `2026-10-15T13:58:38.000+0000`; DuckDB wants `2026-10-15 13:58:38`.
fn ts_at(v: &Value, key: &str) -> Option<String> {
    let s = str_at(v, key)?;
    chrono::DateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f%z")
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(&s))
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn store(conn: &Connection, f: &Fetched) -> Result<SyncReport> {
    conn.execute_batch("BEGIN TRANSACTION")?;
    let me = &f.me;
    let social = me.get("socialFacts").cloned().unwrap_or(Value::Null);
    conn.execute(
        "INSERT OR REPLACE INTO zwift_profile_history VALUES (CAST(? AS TIMESTAMP), ?, ?, ?)",
        params![
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            int_at(me, "totalExperiencePoints"), int_at(me, "totalGold"), int_at(me, "achievementLevel")
        ],
    )?;
    conn.execute("DELETE FROM zwift_profile", [])?;
    conn.execute(
        "INSERT INTO zwift_profile VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, current_timestamp, ?)",
        params![
            int_at(me, "id"), str_at(me, "firstName"), str_at(me, "lastName"), str_at(me, "countryAlpha3"),
            num_at(me, "ftp"), num_at(me, "weight").map(|g| g / 1000.0), num_at(me, "height").map(|mm| mm / 10.0),
            int_at(me, "achievementLevel"), int_at(me, "totalExperiencePoints"), int_at(me, "totalGold"),
            num_at(me, "totalDistance").map(|m| m / 1000.0), num_at(me, "totalDistanceClimbed"),
            num_at(me, "totalTimeInMinutes").map(|m| m / 60.0),
            int_at(&social, "followersCount"), int_at(&social, "followeesCount"), me.to_string()
        ],
    )?;

    for a in f.activities.iter().flatten() {
        let Some(id) = str_at(a, "id_str").or_else(|| str_at(a, "id")) else { continue };
        conn.execute(
            "INSERT OR REPLACE INTO zwift_activities VALUES (?, ?, CAST(? AS TIMESTAMP), CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id, str_at(a, "name"), ts_at(a, "startDate"), ts_at(a, "endDate"), num_at(a, "distanceInMeters"),
                num_at(a, "totalElevation"), num_at(a, "avgWatts"), num_at(a, "calories"),
                num_at(a, "movingTimeInMs").map(|ms| ms / 1000.0), str_at(a, "sport"), int_at(a, "worldId"),
                int_at(a, "activityRideOnCount"), int_at(a, "activityCommentCount"), str_at(a, "privacy"),
                a.to_string()
            ],
        )?;
    }

    for (direction, rows, profile_key, id_key) in [
        ("follower", &f.followers, "followerProfile", "followerId"),
        ("followee", &f.followees, "followeeProfile", "followeeId"),
    ] {
        let Some(rows) = rows else { continue };
        conn.execute("DELETE FROM zwift_follows WHERE direction = ?", params![direction])?;
        for row in rows {
            let p = row.get(profile_key).cloned().unwrap_or(Value::Null);
            let Some(profile_id) = int_at(row, id_key).or_else(|| int_at(&p, "id")) else { continue };
            conn.execute(
                "INSERT OR REPLACE INTO zwift_follows VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    direction, profile_id, str_at(&p, "firstName"),
                    str_at(&p, "lastName"), str_at(&p, "countryAlpha3"), bool_at(&p, "riding"),
                    int_at(&p, "worldId"), str_at(row, "status"), row.to_string()
                ],
            )?;
        }
    }

    // DuckDB's bundled build has no time-zone functions, so the cutoff is computed here
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(1)).format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute("DELETE FROM zwift_events WHERE start_utc < CAST(? AS TIMESTAMP)", params![cutoff])?;
    for e in f.events.iter().flatten() {
        if int_at(e, "id").is_none() {
            continue;
        }
        conn.execute(
            "INSERT OR REPLACE INTO zwift_events VALUES (?, ?, CAST(? AS TIMESTAMP), ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                int_at(e, "id"), str_at(e, "name"), ts_at(e, "eventStart"), str_at(e, "sport"),
                str_at(e, "eventType"), int_at(e, "mapId"), int_at(e, "routeId"), num_at(e, "distanceInMeters"),
                num_at(e, "durationInSeconds"), int_at(e, "laps"), e.to_string()
            ],
        )?;
    }
    conn.execute_batch("COMMIT")?;
    let n = |v: &Option<Vec<Value>>| v.as_ref().map_or(0, Vec::len);
    Ok(SyncReport {
        activities: n(&f.activities),
        followers: n(&f.followers),
        followees: n(&f.followees),
        events: n(&f.events),
        problems: f.problems.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_zwift_dates() {
        let v = json!({ "a": "2026-10-15T13:58:38.000+0000", "b": "2026-10-15T14:00:00Z" });
        assert_eq!(ts_at(&v, "a").as_deref(), Some("2026-10-15 13:58:38"));
        assert_eq!(ts_at(&v, "b").as_deref(), Some("2026-10-15 14:00:00"));
    }

    #[test]
    fn stores_a_sync() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        let f = Fetched {
            me: json!({ "id": 42, "firstName": "James", "weight": 75000, "totalGold": 123456,
                        "socialFacts": { "followersCount": 10, "followeesCount": 12 } }),
            problems: vec![],
            activities: Some(vec![json!({ "id_str": "999", "name": "Zwift - Watopia", "startDate": "2026-10-15T13:58:38.000+0000",
                                     "distanceInMeters": 30000.5, "activityRideOnCount": 7 })]),
            followers: Some(vec![json!({ "followerId": 7, "followerProfile": { "firstName": "A", "riding": true, "worldId": 1 } })]),
            followees: None,
            events: Some(vec![json!({ "id": 5, "name": "Group Ride", "eventStart": "2099-01-01T10:00:00.000+0000", "sport": "CYCLING" })]),
        };
        let r = store(&conn, &f).unwrap();
        assert_eq!((r.activities, r.followers, r.events), (1, 1, 1));
        let (w, drops): (f64, i64) =
            conn.query_row("SELECT weight_kg, drops FROM zwift_profile", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((w, drops), (75.0, 123456));
        let ride_ons: i64 = conn.query_row("SELECT ride_ons FROM zwift_activities", [], |r| r.get(0)).unwrap();
        assert_eq!(ride_ons, 7);
    }

    #[test]
    fn folds_snapshots_once() {
        let dir = std::env::temp_dir().join(format!("zwift-hub-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let line = |t: &str, xp: i64| {
            serde_json::to_string(&Snapshot {
                fetched_at: t.into(), total_xp: Some(xp), drops: Some(5), achievement_level: None, riding: false,
            })
            .unwrap()
        };
        fs::write(dir.join(SNAPSHOTS), format!("{}\n{}\n", line("2026-10-21T20:00:00Z", 1), line("2026-10-21T21:00:00Z", 2))).unwrap();
        assert_eq!(last_snapshot(&dir.join(SNAPSHOTS)).unwrap().total_xp, Some(2));
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        assert_eq!(fold_snapshots(&conn, &dir).unwrap(), 2);
        assert_eq!(fold_snapshots(&conn, &dir).unwrap(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn api_only_rides_join_the_ride_list_with_estimated_earnings() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::SCHEMA).unwrap();
        conn.execute_batch(crate::db::RIDES_VIEW).unwrap();
        conn.execute_batch(
            "INSERT INTO zwift_profile_history VALUES
                ('2026-10-20 06:00:00', 1000, 500, NULL),
                ('2026-10-20 09:00:00', 1600, 800, NULL),
                ('2026-10-22 09:00:00', 2600, 900, NULL);
             INSERT INTO zwift_activities (id, name, start_utc, end_utc, distance_m, sport) VALUES
                ('1', 'Zwift - Group Ride: Tempus Fugit in Watopia', '2026-10-20 07:00:00', '2026-10-20 08:00:00', 40000, 'CYCLING'),
                ('2', 'Zwift - Volcano Flat in Watopia', '2026-10-21 07:00:00', '2026-10-21 08:00:00', 30000, 'CYCLING'),
                ('3', 'Zwift - Makuri 40 in Makuri Islands', '2026-10-21 18:00:00', '2026-10-21 19:00:00', 30000, 'CYCLING');",
        )
        .unwrap();
        let rows: Vec<(String, Option<String>, Option<i64>, Option<i64>, String)> = conn
            .prepare("SELECT title, world, xp, drops, kind FROM rides WHERE earnings_source = 'api' ORDER BY start_utc")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows.len(), 3);
        // Alone between two syncs: the change in totals is what it earned
        assert_eq!((rows[0].2, rows[0].3), (Some(600), Some(300)));
        assert_eq!(rows[0].1.as_deref(), Some("Watopia"));
        assert_eq!(rows[0].4, "Group ride");
        // Two rides between the same syncs can't be told apart
        assert_eq!((rows[1].2, rows[2].2), (None, None));
    }

}
