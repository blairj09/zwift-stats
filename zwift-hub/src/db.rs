use std::path::Path;

use anyhow::Result;
use duckdb::Connection;
use duckdb::types::{TimeUnit, Value};
use serde_json::{Map, Value as Json};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS log_files (
    sha256        VARCHAR PRIMARY KEY,
    original_name VARCHAR,
    log_started   VARCHAR,
    bytes         BIGINT,
    archived_at   TIMESTAMP DEFAULT current_timestamp
);

-- One row per activity from the game log's [ECONOMY] block
CREATE TABLE IF NOT EXISTS activities (
    start_utc         TIMESTAMP PRIMARY KEY,
    end_utc           TIMESTAMP,
    sport             VARCHAR,
    kind              VARCHAR,
    route             VARCHAR,
    title             VARCHAR,
    world             VARCHAR,
    world_id          INTEGER,
    fit_file          VARCHAR,
    distance_m        DOUBLE,
    elevation_m       DOUBLE,
    calories          DOUBLE,
    xp                BIGINT,
    drops             BIGINT,
    start_level       INTEGER,
    end_level         INTEGER,
    start_total_xp    BIGINT,
    end_total_xp      BIGINT,
    start_drops       BIGINT,
    end_drops         BIGINT,
    streak_weeks      INTEGER,
    week_ride_count   INTEGER,
    bonus_xp          BIGINT,
    event_name        VARCHAR,
    ride_ons_received INTEGER,
    ride_ons_given    INTEGER,
    game_version      VARCHAR
);

CREATE TABLE IF NOT EXISTS xp_sources (
    start_utc TIMESTAMP,
    source    VARCHAR,
    xp        BIGINT,
    PRIMARY KEY (start_utc, source)
);

-- Session summary of every .fit file in the Activities folder
CREATE TABLE IF NOT EXISTS fit_files (
    fit_file         VARCHAR PRIMARY KEY,
    start_utc        TIMESTAMP,
    elapsed_s        DOUBLE,
    moving_s         DOUBLE,
    distance_m       DOUBLE,
    ascent_m         DOUBLE,
    calories         DOUBLE,
    avg_power        DOUBLE,
    max_power        DOUBLE,
    normalized_power DOUBLE,
    avg_hr           DOUBLE,
    max_hr           DOUBLE,
    avg_cadence      DOUBLE,
    sport            VARCHAR,
    file_mtime       BIGINT
);

-- XP/drops recorded by the earlier zwift-stats tool, for rides whose logs are gone
CREATE TABLE IF NOT EXISTS legacy_rides (
    fit_file           VARCHAR PRIMARY KEY,
    xp                 BIGINT,
    drops              BIGINT,
    duration_s         DOUBLE,
    strava_activity_id BIGINT,
    ride_name          VARCHAR
);

-- From Zwift's API after `zwift-hub login`; `raw` keeps the full JSON for fields not broken out
CREATE TABLE IF NOT EXISTS zwift_profile (
    id BIGINT PRIMARY KEY, first_name VARCHAR, last_name VARCHAR, country VARCHAR, ftp DOUBLE,
    weight_kg DOUBLE, height_cm DOUBLE, achievement_level BIGINT, total_xp BIGINT, drops BIGINT,
    total_km DOUBLE, total_climb_m DOUBLE, total_hours DOUBLE, followers BIGINT, followees BIGINT,
    fetched_at TIMESTAMP, raw VARCHAR
);

CREATE TABLE IF NOT EXISTS app_settings (key VARCHAR PRIMARY KEY, value VARCHAR);

-- What was done for each ride on Strava (posted, already had the block, not found yet, error)
CREATE TABLE IF NOT EXISTS strava_posts (
    start_utc TIMESTAMP PRIMARY KEY, status VARCHAR, strava_id BIGINT, updated_at TIMESTAMP,
    attempts BIGINT, error VARCHAR
);

-- Your XP and Drops totals at each sync. Between two of them, if you rode only once, the
-- difference is what that ride earned: the only way to know earnings for rides recorded on
-- another device, which leave no game log on this Mac.
CREATE TABLE IF NOT EXISTS zwift_profile_history (
    fetched_at TIMESTAMP PRIMARY KEY, total_xp BIGINT, drops BIGINT, achievement_level BIGINT
);

CREATE TABLE IF NOT EXISTS zwift_activities (
    id VARCHAR PRIMARY KEY, name VARCHAR, start_utc TIMESTAMP, end_utc TIMESTAMP, distance_m DOUBLE,
    elevation_m DOUBLE, avg_watts DOUBLE, calories DOUBLE, moving_s DOUBLE, sport VARCHAR,
    world_id BIGINT, ride_ons BIGINT, comments BIGINT, privacy VARCHAR, raw VARCHAR
);

CREATE TABLE IF NOT EXISTS zwift_follows (
    direction VARCHAR, profile_id BIGINT, first_name VARCHAR, last_name VARCHAR, country VARCHAR,
    riding BOOLEAN, world_id BIGINT, status VARCHAR, raw VARCHAR,
    PRIMARY KEY (direction, profile_id)
);

CREATE TABLE IF NOT EXISTS zwift_events (
    id BIGINT PRIMARY KEY, name VARCHAR, start_utc TIMESTAMP, sport VARCHAR, event_type VARCHAR,
    map_id BIGINT, route_id BIGINT, distance_m DOUBLE, duration_s DOUBLE, laps BIGINT, raw VARCHAR
);
"#;

/// Every ride, whichever sources know about it. Earnings come from the game log when it still
/// exists, else from the legacy database; ride metrics prefer the FIT file. Rides only Zwift's
/// API knows about (recorded on a phone, Apple TV or another computer) come last, with XP and
/// Drops estimated from the change in your totals around them.
pub const RIDES_VIEW: &str = r#"
CREATE OR REPLACE VIEW rides AS
WITH joined AS (
    SELECT
        coalesce(a.start_utc, f.start_utc)                           AS start_utc,
        coalesce(a.fit_file, f.fit_file)                             AS fit_file,
        coalesce(a.title, l.ride_name)                               AS title,
        coalesce(a.route, regexp_extract(l.ride_name, '^Zwift - (.*?)( in [A-Z][A-Za-z ]+)?$', 1)) AS route,
        coalesce(a.world, regexp_extract(l.ride_name, ' in ([A-Z][A-Za-z ]+)$', 1)) AS world,
        a.kind,
        coalesce(a.sport, upper(f.sport))                            AS sport,
        a.event_name,
        coalesce(a.xp, l.xp)                                         AS xp,
        coalesce(a.drops, l.drops)                                   AS drops,
        coalesce(f.elapsed_s, epoch(a.end_utc) - epoch(a.start_utc), l.duration_s) AS duration_s,
        f.moving_s,
        coalesce(f.distance_m, a.distance_m)                         AS distance_m,
        coalesce(f.ascent_m, a.elevation_m)                          AS elevation_m,
        coalesce(f.calories, a.calories)                             AS calories,
        f.avg_power, f.max_power, f.normalized_power, f.avg_hr, f.max_hr, f.avg_cadence,
        a.end_level                                                  AS level,
        a.end_total_xp                                               AS total_xp,
        a.end_drops                                                  AS total_drops,
        a.streak_weeks,
        a.ride_ons_received, a.ride_ons_given,
        l.strava_activity_id,
        CASE WHEN a.start_utc IS NOT NULL THEN 'log'
             WHEN l.fit_file IS NOT NULL THEN 'legacy'
             ELSE 'fit' END                                          AS earnings_source
    FROM activities a
    FULL OUTER JOIN fit_files f ON f.fit_file = a.fit_file
    LEFT JOIN legacy_rides l ON l.fit_file = coalesce(a.fit_file, f.fit_file)
),
-- Known totals over time: each sync, plus the before and after of every logged ride
totals AS (
    SELECT fetched_at AS t, total_xp AS xp, drops FROM zwift_profile_history
    UNION ALL SELECT start_utc, start_total_xp, start_drops FROM activities WHERE start_total_xp IS NOT NULL
    UNION ALL SELECT end_utc, end_total_xp, end_drops FROM activities WHERE end_total_xp IS NOT NULL
),
api AS (
    SELECT z.*, coalesce(z.end_utc, z.start_utc + to_seconds(coalesce(z.moving_s, 0)::BIGINT)) AS finish
    FROM zwift_activities z
    WHERE z.start_utc IS NOT NULL
      AND NOT EXISTS (SELECT 1 FROM joined j WHERE abs(epoch(j.start_utc) - epoch(z.start_utc)) <= 300)
),
api_before AS (
    SELECT api.*, b.t AS t0, b.xp AS xp0, b.drops AS drops0
    FROM api ASOF LEFT JOIN totals b ON api.start_utc >= b.t
),
api_totals AS (
    SELECT ab.*, a.t AS t1, a.xp AS xp1, a.drops AS drops1,
        (SELECT count(*) FROM zwift_activities o WHERE o.start_utc > ab.t0 AND o.start_utc < a.t) AS rides_between
    FROM api_before ab ASOF LEFT JOIN totals a ON ab.finish <= a.t
),
api_rides AS (
    SELECT
        start_utc, NULL AS fit_file, name AS title,
        regexp_extract(name, '^Zwift - (?:.*: )?(.*?)( in [A-Z][A-Za-z ]+)?$', 1) AS route,
        nullif(regexp_extract(name, ' in ([A-Z][A-Za-z ]+)$', 1), '') AS world,
        CASE WHEN name ILIKE '%pacer%' THEN 'Pacer group ride'
             WHEN name ILIKE '%race%' THEN 'Race'
             WHEN name ILIKE '%group ride%' THEN 'Group ride'
             WHEN name ILIKE '%workout%' OR name ILIKE 'TrainerRoad%' THEN 'Workout'
             ELSE 'Free ride' END AS kind,
        upper(sport) AS sport, NULL AS event_name,
        -- Only when this was the sole ride between two known totals; Drops can also fall from
        -- spending them, so a drop in the balance gives no estimate
        CASE WHEN rides_between = 1 AND xp1 >= xp0 THEN xp1 - xp0 END AS xp,
        CASE WHEN rides_between = 1 AND drops1 >= drops0 THEN drops1 - drops0 END AS drops,
        epoch(finish) - epoch(start_utc) AS duration_s, moving_s, distance_m, elevation_m, calories,
        avg_watts AS avg_power, NULL AS max_power, NULL AS normalized_power, NULL AS avg_hr, NULL AS max_hr,
        NULL AS avg_cadence, NULL AS level, NULL AS total_xp, NULL AS total_drops, NULL AS streak_weeks,
        ride_ons AS ride_ons_received, NULL AS ride_ons_given, NULL AS strava_activity_id,
        'api' AS earnings_source
    FROM api_totals
),
all_rides AS (
    SELECT * FROM joined
    UNION ALL BY NAME
    SELECT * FROM api_rides
)
SELECT *,
    distance_m / 1000                                      AS km,
    duration_s / 3600                                      AS hours,
    xp    / nullif(duration_s / 3600, 0)                   AS xp_per_hour,
    drops / nullif(duration_s / 3600, 0)                   AS drops_per_hour,
    xp    / nullif(distance_m / 1000, 0)                   AS xp_per_km,
    drops / nullif(distance_m / 1000, 0)                   AS drops_per_km,
    drops / nullif(elevation_m, 0)                         AS drops_per_m_climbed
FROM all_rides
WHERE duration_s >= 60 OR xp > 0
"#;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(RIDES_VIEW)?;
    // The explorer runs user-typed SQL; nothing here needs DuckDB to read or write other files
    conn.execute_batch("SET enable_external_access = false; SET lock_configuration = true;")?;
    Ok(conn)
}

fn value_to_json(v: Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Boolean(b) => Json::Bool(b),
        Value::TinyInt(x) => x.into(),
        Value::SmallInt(x) => x.into(),
        Value::Int(x) => x.into(),
        Value::BigInt(x) => x.into(),
        Value::HugeInt(x) => (x as f64).into(),
        Value::UHugeInt(x) => (x as f64).into(),
        Value::UTinyInt(x) => x.into(),
        Value::USmallInt(x) => x.into(),
        Value::UInt(x) => x.into(),
        Value::UBigInt(x) => x.into(),
        Value::Float(x) => Json::from(x as f64),
        Value::Double(x) => Json::from(x),
        Value::Decimal(d) => d.to_string().parse::<f64>().map(Json::from).unwrap_or(Json::Null),
        Value::Timestamp(unit, t) => {
            let micros = match unit {
                TimeUnit::Second => t * 1_000_000,
                TimeUnit::Millisecond => t * 1_000,
                TimeUnit::Microsecond => t,
                TimeUnit::Nanosecond => t / 1_000,
            };
            chrono::DateTime::from_timestamp_micros(micros)
                .map(|d| Json::String(d.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
                .unwrap_or(Json::Null)
        }
        Value::Date32(days) => chrono::NaiveDate::from_num_days_from_ce_opt(days + 719_163)
            .map(|d| Json::String(d.to_string()))
            .unwrap_or(Json::Null),
        Value::Text(s) | Value::Enum(s) => Json::String(s),
        Value::List(l) | Value::Array(l) => Json::Array(l.into_iter().map(value_to_json).collect()),
        other => Json::String(format!("{other:?}")),
    }
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Json>>,
}

/// Runs a query and returns column names and rows as JSON values, up to `limit` rows.
pub fn query_json(conn: &Connection, sql: &str, limit: usize) -> Result<QueryResult> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;
    let columns = rows.as_ref().map(|s| s.column_names()).unwrap_or_default();
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        if out.len() >= limit {
            break;
        }
        let mut r = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            r.push(value_to_json(row.get::<_, Value>(i)?));
        }
        out.push(r);
    }
    Ok(QueryResult { columns, rows: out })
}

/// Rows as objects keyed by column name.
pub fn query_objects(conn: &Connection, sql: &str) -> Result<Json> {
    let res = query_json(conn, sql, usize::MAX)?;
    Ok(Json::Array(
        res.rows
            .into_iter()
            .map(|r| Json::Object(res.columns.iter().cloned().zip(r).collect::<Map<_, _>>()))
            .collect(),
    ))
}
