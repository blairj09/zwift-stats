use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use duckdb::Connection;
use rust_embed::Embed;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{companion, db};
use crate::import::{self, Paths};
use crate::watch;

#[derive(Embed)]
#[folder = "assets/"]
struct Assets;

struct AppState {
    conn: Mutex<Connection>,
    paths: Paths,
    /// Bumped after every import, so open explorer pages know to reload
    imports: AtomicU64,
    last_import: Mutex<Option<String>>,
}

impl AppState {
    fn import(&self) -> Result<import::ImportReport> {
        let conn = self.conn.lock().unwrap();
        let report = import::run(&conn, &self.paths)?;
        self.imports.fetch_add(1, Ordering::Relaxed);
        *self.last_import.lock().unwrap() = Some(chrono::Utc::now().to_rfc3339());
        Ok(report)
    }

    async fn sync(&self) -> Result<companion::SyncReport> {
        let fetched = companion::fetch(&self.paths.data_dir).await?;
        let conn = self.conn.lock().unwrap();
        let report = companion::store(&conn, &fetched)?;
        self.imports.fetch_add(1, Ordering::Relaxed);
        Ok(report)
    }
}

type Shared = Arc<AppState>;

const SQL_ROW_LIMIT: usize = 5000;

/// Keeps the folder watcher alive for as long as the app runs.
pub struct Running {
    pub router: Router,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    state: Shared,
    _watcher: Option<notify::RecommendedWatcher>,
}

impl Running {
    /// Imports on a worker thread; open pages reload when it finishes.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn import_in_background(&self) {
        let st = self.state.clone();
        tokio::task::spawn_blocking(move || match st.import() {
            Ok(r) => crate::print_report(&r),
            Err(e) => eprintln!("import failed: {e:#}"),
        });
    }
}

/// Imports, starts the watcher and the Companion refresh, and returns the routes. Shared by the
/// web server and the Mac app, which answers the same routes without opening a port. Must be
/// called inside a Tokio runtime.
pub fn start(conn: Connection, paths: Paths, initial_import: bool, watch: bool) -> Result<Running> {
    let state = Arc::new(AppState {
        conn: Mutex::new(conn),
        paths,
        imports: AtomicU64::new(0),
        last_import: Mutex::new(None),
    });
    if initial_import {
        crate::print_report(&state.import()?);
    }
    let watcher = if watch {
        let st = state.clone();
        Some(watch::spawn(&state.paths, move || match st.import() {
            Ok(r) => crate::print_report(&r),
            Err(e) => eprintln!("import failed: {e:#}"),
        })?)
    } else {
        None
    };
    let router = Router::new()
        .route("/", get(|| asset("index.html")))
        .route("/assets/{*file}", get(|Path(file): Path<String>| asset_owned(file)))
        .route("/api/rides", get(rides))
        .route("/api/xp_sources", get(xp_sources))
        .route("/api/status", get(status))
        .route("/api/import", post(run_import))
        .route("/api/sql", post(sql))
        .route("/api/companion", get(companion_data))
        .route("/api/sync", post(run_sync))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .with_state(state.clone());

    // Companion data goes stale as friends start riding and events fill; refresh it while running
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
        loop {
            tick.tick().await;
            if companion::signed_in(&st.paths.data_dir) {
                match st.sync().await {
                    Ok(r) => crate::print_sync(&r),
                    Err(e) => eprintln!("Zwift sync failed: {e:#}"),
                }
            }
        }
    });
    Ok(Running { router, state, _watcher: watcher })
}

pub async fn serve(conn: Connection, paths: Paths, port: u16, initial_import: bool, watch: bool) -> Result<()> {
    let running = start(conn, paths, initial_import, watch)?;
    let app = running.router.clone().layer(middleware::from_fn(local_only));

    // Loopback only: the database is personal and the SQL endpoint is unauthenticated
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("Zwift Stats running at http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn is_local(value: &str) -> bool {
    let host = value.split("://").last().unwrap_or_default();
    let host = host.split(['/', ':']).next().unwrap_or_default();
    matches!(host, "127.0.0.1" | "localhost")
}

/// The server has no authentication, so it only answers pages it served itself: a web page
/// elsewhere can't post to it, and a site that points its own domain at 127.0.0.1 (DNS
/// rebinding) still sends its own name in Host and is refused.
async fn local_only(headers: HeaderMap, req: Request, next: Next) -> Response {
    let header_ok = |name: header::HeaderName, required: bool| match headers.get(name).and_then(|v| v.to_str().ok()) {
        Some(v) => is_local(v),
        None => !required,
    };
    if header_ok(header::HOST, true) && header_ok(header::ORIGIN, false) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "Zwift Stats only answers pages it served").into_response()
    }
}

async fn asset(path: &'static str) -> Response {
    asset_owned(path.to_string()).await
}

async fn asset_owned(path: String) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref().to_string())], file.data.into_owned()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn error(e: impl std::fmt::Display) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response()
}

fn query(state: &Shared, sql: &str) -> Response {
    let conn = state.conn.lock().unwrap();
    match db::query_objects(&conn, sql) {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}

async fn rides(State(state): State<Shared>) -> Response {
    query(&state, "SELECT * FROM rides ORDER BY start_utc")
}

async fn xp_sources(State(state): State<Shared>) -> Response {
    query(&state, "SELECT * FROM xp_sources ORDER BY start_utc, xp DESC")
}

async fn status(State(state): State<Shared>) -> Response {
    let conn = state.conn.lock().unwrap();
    let sql = "SELECT
        (SELECT count(*) FROM rides) AS rides,
        (SELECT count(*) FROM rides WHERE xp IS NOT NULL) AS rides_with_xp,
        (SELECT count(*) FROM activities) AS log_activities,
        (SELECT count(*) FROM log_files) AS archived_logs,
        (SELECT count(*) FROM fit_files) AS fit_files,
        (SELECT count(*) FROM legacy_rides) AS legacy_rides,
        (SELECT max(archived_at) FROM log_files) AS last_new_log";
    match db::query_objects(&conn, sql) {
        Ok(Value::Array(mut rows)) if !rows.is_empty() => {
            let mut row = rows.remove(0);
            row["zwift_dir"] = json!(state.paths.zwift_dir.display().to_string());
            row["data_dir"] = json!(state.paths.data_dir.display().to_string());
            row["imports"] = json!(state.imports.load(Ordering::Relaxed));
            row["last_import"] = json!(*state.last_import.lock().unwrap());
            Json(row).into_response()
        }
        Ok(_) => error("no status"),
        Err(e) => error(e),
    }
}

async fn run_import(State(state): State<Shared>) -> Response {
    match state.import() {
        Ok(r) => Json(json!({
            "logs_archived": r.logs_archived,
            "activities": r.activities,
            "fit_new": r.fit_new,
            "fit_failed": r.fit_failed,
            "legacy_rides": r.legacy_rides,
        }))
        .into_response(),
        Err(e) => error(e),
    }
}

async fn run_sync(State(state): State<Shared>) -> Response {
    match state.sync().await {
        Ok(r) => Json(json!({ "activities": r.activities, "followers": r.followers, "followees": r.followees, "events": r.events }))
            .into_response(),
        Err(e) => error(format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

/// Signs in from the Companion tab; the password is used for this one request and never stored
async fn login(State(state): State<Shared>, Json(req): Json<LoginRequest>) -> Response {
    let name = match companion::sign_in(&state.paths.data_dir, &req.email, &req.password).await {
        Ok(name) => name,
        Err(e) => return error(format!("{e:#}")),
    };
    drop(req);
    let sync = state.sync().await.err().map(|e| format!("{e:#}"));
    Json(json!({ "name": name, "sync_error": sync })).into_response()
}

async fn logout(State(state): State<Shared>) -> Response {
    // Signing out also forgets what was fetched with that sign-in
    let cleared = state.conn.lock().unwrap().execute_batch(
        "DELETE FROM zwift_profile; DELETE FROM zwift_activities; DELETE FROM zwift_follows; DELETE FROM zwift_events;",
    );
    match companion::sign_out(&state.paths.data_dir).and_then(|_| Ok(cleared?)) {
        Ok(_) => {
            state.imports.fetch_add(1, Ordering::Relaxed);
            Json(json!({ "signed_out": true })).into_response()
        }
        Err(e) => error(e),
    }
}

/// Everything the Companion tab shows, in one response
async fn companion_data(State(state): State<Shared>) -> Response {
    let conn = state.conn.lock().unwrap();
    let q = |sql: &str| db::query_objects(&conn, sql);
    let recent = (chrono::Utc::now() - chrono::Duration::hours(1)).format("%Y-%m-%d %H:%M:%S");
    let result = (|| -> Result<Value> {
        Ok(json!({
            "signed_in": companion::signed_in(&state.paths.data_dir),
            "profile": q("SELECT * EXCLUDE (raw) FROM zwift_profile")?,
            "riding_now": q("SELECT first_name, last_name, country, world_id FROM zwift_follows
                             WHERE direction = 'followee' AND riding ORDER BY first_name")?,
            "follows": q("SELECT direction, count(*) AS n FROM zwift_follows GROUP BY ALL")?,
            "events": q(&format!("SELECT id, name, start_utc, sport, event_type, map_id, distance_m, duration_s, laps
                         FROM zwift_events WHERE start_utc >= TIMESTAMP '{recent}' ORDER BY start_utc LIMIT 300"))?,
            "activities": q("SELECT id, name, start_utc, distance_m, elevation_m, avg_watts, moving_s, ride_ons, comments
                             FROM zwift_activities ORDER BY start_utc DESC LIMIT 100")?,
        }))
    })();
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(e),
    }
}

#[derive(Deserialize)]
struct SqlRequest {
    sql: String,
}

/// Read-only explorer queries. A single statement starting with a read keyword is allowed.
pub fn is_read_only(sql: &str) -> bool {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    if trimmed.contains(';') {
        return false;
    }
    let first = trimmed.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    matches!(
        first.as_str(),
        "select" | "with" | "from" | "describe" | "summarize" | "show" | "explain" | "pivot" | "unpivot" | "values" | "table"
    )
}

async fn sql(State(state): State<Shared>, Json(req): Json<SqlRequest>) -> Response {
    if !is_read_only(&req.sql) {
        return error("Only a single read-only query (SELECT, WITH, FROM, DESCRIBE, SUMMARIZE, ...) is allowed.");
    }
    let conn = state.conn.lock().unwrap();
    match db::query_json(&conn, req.sql.trim().trim_end_matches(';'), SQL_ROW_LIMIT) {
        Ok(r) => Json(json!({ "columns": r.columns, "rows": r.rows, "limit": SQL_ROW_LIMIT })).into_response(),
        Err(e) => error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::is_read_only;

    #[test]
    fn read_only_guard() {
        assert!(is_read_only("select * from rides"));
        assert!(is_read_only("  WITH x AS (select 1) select * from x;"));
        assert!(is_read_only("summarize rides"));
        assert!(!is_read_only("delete from rides"));
        assert!(!is_read_only("select 1; drop table rides"));
        assert!(!is_read_only("attach 'x.db'"));
        assert!(!is_read_only("copy rides to 'x.csv'"));
    }
}
