import fs from 'fs';
import os from 'os';
import path from 'path';
import { getRides, rebuildBests, updateRideDetails, updateRideName } from './bests.js';
import { parseFit } from './fit-parser.js';
import { getActivity } from './strava.js';

export interface RefreshOptions {
  names?: boolean;
  activitiesDir?: string;
}

// Strava allows 100 read requests per 15 minutes; leave headroom for the daemon
const MAX_NAME_LOOKUPS = 80;

/**
 * Re-reads every stored ride's FIT file to fill in start time, distance, elevation and power,
 * optionally fetches ride names from Strava (read-only), then rebuilds personal bests.
 * Never writes to Strava.
 */
export async function runRefresh(opts: RefreshOptions = {}): Promise<void> {
  const activitiesDir = opts.activitiesDir ?? path.join(os.homedir(), 'Documents', 'Zwift', 'Activities');
  const rides = getRides();
  console.log(`\nRefreshing ${rides.length} stored ride(s) from ${activitiesDir}...`);

  let updated = 0;
  let missing = 0;
  let failed = 0;
  for (const ride of rides) {
    const fitPath = path.join(activitiesDir, ride.fitFile);
    if (!fs.existsSync(fitPath)) {
      missing++;
      continue;
    }
    try {
      const fit = await parseFit(fitPath);
      updateRideDetails(ride.fitFile, {
        startTime: fit.startTime.toISOString(),
        movingSeconds: fit.movingSeconds ?? undefined,
        distanceKm: fit.distanceKm ?? undefined,
        elevationM: fit.elevationM ?? undefined,
        avgPower: fit.avgPower ?? undefined,
        avgHeartRate: fit.avgHeartRate ?? undefined,
      });
      updated++;
    } catch (err: any) {
      console.log(`  ${ride.fitFile}: ${err.message}`);
      failed++;
    }
  }
  console.log(`  FIT details: ${updated} updated, ${missing} FIT file(s) not on disk, ${failed} unreadable.`);

  if (opts.names) {
    const unnamed = getRides().filter(r => r.stravaActivityId && !r.rideName);
    const batch = unnamed.slice(0, MAX_NAME_LOOKUPS);
    console.log(`  Fetching ${batch.length} of ${unnamed.length} missing ride name(s) from Strava...`);
    for (const ride of batch) {
      try {
        const activity = await getActivity(ride.stravaActivityId!);
        updateRideName(ride.fitFile, activity.name);
      } catch (err: any) {
        console.log(`  Stopped fetching names: ${err.message}`);
        break;
      }
    }
    if (unnamed.length > batch.length) {
      console.log('  Strava rate limit reached for this run — run `zwift-stats refresh --names` again in 15 minutes.');
    }
  }

  rebuildBests();
  console.log('  Personal bests rebuilt from ride history.\n');
}
