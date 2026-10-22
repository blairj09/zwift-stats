//! Session summary from a Zwift `.fit` file. Zwift's FIT files carry no XP or drops, but they
//! cover every ride ever saved, long after the game log that recorded its earnings has rotated away.

use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use fitparser::profile::MesgNum;
use fitparser::{FitDataRecord, Value};

#[derive(Debug, Clone, Default)]
pub struct FitSummary {
    pub start_utc: String,
    pub elapsed_s: f64,
    pub moving_s: Option<f64>,
    pub distance_m: Option<f64>,
    pub ascent_m: Option<f64>,
    pub calories: Option<f64>,
    pub avg_power: Option<f64>,
    pub max_power: Option<f64>,
    pub normalized_power: Option<f64>,
    pub avg_hr: Option<f64>,
    pub max_hr: Option<f64>,
    pub avg_cadence: Option<f64>,
    pub sport: Option<String>,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Float64(x) => Some(*x),
        Value::Float32(x) => Some(*x as f64),
        Value::UInt8(x) => Some(*x as f64),
        Value::UInt16(x) => Some(*x as f64),
        Value::UInt32(x) => Some(*x as f64),
        Value::UInt64(x) => Some(*x as f64),
        Value::SInt8(x) => Some(*x as f64),
        Value::SInt16(x) => Some(*x as f64),
        Value::SInt32(x) => Some(*x as f64),
        Value::SInt64(x) => Some(*x as f64),
        _ => None,
    }
}

/// Reads the session summary. `Ok(None)` for files Zwift writes for activities that never
/// started (a few hundred bytes with no session or no start time).
pub fn read_summary(path: &Path) -> Result<Option<FitSummary>> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let records: Vec<FitDataRecord> = fitparser::from_reader(&mut file).map_err(|e| anyhow!("{e}"))?;
    let Some(session) = records.iter().find(|r| r.kind() == MesgNum::Session) else {
        return Ok(None);
    };

    let mut s = FitSummary::default();
    for field in session.fields() {
        let v = field.value();
        match field.name() {
            "start_time" => {
                if let Value::Timestamp(t) = v {
                    s.start_utc = t.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string();
                }
            }
            "total_elapsed_time" => s.elapsed_s = num(v).unwrap_or(0.0),
            "total_timer_time" => s.moving_s = num(v),
            "total_distance" => s.distance_m = num(v),
            "total_ascent" => s.ascent_m = num(v),
            "total_calories" => s.calories = num(v),
            "avg_power" => s.avg_power = num(v),
            "max_power" => s.max_power = num(v),
            "normalized_power" => s.normalized_power = num(v),
            "avg_heart_rate" => s.avg_hr = num(v),
            "max_heart_rate" => s.max_hr = num(v),
            "avg_cadence" => s.avg_cadence = num(v),
            "sport" => s.sport = Some(v.to_string()),
            _ => {}
        }
    }
    Ok((!s.start_utc.is_empty()).then_some(s))
}
