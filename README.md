# zwift-stats

Automatically appends Zwift XP and Drops stats to your Strava activity descriptions after every ride. Tracks personal bests and highlights them inline.

```
--- Zwift Stats ---
XP: 🏆 2,482  (🏆 1,338/hr)
Drops: 🏆 52,385  (🏆 28,236/hr)
```

## How it works

Zwift writes a plaintext summary to `~/Documents/Zwift/Logs/Log.txt` at the end of every activity, including XP earned and Drops earned. A local daemon watches your Zwift Activities folder for new `.fit` files (the ride-complete signal), reads the corresponding log entry, computes per-hour rates, checks personal bests, and updates the Strava activity description automatically — typically within 30–60 seconds of saving your ride.

> **Why not the FIT file?** Zwift doesn't embed XP or Drops in the FIT files it uploads to Strava. The log file is the only reliable local source for these values.

## Requirements

- **macOS** (the daemon uses the macOS LaunchAgent system; Zwift paths are macOS defaults)
- **Node.js 18+**
- **Zwift** installed with activities syncing to Strava
- **A Strava API application** (free, takes 2 minutes to create)

## Setup

### 1. Clone and install dependencies

```bash
git clone https://github.com/jamesblair/zwift-stats.git
cd zwift-stats
npm install
```

### 2. Create a Strava API application

1. Go to [strava.com/settings/api](https://www.strava.com/settings/api)
2. Fill in any name and website (these are just labels for your own reference)
3. Set **Authorization Callback Domain** to `localhost`
4. Note your **Client ID** and **Client Secret**

If you already have a Strava API app (e.g. for another integration), you can reuse the same credentials — each OAuth flow generates independent tokens.

### 3. Authenticate with Strava

```bash
npm run auth
```

This will prompt for your Client ID and Client Secret, open your browser to Strava's authorization page, and save tokens to `~/.config/zwift-stats/`. You only need to do this once.

### 4. Install the daemon

```bash
npm run daemon:install
```

This generates a correctly-pathed LaunchAgent plist for your machine, installs it to `~/Library/LaunchAgents/`, and starts it immediately. The daemon will auto-start on every login.

That's it. The next time you finish a Zwift ride and hit **Save**, your Strava description will be updated automatically.

## Usage

### Automatic

Once the daemon is installed, everything is hands-off. Finish a ride in Zwift → save → description appears on Strava within ~60 seconds.

### Manual processing

To process a specific ride (useful for backfilling past rides that are in your log):

```bash
npm run process -- ~/Documents/Zwift/Activities/2026-06-01-09-41-22.fit
```

Preview without updating Strava:

```bash
npm run process -- ~/Documents/Zwift/Activities/2026-06-01-09-41-22.fit --dry-run
```

### Daemon management

```bash
npm run daemon:status     # Check if the daemon is running
npm run daemon:install    # Install (or reinstall after moving the project)
npm run daemon:uninstall  # Stop and remove the daemon
```

## Personal bests

Personal bests are tracked globally across all rides and stored in a local SQLite database at `~/.local/share/zwift-stats/bests.db`. Four metrics are tracked:

| Metric | Description |
|---|---|
| `xp_per_hour` | Best XP rate — favours intense, focused efforts |
| `drops_per_hour` | Best Drops rate |
| `total_xp` | Most XP earned in a single ride — favours long rides |
| `total_drops` | Most Drops earned in a single ride |

A 🏆 appears inline next to any value that sets a new personal best:

```
XP: 🏆 2,482  (🏆 1,338/hr)   ← both total and rate are new records
XP: 2,482  (🏆 1,338/hr)       ← rate PR only (short hard effort)
XP: 🏆 2,482  (1,338/hr)       ← total PR only (longest ride)
XP: 2,482  (1,338/hr)          ← neither
```

## Duplicate activity handling

If Strava has multiple Virtual Ride activities in the same time window (e.g. a duplicate from a different recording device), the tool identifies the Zwift-uploaded activity by its `external_id` field (`zwift-activity-...`), which Strava sets when Zwift uploads the file directly.

## Long-term maintenance

This tool is designed to be set-and-forget:

- **Strava tokens** auto-refresh silently. If you ever revoke access in Strava settings, re-run `npm run auth`.
- **Zwift updates** occasionally change log formatting. If descriptions stop appearing, run a `--dry-run` to diagnose. The relevant patterns are in `src/log-parser.ts`.
- **After moving the project** to a different directory, re-run `npm run daemon:install` to regenerate the plist with updated paths.

## Project structure

```
src/
  index.ts        CLI entry point
  auth.ts         Strava OAuth flow
  watcher.ts      Watches ~/Documents/Zwift/Activities for new .fit files
  log-parser.ts   Extracts XP/Drops from ~/Documents/Zwift/Logs/Log.txt
  fit-parser.ts   Extracts duration and start time from .fit files
  pipeline.ts     Orchestrates one ride end-to-end
  strava.ts       Strava API client (token refresh, activity match, update)
  bests.ts        SQLite personal bests store
  format.ts       Builds the description block string
  daemon.ts       Installs/uninstalls the macOS LaunchAgent
```
