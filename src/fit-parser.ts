import fs from 'fs';
import FitParser from 'fit-file-parser';

export interface FitSession {
  startTime: Date;
  durationSeconds: number;
  movingSeconds: number | null;
  distanceKm: number | null;
  elevationM: number | null;
  avgPower: number | null;
  avgHeartRate: number | null;
}

function numOrNull(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

export function parseFit(fitPath: string): Promise<FitSession> {
  return new Promise((resolve, reject) => {
    // Lengths stay in metres (lengthUnit applies to ascent as well as distance); converted below
    const parser = new (FitParser as any)({ force: true, speedUnit: 'km/h', lengthUnit: 'm' });
    const buf = fs.readFileSync(fitPath);
    parser.parse(buf, (err: Error | null, data: any) => {
      if (err) return reject(new Error(`FIT parse error: ${err.message}`));
      const session = data?.sessions?.[0];
      if (!session) return reject(new Error('No session record found in FIT file'));
      const startTime = new Date(session.start_time ?? session.timestamp);
      const durationSeconds: number = session.total_elapsed_time ?? 0;
      const distanceM = numOrNull(session.total_distance);
      resolve({
        startTime,
        durationSeconds,
        movingSeconds: numOrNull(session.total_timer_time),
        distanceKm: distanceM !== null ? distanceM / 1000 : null,
        elevationM: numOrNull(session.total_ascent),
        avgPower: numOrNull(session.avg_power),
        avgHeartRate: numOrNull(session.avg_heart_rate),
      });
    });
  });
}
