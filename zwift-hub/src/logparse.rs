//! Parses Zwift's game log (`Documents/Zwift/Logs/Log.txt`).
//!
//! At the end of every activity Zwift dumps an `[ECONOMY] Activity:` block with the ride's
//! UTC start/end, distance, climbing, XP, drops and level totals, followed by a `Streaks:` block.
//! The block is written several times per activity (at the start with 0 XP, and on save), so we
//! keep the last block for each start time. Lines between the activity being created and its
//! final block are attributed to it: XP grants by source, ride-ons and the group event joined.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LogActivity {
    pub start_utc: String,
    pub end_utc: String,
    pub sport: String,
    /// Economy name, e.g. "Free ride on:  Sand And Sequoias"
    pub economy_name: String,
    /// What kind of ride it was, from the saved title when it says more than the economy name:
    /// "Free ride", "Workout", "Pacer group ride", "Group ride", "Race", ...
    pub kind: String,
    /// Route from the economy name (text after "on:")
    pub route: String,
    /// The title Zwift saves and uploads, e.g. "Zwift - Out And Back Again in Watopia"
    pub title: Option<String>,
    pub world: Option<String>,
    pub world_id: Option<i64>,
    pub fit_file: Option<String>,
    pub distance_m: f64,
    pub elevation_m: f64,
    pub calories: f64,
    pub xp: i64,
    pub drops: i64,
    pub start_level: i64,
    pub end_level: i64,
    pub start_total_xp: i64,
    pub end_total_xp: i64,
    pub start_drops: i64,
    pub end_drops: i64,
    pub streak_weeks: Option<i64>,
    pub week_ride_count: Option<i64>,
    pub bonus_xp: Option<i64>,
    pub event_name: Option<String>,
    pub ride_ons_received: i64,
    pub ride_ons_given: i64,
    pub xp_by_source: BTreeMap<String, i64>,
    pub game_version: Option<String>,
}

static GAME_VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Game Version: (\S+)").unwrap());
static CREATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Creating New Activity \{worldId: (\d+)\}").unwrap());
static SAVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"save_activity with \{name: (.*), uploadTo3P: True, fitFileNameToUpload: .*fitFileNameShort: ([^}]+\.fit)\}").unwrap()
});
static GRANT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[XP\] Granting XP: (\d+), XP Type: (\w+)").unwrap());
static RIDE_ON_RECEIVED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"HUD_Notify: .* says Ride On!").unwrap());
static RIDE_ON_GIVEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Total Ride Ons Given: (\d+)").unwrap());
static EVENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[Group Events\] Registration success for group "([^"]+)""#).unwrap());
static BONUS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[ECONOMY\] Last Ride Bonus XP: (\d+)").unwrap());
static KV: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z ]*?)\s*:\s?(.*)$").unwrap());
static TITLE_WORLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" in ([A-Z][A-Za-z ]+)$").unwrap());

/// Per-activity context gathered from log lines since the activity was created.
#[derive(Default)]
struct Segment {
    world_id: Option<i64>,
    title: Option<String>,
    fit_file: Option<String>,
    event_name: Option<String>,
    ride_ons_received: i64,
    ride_ons_given: i64,
    xp_by_source: BTreeMap<String, i64>,
}

pub fn parse_log(text: &str) -> Vec<LogActivity> {
    let lines: Vec<&str> = text.lines().collect();
    let mut game_version = None;
    let mut seg = Segment::default();
    // Keyed by start time so the last block written for an activity wins
    let mut by_start: BTreeMap<String, LogActivity> = BTreeMap::new();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if game_version.is_none() {
            if let Some(c) = GAME_VERSION.captures(line) {
                game_version = Some(c[1].to_string());
            }
        }
        if let Some(c) = CREATE.captures(line) {
            seg = Segment { world_id: c[1].parse().ok(), ..Default::default() };
        } else if let Some(c) = SAVE.captures(line) {
            seg.title = Some(c[1].trim().to_string());
            seg.fit_file = Some(c[2].trim().to_string());
        } else if let Some(c) = GRANT.captures(line) {
            *seg.xp_by_source.entry(c[2].to_string()).or_default() += c[1].parse::<i64>().unwrap_or(0);
        } else if RIDE_ON_RECEIVED.is_match(line) {
            seg.ride_ons_received += 1;
        } else if RIDE_ON_GIVEN.is_match(line) {
            seg.ride_ons_given += 1;
        } else if let Some(c) = EVENT.captures(line) {
            seg.event_name = Some(c[1].to_string());
        } else if line.contains("[ECONOMY] Activity:") {
            let (fields, next) = read_block(&lines, i + 1);
            let mut act = activity_from_fields(&fields);
            i = next;
            // Streaks and bonus XP follow the activity block
            let mut j = i;
            while j < lines.len() && j < i + 40 {
                if lines[j].contains("[ECONOMY] Streaks:") {
                    let (streak, _) = read_block(&lines, j + 1);
                    act.streak_weeks = streak.get("Cur streak length").and_then(|v| v.parse().ok());
                    act.week_ride_count = streak.get("Week Qualifying Ride Count").and_then(|v| v.parse().ok());
                } else if let Some(c) = BONUS.captures(lines[j]) {
                    act.bonus_xp = c[1].parse().ok();
                    break;
                }
                j += 1;
            }
            if act.start_utc.is_empty() {
                continue;
            }
            act.world_id = seg.world_id;
            act.title = seg.title.clone();
            act.world = seg
                .title
                .as_deref()
                .and_then(|t| TITLE_WORLD.captures(t))
                .map(|c| c[1].to_string())
                .or_else(|| seg.world_id.and_then(world_name).map(str::to_string));
            if let Some(title) = seg.title.as_deref() {
                act.kind = ride_kind(title, &act.kind);
            }
            act.fit_file = seg.fit_file.clone();
            act.event_name = seg.event_name.clone();
            act.ride_ons_received = seg.ride_ons_received;
            act.ride_ons_given = seg.ride_ons_given;
            act.xp_by_source = seg.xp_by_source.clone();
            act.game_version = game_version.clone();
            let keep = match by_start.get(&act.start_utc) {
                // A later block for the same ride replaces an earlier one unless it lost the earnings
                Some(prev) => act.xp >= prev.xp || act.drops >= prev.drops,
                None => true,
            };
            if keep {
                by_start.insert(act.start_utc.clone(), act);
            }
            continue;
        }
        i += 1;
    }

    by_start
        .into_values()
        .filter(|a| a.xp > 0 || a.drops > 0 || a.distance_m > 100.0)
        .collect()
}

/// Zwift's world ids, for activities whose saved title doesn't name the world
fn world_name(id: i64) -> Option<&'static str> {
    Some(match id {
        1 => "Watopia",
        2 => "Richmond",
        3 => "London",
        4 => "New York",
        5 => "Innsbruck",
        6 => "Bologna",
        7 => "Yorkshire",
        8 => "Crit City",
        9 => "Makuri Islands",
        10 => "France",
        11 => "Paris",
        13 => "Scotland",
        _ => return None,
    })
}

/// The economy block calls most rides "Free ride"; the saved title knows about group rides,
/// pacer rides, races and third-party workouts.
fn ride_kind(title: &str, economy_kind: &str) -> String {
    let t = title.to_ascii_lowercase();
    let kind = if t.contains("pacer group ride") {
        "Pacer group ride"
    } else if t.contains("race") {
        "Race"
    } else if t.contains("group ride") || t.contains("group workout") {
        "Group ride"
    } else if t.contains("trainerroad") || t.contains("workout") || economy_kind == "Workout" {
        "Workout"
    } else if economy_kind.is_empty() {
        "Free ride"
    } else {
        economy_kind
    };
    kind.to_string()
}

/// Reads `Key : value` lines until a blank or timestamped line. Returns the fields and the next index.
fn read_block(lines: &[&str], start: usize) -> (BTreeMap<String, String>, usize) {
    let mut fields = BTreeMap::new();
    let mut i = start;
    while i < lines.len() {
        let l = lines[i].trim_end();
        if l.is_empty() || l.starts_with('[') {
            break;
        }
        if let Some(c) = KV.captures(l) {
            fields.insert(c[1].trim().to_string(), c[2].to_string());
        }
        i += 1;
    }
    (fields, i)
}

fn activity_from_fields(f: &BTreeMap<String, String>) -> LogActivity {
    let s = |k: &str| f.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let n = |k: &str| f.get(k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
    let i = |k: &str| f.get(k).and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0);
    let economy_name = s("Name");
    let (kind, route) = match economy_name.split_once(" on:") {
        Some((k, r)) => (k.trim().to_string(), r.trim().to_string()),
        None => (String::new(), economy_name.clone()),
    };
    LogActivity {
        start_utc: s("Start UTC time"),
        end_utc: s("End UTC time"),
        sport: s("Sport"),
        economy_name,
        kind,
        route,
        distance_m: n("Dist traveled meters"),
        elevation_m: n("Elevation climbed meters"),
        calories: n("Calories burned"),
        xp: i("Earned XP"),
        drops: i("Earned drops"),
        start_level: i("Starting Level"),
        end_level: i("Ending Level"),
        start_total_xp: i("Starting Total XP"),
        end_total_xp: i("Ending Total XP"),
        start_drops: i("Starting Drops"),
        end_drops: i("Ending Drops"),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
[7:55:04] Log Time: 7:55:04 2026-10-15
[7:55:04] Game Version: 1.123.0(167050) rc/1.123.0
[7:58:30] INFO LEVEL: [SaveActivityService] GameLoadLevel: Creating New Activity {worldId: 1}
[7:58:38] [ECONOMY] Activity:
Sport                    : CYCLING
Name                     : Free ride on:Downtown Eruption
Start UTC time           : 2026-10-15T13:58:38Z
End UTC time             : 2026-10-15T13:58:38Z
Dist traveled meters     : 0.000000
Elevation climbed meters : 0.000000
Calories burned          : 0.000000
Earned XP                : 0
Earned drops             : 0

[7:58:40] INFO LEVEL: [Group Events] Registration success for group \"Stage 2 - Tour of Watopia 2026\", subgroup \"Standard\"
[8:00:00] [XP] Granting XP: 20, XP Type: XP_DISTANCE
[8:01:00] [XP] Granting XP: 20, XP Type: XP_DISTANCE
[8:02:00] [XP] Granting XP: 250, XP Type: XP_CONFETTI_SOCKS
[8:03:00] HUD_Notify: J.Doe [TEAM] says Ride On!
[8:04:00] Total Ride Ons Given: 1
[8:48:04] INFO LEVEL: [SaveActivityService] ZNet::SaveActivity calling zwift_network::save_activity with {name: Zwift - Out And Back Again in Watopia, uploadTo3P: True, fitFileNameToUpload: /Users/x/Documents/Zwift/Activities/2026-10-15-07-58-41.fit, fitFileNameShort: 2026-10-15-07-58-41.fit}
[8:48:05] [ECONOMY] Activity:
Sport                    : CYCLING
Name                     : Free ride on:Downtown Eruption
Start UTC time           : 2026-10-15T13:58:38Z
End UTC time             : 2026-10-15T14:48:04Z
Dist traveled meters     : 25012.738281
Elevation climbed meters : 312.680756
Calories burned          : 541.766541
Earned XP                : 1500
Earned drops             : 31547
Starting Level           : 102
Starting Total XP        : 857556
Starting Target XP       : 857556
Starting Drops           : 1225487
Ending Level             : 102
Ending Total XP          : 859056
Ending Target XP         : 859056
Ending Drops             : 1257034

[8:48:05] [ECONOMY] Streaks:
Cur streak length                 : 113
Week Qualifying Ride Count        : 2

[8:48:05] [ECONOMY] Last Ride Bonus XP: 0
";

    #[test]
    fn parses_final_activity_block_with_context() {
        let acts = parse_log(SAMPLE);
        assert_eq!(acts.len(), 1);
        let a = &acts[0];
        assert_eq!(a.start_utc, "2026-10-15T13:58:38Z");
        assert_eq!(a.end_utc, "2026-10-15T14:48:04Z");
        assert_eq!(a.kind, "Free ride");
        assert_eq!(a.economy_name, "Free ride on:Downtown Eruption");
        assert_eq!(a.route, "Downtown Eruption");
        assert_eq!(a.title.as_deref(), Some("Zwift - Out And Back Again in Watopia"));
        assert_eq!(a.world.as_deref(), Some("Watopia"));
        assert_eq!(a.world_id, Some(1));
        assert_eq!(a.fit_file.as_deref(), Some("2026-10-15-07-58-41.fit"));
        assert_eq!(a.xp, 1500);
        assert_eq!(a.drops, 31547);
        assert_eq!(a.end_total_xp, 859056);
        assert_eq!(a.streak_weeks, Some(113));
        assert_eq!(a.week_ride_count, Some(2));
        assert_eq!(a.bonus_xp, Some(0));
        assert_eq!(a.event_name.as_deref(), Some("Stage 2 - Tour of Watopia 2026"));
        assert_eq!(a.ride_ons_received, 1);
        assert_eq!(a.ride_ons_given, 1);
        assert_eq!(a.xp_by_source.get("XP_DISTANCE"), Some(&40));
        assert_eq!(a.xp_by_source.get("XP_CONFETTI_SOCKS"), Some(&250));
        assert_eq!(a.game_version.as_deref(), Some("1.123.0(167050)"));
        assert!((a.distance_m - 25012.738281).abs() < 1e-6);
    }

    #[test]
    fn ride_kind_prefers_title() {
        assert_eq!(ride_kind("Zwift - Pacer Group Ride with Yumi", "Free ride"), "Pacer group ride");
        assert_eq!(ride_kind("Zwift - Group Ride: Standard - Stage 2 - Tour of Watopia 2026", "Free ride"), "Group ride");
        assert_eq!(ride_kind("Zwift - TrainerRoad: Bashful", "Free ride"), "Workout");
        assert_eq!(ride_kind("Zwift - Race: Crit City Showdown", "Free ride"), "Race");
        assert_eq!(ride_kind("Zwift - Sand And Sequoias in Watopia", "Free ride"), "Free ride");
    }

    #[test]
    fn world_falls_back_to_world_id() {
        let text = SAMPLE.replace("Out And Back Again in Watopia", "Pacer Group Ride with Coco");
        let a = &parse_log(&text)[0];
        assert_eq!(a.world.as_deref(), Some("Watopia"));
        assert_eq!(a.kind, "Pacer group ride");
    }

    #[test]
    fn drops_empty_activities() {
        let text = SAMPLE.split("[7:58:40]").next().unwrap();
        assert!(parse_log(text).is_empty());
    }
}
