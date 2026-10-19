import fs from 'fs';
import os from 'os';
import path from 'path';
import { execSync } from 'child_process';
import { getRides, getAllBests, rideDate } from './bests.js';

const TEMPLATE_PATH = path.join(__dirname, 'dashboard.html');
const DEFAULT_OUTPUT = path.join(os.homedir(), '.local', 'share', 'zwift-stats', 'dashboard.html');

export function generateChart(outputPath?: string): void {
  const rides = getRides();

  if (rides.length === 0) {
    console.log('No rides recorded yet. Process a FIT file first.');
    return;
  }

  const data = {
    generatedAt: new Date().toISOString(),
    rides: rides.map(r => ({
      date: rideDate(r),
      name: r.rideName ?? null,
      stravaId: r.stravaActivityId ?? null,
      xp: r.xp,
      drops: r.drops,
      durS: r.durationSeconds,
      km: r.distanceKm ?? null,
      elevM: r.elevationM ?? null,
      power: r.avgPower ?? null,
      hr: r.avgHeartRate ?? null,
    })),
    bests: getAllBests(),
  };

  // Escape "<" so ride names can never close the script tag
  const json = JSON.stringify(data).replace(/</g, '\\u003c');
  const html = fs.readFileSync(TEMPLATE_PATH, 'utf8').replace('/*__DATA__*/null', json);

  const dest = outputPath ? path.resolve(outputPath.replace(/^~/, os.homedir())) : DEFAULT_OUTPUT;
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.writeFileSync(dest, html, 'utf8');
  console.log(`Dashboard written to: ${dest}`);

  const missingKm = rides.filter(r => r.distanceKm === undefined).length;
  if (missingKm > 0) {
    console.log(`${missingKm} ride(s) have no distance yet; run \`zwift-stats refresh\` to fill them in from FIT files.`);
  }

  try {
    execSync(`open "${dest}"`);
  } catch {
    console.log('Could not open browser automatically. Open the file above manually.');
  }
}
