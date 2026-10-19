#!/usr/bin/env node
import path from 'path';
import os from 'os';

const [, , command, ...args] = process.argv;

async function main() {
  switch (command) {
    case 'auth': {
      const { runAuth } = await import('./auth.js');
      await runAuth();
      break;
    }

    case 'start': {
      const { startWatcher } = await import('./watcher.js');
      startWatcher();
      break;
    }

    case 'daemon': {
      const sub = args[0];
      const { installDaemon, uninstallDaemon, daemonStatus } = await import('./daemon.js');
      if (sub === 'install') installDaemon();
      else if (sub === 'uninstall') uninstallDaemon();
      else if (sub === 'status') daemonStatus();
      else {
        console.error('Usage: zwift-stats daemon install|uninstall|status');
        process.exit(1);
      }
      break;
    }

    case 'backfill': {
      const { runBackfill } = await import('./backfill.js');
      await runBackfill({ dryRun: args.includes('--dry-run') });
      break;
    }

    case 'stats': {
      const { printStats } = await import('./summary.js');
      printStats();
      break;
    }

    case 'refresh': {
      const { runRefresh } = await import('./refresh.js');
      await runRefresh({ names: args.includes('--names') });
      break;
    }

    case 'dashboard':
    case 'chart': {
      const { generateChart } = await import('./chart.js');
      const outputFlag = args.indexOf('--output');
      const outputPath = outputFlag !== -1 ? args[outputFlag + 1] : undefined;
      generateChart(outputPath);
      break;
    }

    case 'process': {
      const fitArg = args.find(a => !a.startsWith('--'));
      const dryRun = args.includes('--dry-run');

      if (!fitArg) {
        console.error('Usage: zwift-stats process <path-to-fit-file> [--dry-run]');
        process.exit(1);
      }

      // Resolve relative paths; also support bare filenames from the Activities dir
      let fitPath = path.resolve(fitArg);
      if (!require('fs').existsSync(fitPath)) {
        const fallback = path.join(os.homedir(), 'Documents', 'Zwift', 'Activities', fitArg);
        if (require('fs').existsSync(fallback)) {
          fitPath = fallback;
        } else {
          // Try current directory
          fitPath = path.resolve(process.cwd(), fitArg);
        }
      }

      const { processFitFile } = await import('./pipeline.js');
      await processFitFile(fitPath, { dryRun });
      break;
    }

    default: {
      console.log(`zwift-stats — Auto-annotate Strava with Zwift XP and Drops

Commands:
  auth                           Set up Strava OAuth (run once)
  start                          Start the file-watching daemon
  process <file> [--dry-run]     Manually process a FIT file
  backfill [--dry-run]           Scan all Zwift logs and process historical rides
  stats                          Print lifetime XP/Drops summary statistics
  refresh [--names]              Fill distance/elevation from FIT files (and ride names from Strava), rebuild bests
  dashboard [--output <file>]    Generate the XP/Drops dashboard and open in browser (alias: chart)
  daemon install                 Install and start the macOS LaunchAgent
  daemon uninstall               Stop and remove the macOS LaunchAgent
  daemon status                  Check whether the daemon is running
`);
      if (command) {
        console.error(`Unknown command: ${command}`);
        process.exit(1);
      }
    }
  }
}

main().catch(err => {
  console.error('Fatal error:', err.message);
  process.exit(1);
});
