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
  migrateRides(_db);
  return _db;
}

// Columns added after the first release; ALTERed in on open so existing databases keep their rides
const RIDE_COLUMNS: Record<string, string> = {
  start_time: 'TEXT',
  ride_name: 'TEXT',
  moving_seconds: 'INTEGER',
  distance_km: 'REAL',
  elevation_m: 'REAL',
  avg_power: 'REAL',
  avg_hr: 'REAL',
};

function migrateRides(database: Database.Database): void {
  const existing = new Set(
    (database.prepare('PRAGMA table_info(rides)').all() as any[]).map(c => c.name),
  );
  for (const [name, type] of Object.entries(RIDE_COLUMNS)) {
    if (!existing.has(name)) database.exec(`ALTER TABLE rides ADD COLUMN ${name} ${type}`);
  }
}

export type Metric =
  | 'xp_per_hour'
  | 'drops_per_hour'
  | 'xp_per_km'
  | 'drops_per_km'
  | 'total_xp'
  | 'total_drops';

// Per-km bests ignore very short rides, where a single segment or badge skews the rate
export const MIN_KM_FOR_RATE_BEST = 5;

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
  startTime?: string;
  rideName?: string;
  xp: number;
  drops: number;
  durationSeconds: number;
  movingSeconds?: number;
  distanceKm?: number;
  elevationM?: number;
  avgPower?: number;
  avgHeartRate?: number;
  xpPerHour: number;
  dropsPerHour: number;
}

export type RideDetails = Pick<
  RideRecord,
  'startTime' | 'movingSeconds' | 'distanceKm' | 'elevationM' | 'avgPower' | 'avgHeartRate'
>;

export function isAlreadyProcessed(fitFile: string): boolean {
  const row = db().prepare('SELECT id FROM rides WHERE fit_file = ?').get(fitFile);
  return !!row;
}

export function saveRide(ride: RideRecord): void {
  db()
    .prepare(`
      INSERT OR IGNORE INTO rides
        (fit_file, strava_activity_id, processed_at, start_time, ride_name, xp, drops,
         duration_seconds, moving_seconds, distance_km, elevation_m, avg_power, avg_hr,
         xp_per_hour, drops_per_hour)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `)
    .run(
      ride.fitFile,
      ride.stravaActivityId ?? null,
      ride.processedAt,
      ride.startTime ?? null,
      ride.rideName ?? null,
      ride.xp,
      ride.drops,
      ride.durationSeconds,
      ride.movingSeconds ?? null,
      ride.distanceKm ?? null,
      ride.elevationM ?? null,
      ride.avgPower ?? null,
      ride.avgHeartRate ?? null,
      ride.xpPerHour,
      ride.dropsPerHour,
    );
}

export function updateRideDetails(fitFile: string, details: RideDetails): void {
  db()
    .prepare(`
      UPDATE rides SET
        start_time = ?, moving_seconds = ?, distance_km = ?, elevation_m = ?, avg_power = ?, avg_hr = ?
      WHERE fit_file = ?
    `)
    .run(
      details.startTime ?? null,
      details.movingSeconds ?? null,
      details.distanceKm ?? null,
      details.elevationM ?? null,
      details.avgPower ?? null,
      details.avgHeartRate ?? null,
      fitFile,
    );
}

export function updateRideStravaId(fitFile: string, stravaActivityId: number, rideName?: string): void {
  db()
    .prepare('UPDATE rides SET strava_activity_id = ?, ride_name = COALESCE(?, ride_name) WHERE fit_file = ?')
    .run(stravaActivityId, rideName ?? null, fitFile);
}

export function updateRideName(fitFile: string, rideName: string): void {
  db().prepare('UPDATE rides SET ride_name = ? WHERE fit_file = ?').run(rideName, fitFile);
}

export interface CheckedBests {
  newXpPerHour: boolean;
  newDropsPerHour: boolean;
  newXpPerKm: boolean;
  newDropsPerKm: boolean;
  newTotalXp: boolean;
  newTotalDrops: boolean;
}

export interface BestCandidates {
  xp: number;
  drops: number;
  xpPerHour: number;
  dropsPerHour: number;
  distanceKm?: number;
}

function candidateValues(c: BestCandidates): [Metric, number | null, keyof CheckedBests][] {
  const perKm = c.distanceKm && c.distanceKm >= MIN_KM_FOR_RATE_BEST;
  return [
    ['xp_per_hour', c.xpPerHour, 'newXpPerHour'],
    ['drops_per_hour', c.dropsPerHour, 'newDropsPerHour'],
    ['xp_per_km', perKm ? c.xp / c.distanceKm! : null, 'newXpPerKm'],
    ['drops_per_km', perKm ? c.drops / c.distanceKm! : null, 'newDropsPerKm'],
    ['total_xp', c.xp, 'newTotalXp'],
    ['total_drops', c.drops, 'newTotalDrops'],
  ];
}

function noBests(): CheckedBests {
  return {
    newXpPerHour: false,
    newDropsPerHour: false,
    newXpPerKm: false,
    newDropsPerKm: false,
    newTotalXp: false,
    newTotalDrops: false,
  };
}

export function previewBests(c: BestCandidates): CheckedBests {
  const result = noBests();
  for (const [metric, value, key] of candidateValues(c)) {
    if (value === null) continue;
    const existing = getBest(metric);
    result[key] = !existing || value > existing.value;
  }
  return result;
}

export function getRides(): RideRecord[] {
  const rows = db()
    .prepare('SELECT * FROM rides ORDER BY COALESCE(start_time, processed_at) ASC')
    .all() as any[];
  return rows.map(r => ({
    fitFile: r.fit_file,
    stravaActivityId: r.strava_activity_id ?? undefined,
    processedAt: r.processed_at,
    startTime: r.start_time ?? undefined,
    rideName: r.ride_name ?? undefined,
    xp: r.xp,
    drops: r.drops,
    durationSeconds: r.duration_seconds,
    movingSeconds: r.moving_seconds ?? undefined,
    distanceKm: r.distance_km ?? undefined,
    elevationM: r.elevation_m ?? undefined,
    avgPower: r.avg_power ?? undefined,
    avgHeartRate: r.avg_hr ?? undefined,
    xpPerHour: r.xp_per_hour,
    dropsPerHour: r.drops_per_hour,
  }));
}

/** When the ride happened: the FIT start time, falling back to when it was processed. */
export function rideDate(ride: RideRecord): string {
  return ride.startTime ?? ride.processedAt;
}

export function getAllBests(): PersonalBest[] {
  const rows = db().prepare('SELECT * FROM personal_bests').all() as any[];
  return rows.map(r => ({
    metric: r.metric as Metric,
    value: r.value,
    achievedAt: r.achieved_at,
    stravaActivityId: r.strava_activity_id ?? undefined,
    rideName: r.ride_name ?? undefined,
  }));
}

export function checkAndUpdateBests(
  c: BestCandidates,
  achievedAt: string,
  stravaActivityId?: number,
  rideName?: string,
): CheckedBests {
  const result = noBests();
  for (const [metric, value, key] of candidateValues(c)) {
    if (value === null) continue;
    const existing = getBest(metric);
    if (!existing || value > existing.value) {
      upsertBest({ metric, value, achievedAt, stravaActivityId, rideName });
      result[key] = true;
    }
  }
  return result;
}

/** Recompute every personal best from the rides table, replacing what is stored. */
export function rebuildBests(): void {
  const rides = getRides();
  db().transaction(() => {
    db().exec('DELETE FROM personal_bests');
    for (const r of rides) {
      checkAndUpdateBests(
        { xp: r.xp, drops: r.drops, xpPerHour: r.xpPerHour, dropsPerHour: r.dropsPerHour, distanceKm: r.distanceKm },
        rideDate(r),
        r.stravaActivityId,
        r.rideName,
      );
    }
  })();
}
