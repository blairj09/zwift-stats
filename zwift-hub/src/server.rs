use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use duckdb::Connection;
use rust_embed::Embed;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::db;
use crate::import::{self, Paths};

#[derive(Embed)]
#[folder = "assets/"]
struct Assets;

struct AppState {
    conn: Mutex<Connection>,
    paths: Paths,
}

type Shared = Arc<AppState>;

const SQL_ROW_LIMIT: usize = 5000;

pub async fn serve(conn: Connection, paths: Paths, port: u16) -> Result<()> {
    let state = Arc::new(AppState { conn: Mutex::new(conn), paths });
    let app = Router::new()
        .route("/", get(|| asset("index.html")))
        .route("/assets/{*file}", get(|Path(file): Path<String>| asset_owned(file)))
        .route("/api/rides", get(rides))
        .route("/api/xp_sources", get(xp_sources))
        .route("/api/status", get(status))
        .route("/api/import", post(run_import))
        .route("/api/sql", post(sql))
        .with_state(state);

    // Loopback only: the database is personal and the SQL endpoint is unauthenticated
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("Zwift Hub running at http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
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
            Json(row).into_response()
        }
        Ok(_) => error("no status"),
        Err(e) => error(e),
    }
}

async fn run_import(State(state): State<Shared>) -> Response {
    let conn = state.conn.lock().unwrap();
    match import::run(&conn, &state.paths) {
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
