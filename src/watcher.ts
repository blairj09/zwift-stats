import chokidar from 'chokidar';
import path from 'path';
import os from 'os';
import { processFitFile } from './pipeline.js';

const ACTIVITIES_DIR = path.join(os.homedir(), 'Documents', 'Zwift', 'Activities');
const SETTLE_DELAY_MS = 5_000;

export function startWatcher(activitiesDir = ACTIVITIES_DIR): void {
  console.log(`Watching for new Zwift activities in:\n  ${activitiesDir}\n`);

  const watcher = chokidar.watch(`${activitiesDir}/*.fit`, {
    ignoreInitial: true,
    awaitWriteFinish: { stabilityThreshold: 2000, pollInterval: 500 },
  });

  watcher.on('add', (filePath: string) => {
    const fileName = path.basename(filePath);

    // Ignore the in-progress file — only process named final files
    if (fileName === 'inProgressActivity.fit') return;

    console.log(`New FIT file detected: ${fileName}`);
    console.log(`Waiting ${SETTLE_DELAY_MS / 1000}s for log to finish writing...`);

    setTimeout(async () => {
      try {
        await processFitFile(filePath);
      } catch (err) {
        console.error(`Error processing ${fileName}:`, err);
      }
    }, SETTLE_DELAY_MS);
  });

  watcher.on('error', (err: Error) => console.error('Watcher error:', err));

  process.on('SIGINT', () => {
    console.log('\nShutting down watcher.');
    watcher.close();
    process.exit(0);
  });
}
