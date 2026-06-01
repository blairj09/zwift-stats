import Database from 'better-sqlite3';
import fs from 'fs';
import os from 'os';
import path from 'path';

const DATA_DIR = path.join(os.homedir(), '.local', 'share', 'zwift-stats');
const DB_PATH = path.join(DATA_DIR, 'bests.db');

let _db: Database.Database | null = null;

function db(): Database.Database {
  if (_db) return _db;
  fs.mkdirSync(DATA_DIR, { recursive: true });
  _db = new Database(DB_PATH);
  _db.exec(`
    CREATE TABLE IF NOT EXISTS personal_bests (
      metric      TEXT PRIMARY KEY,
      value       REAL NOT NULL,
      achieved_at TEXT NOT NULL,
      strava_activity_id INTEGER,
      ride_name   TEXT
    );

    CREATE TABLE IF NOT EXISTS rides (
      id                 INTEGER PRIMARY KEY AUTOINCREMENT,
      fit_file           TEXT UNIQUE NOT NULL,
      strava_activity_id INTEGER,
      processed_at       TEXT NOT NULL,
      xp                 INTEGER,
      drops              INTEGER,
      duration_seconds   INTEGER,
      xp_per_hour        REAL,
      drops_per_hour     REAL
    );
  `);
  return _db;
}

export type Metric = 'xp_per_hour' | 'drops_per_hour' | 'total_xp' | 'total_drops';

export interface PersonalBest {
  metric: Metric;
  value: number;
  achievedAt: string;
  stravaActivityId?: number;
  rideName?: string;
}

export function getBest(metric: Metric): PersonalBest | null {
  const row = db()
    .prepare('SELECT * FROM personal_bests WHERE metric = ?')
    .get(metric) as any;
  if (!row) return null;
  return {
    metric: row.metric,
    value: row.value,
    achievedAt: row.achieved_at,
    stravaActivityId: row.strava_activity_id ?? undefined,
    rideName: row.ride_name ?? undefined,
  };
}

export function upsertBest(best: PersonalBest): void {
  db()
    .prepare(`
      INSERT INTO personal_bests (metric, value, achieved_at, strava_activity_id, ride_name)
      VALUES (?, ?, ?, ?, ?)
      ON CONFLICT(metric) DO UPDATE SET
        value = excluded.value,
        achieved_at = excluded.achieved_at,
        strava_activity_id = excluded.strava_activity_id,
        ride_name = excluded.ride_name
    `)
    .run(
      best.metric,
      best.value,
      best.achievedAt,
      best.stravaActivityId ?? null,
      best.rideName ?? null,
    );
}

export interface RideRecord {
  fitFile: string;
  stravaActivityId?: number;
  processedAt: string;
  xp: number;
  drops: number;
  durationSeconds: number;
  xpPerHour: number;
  dropsPerHour: number;
}

export function isAlreadyProcessed(fitFile: string): boolean {
  const row = db().prepare('SELECT id FROM rides WHERE fit_file = ?').get(fitFile);
  return !!row;
}

export function saveRide(ride: RideRecord): void {
  db()
    .prepare(`
      INSERT OR IGNORE INTO rides
        (fit_file, strava_activity_id, processed_at, xp, drops, duration_seconds, xp_per_hour, drops_per_hour)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    `)
    .run(
      ride.fitFile,
      ride.stravaActivityId ?? null,
      ride.processedAt,
      ride.xp,
      ride.drops,
      ride.durationSeconds,
      ride.xpPerHour,
      ride.dropsPerHour,
    );
}

export function updateRideStravaId(fitFile: string, stravaActivityId: number): void {
  db()
    .prepare('UPDATE rides SET strava_activity_id = ? WHERE fit_file = ?')
    .run(stravaActivityId, fitFile);
}

export interface CheckedBests {
  newXpPerHour: boolean;
  newDropsPerHour: boolean;
  newTotalXp: boolean;
  newTotalDrops: boolean;
}

export function previewBests(
  xpPerHour: number,
  dropsPerHour: number,
  endingTotalXp: number,
  endingDrops: number,
): CheckedBests {
  const isNew = (metric: Metric, value: number) => {
    const existing = getBest(metric);
    return !existing || value > existing.value;
  };
  return {
    newXpPerHour: isNew('xp_per_hour', xpPerHour),
    newDropsPerHour: isNew('drops_per_hour', dropsPerHour),
    newTotalXp: isNew('total_xp', endingTotalXp),
    newTotalDrops: isNew('total_drops', endingDrops),
  };
}

export function checkAndUpdateBests(
  xpPerHour: number,
  dropsPerHour: number,
  endingTotalXp: number,
  endingDrops: number,
  achievedAt: string,
  stravaActivityId?: number,
  rideName?: string,
): CheckedBests {
  const result: CheckedBests = {
    newXpPerHour: false,
    newDropsPerHour: false,
    newTotalXp: false,
    newTotalDrops: false,
  };

  const check = (metric: Metric, value: number, key: keyof CheckedBests) => {
    const existing = getBest(metric);
    if (!existing || value > existing.value) {
      upsertBest({ metric, value, achievedAt, stravaActivityId, rideName });
      result[key] = true;
    }
  };

  check('xp_per_hour', xpPerHour, 'newXpPerHour');
  check('drops_per_hour', dropsPerHour, 'newDropsPerHour');
  check('total_xp', endingTotalXp, 'newTotalXp');
  check('total_drops', endingDrops, 'newTotalDrops');

  return result;
}
