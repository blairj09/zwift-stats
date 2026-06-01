import fs from 'fs';
import os from 'os';
import path from 'path';
import { isAlreadyProcessed } from './bests.js';
import { processFitFile } from './pipeline.js';

export interface BackfillOptions {
  dryRun?: boolean;
  logsDir?: string;
  activitiesDir?: string;
}

interface DiscoveredRide {
  fitFileName: string;
  fitFilePath: string;
  logFile: string;
}

function getGameLogFiles(logsDir: string): string[] {
  return fs
    .readdirSync(logsDir)
    .filter(f => f.startsWith('Log') && f.endsWith('.txt'))
    .map(f => path.join(logsDir, f))
    .sort((a, b) => fs.statSync(a).mtimeMs - fs.statSync(b).mtimeMs); // oldest first
}

function scanLogForRides(logFile: string, activitiesDir: string): DiscoveredRide[] {
  const content = fs.readFileSync(logFile, 'utf8');
  const rides: DiscoveredRide[] = [];
  const seen = new Set<string>();

  // Match the final save line per ride: uploadTo3P: True with a real filename (not inProgressActivity)
  const lineRe = /uploadTo3P: True, fitFileNameToUpload: (?!.*inProgressActivity).*fitFileNameShort: ([^}]+\.fit)/;

  for (const line of content.split('\n')) {
    const m = line.match(lineRe);
    if (!m) continue;
    const fitFileName = m[1].trim();
    if (seen.has(fitFileName)) continue;
    seen.add(fitFileName);
    rides.push({ fitFileName, fitFilePath: path.join(activitiesDir, fitFileName), logFile });
  }

  return rides;
}

export async function runBackfill(opts: BackfillOptions = {}): Promise<void> {
  const logsDir = opts.logsDir ?? path.join(os.homedir(), 'Documents', 'Zwift', 'Logs');
  const activitiesDir = opts.activitiesDir ?? path.join(os.homedir(), 'Documents', 'Zwift', 'Activities');

  if (!fs.existsSync(logsDir)) {
    console.error(`Logs directory not found: ${logsDir}`);
    return;
  }

  const logFiles = getGameLogFiles(logsDir);
  console.log(`\nScanning ${logFiles.length} log file(s) for rides...`);

  // Collect all rides, deduplicating across files — if a ride appears in multiple logs,
  // prefer the first (oldest) occurrence since that log is most likely to have the full session.
  const allRides = new Map<string, DiscoveredRide>();
  for (const logFile of logFiles) {
    for (const ride of scanLogForRides(logFile, activitiesDir)) {
      if (!allRides.has(ride.fitFileName)) {
        allRides.set(ride.fitFileName, ride);
      }
    }
  }

  const unprocessed = [...allRides.values()].filter(r => !isAlreadyProcessed(r.fitFileName));
  const alreadyDone = allRides.size - unprocessed.length;

  console.log(`Found ${allRides.size} ride(s): ${alreadyDone} already processed, ${unprocessed.length} to backfill.\n`);

  if (unprocessed.length === 0) {
    console.log('Nothing to do.');
    return;
  }

  let processed = 0;
  let skipped = 0;

  for (const ride of unprocessed) {
    if (!fs.existsSync(ride.fitFilePath)) {
      console.log(`\nSkipping ${ride.fitFileName} — FIT file not on disk.`);
      skipped++;
      continue;
    }

    await processFitFile(ride.fitFilePath, { dryRun: opts.dryRun, logPath: ride.logFile });
    processed++;
  }

  console.log(`\nBackfill complete: ${processed} processed, ${skipped} skipped (FIT file missing).`);
}
