import path from 'path';
import { parseFit } from './fit-parser.js';
import { parseLog } from './log-parser.js';
import { checkAndUpdateBests, previewBests, isAlreadyProcessed, saveRide, updateRideStravaId } from './bests.js';
import { buildBlock, appendBlock, hasMarker } from './format.js';
import { findActivity, getActivity, updateDescription } from './strava.js';

export interface PipelineOptions {
  dryRun?: boolean;
  logPath?: string;
}

export async function processFitFile(fitPath: string, opts: PipelineOptions = {}): Promise<void> {
  const fitFileName = path.basename(fitPath);
  console.log(`\nProcessing: ${fitFileName}`);

  // Idempotency check
  if (!opts.dryRun && isAlreadyProcessed(fitFileName)) {
    console.log('  Already processed — skipping.');
    return;
  }

  // Parse FIT for duration and start time
  const fit = await parseFit(fitPath);
  console.log(`  Start time: ${fit.startTime.toISOString()}`);
  console.log(`  Duration: ${Math.round(fit.durationSeconds / 60)} min`);

  if (fit.durationSeconds < 60) {
    console.log('  Duration < 60s — skipping (likely a cancelled/test activity).');
    return;
  }

  // Parse log for XP and Drops
  const stats = parseLog(fitFileName, opts.logPath);
  if (!stats) {
    console.log('  Could not find XP/Drops in log — skipping.');
    return;
  }

  const hours = fit.durationSeconds / 3600;
  const xpPerHour = stats.xp / hours;
  const dropsPerHour = stats.drops / hours;

  console.log(`  XP: ${stats.xp.toLocaleString('en-GB')} (${Math.round(xpPerHour).toLocaleString('en-GB')}/hr)`);
  console.log(`  Drops: ${stats.drops.toLocaleString('en-GB')} (${Math.round(dropsPerHour).toLocaleString('en-GB')}/hr)`);

  if (opts.dryRun) {
    console.log('\n  [DRY RUN] Checking personal bests without saving...');
    const prs = previewBests(xpPerHour, dropsPerHour, stats.endingTotalXp, stats.endingDrops);
    const block = buildBlock(stats.xp, stats.drops, fit.durationSeconds, prs);
    console.log('\n  Description block that would be appended:\n');
    console.log(block.split('\n').map(l => `    ${l}`).join('\n'));
    return;
  }

  // Save ride record first (marks as processed even if Strava fails)
  saveRide({
    fitFile: fitFileName,
    processedAt: new Date().toISOString(),
    xp: stats.xp,
    drops: stats.drops,
    durationSeconds: fit.durationSeconds,
    xpPerHour,
    dropsPerHour,
  });

  // Find matching Strava activity
  console.log('  Searching for Strava activity...');
  const activity = await findActivity(fit.startTime);
  if (!activity) {
    console.log('  Strava activity not found after retries. Will not update description.');
    return;
  }

  console.log(`  Found Strava activity: "${activity.name}" (id: ${activity.id})`);

  // Fetch full activity to get existing description
  const full = await getActivity(activity.id);
  const existing = full.description ?? '';

  if (hasMarker(existing)) {
    console.log('  Description already has Zwift Stats block — skipping update.');
    updateRideStravaId(fitFileName, activity.id);
    return;
  }

  // Check and update personal bests
  const prs = checkAndUpdateBests(
    xpPerHour, dropsPerHour, stats.endingTotalXp, stats.endingDrops,
    fit.startTime.toISOString(),
    activity.id,
    activity.name,
  );

  if (prs.newXpPerHour) console.log('  🏆 New best XP/hr!');
  if (prs.newDropsPerHour) console.log('  🏆 New best Drops/hr!');

  const block = buildBlock(stats.xp, stats.drops, fit.durationSeconds, prs);
  const newDescription = appendBlock(existing, block);

  await updateDescription(activity.id, newDescription);
  updateRideStravaId(fitFileName, activity.id);

  console.log('  Strava description updated successfully.');
}
