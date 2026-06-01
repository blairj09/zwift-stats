import fs from 'fs';
import FitParser from 'fit-file-parser';

export interface FitSession {
  startTime: Date;
  durationSeconds: number;
}

export function parseFit(fitPath: string): Promise<FitSession> {
  return new Promise((resolve, reject) => {
    const parser = new (FitParser as any)({ force: true, speedUnit: 'km/h', lengthUnit: 'km' });
    const buf = fs.readFileSync(fitPath);
    parser.parse(buf, (err: Error | null, data: any) => {
      if (err) return reject(new Error(`FIT parse error: ${err.message}`));
      const session = data?.sessions?.[0];
      if (!session) return reject(new Error('No session record found in FIT file'));
      const startTime = new Date(session.start_time ?? session.timestamp);
      const durationSeconds: number = session.total_elapsed_time ?? 0;
      resolve({ startTime, durationSeconds });
    });
  });
}
