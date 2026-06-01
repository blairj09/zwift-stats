# zwift-stats

> **macOS only.** This tool relies on the macOS LaunchAgent system for background execution and expects Zwift's default file paths on macOS (`~/Documents/Zwift/`). It will not work on Windows or Linux.

Automatically appends Zwift XP and Drops stats to your Strava activity descriptions after every ride. Tracks personal bests and highlights them inline.

```
--- Zwift Stats ---
XP: 🏆 2,482  (🏆 1,338/hr)
Drops: 🏆 52,385  (🏆 28,236/hr)
```

## How it works

Zwift writes a plaintext summary to `~/Documents/Zwift/Logs/Log.txt` at the end of every activity, including XP earned and Drops earned. A local daemon watches your Zwift Activities folder for new `.fit` files (the ride-complete signal), reads the corresponding log entry, computes per-hour rates, checks personal bests, and updates the Strava activity description — typically within 30–60 seconds of saving your ride.

> **Why not the FIT file?** Zwift doesn't embed XP or Drops in the FIT files it uploads to Strava. The log file is the only reliable local source for these values.

## Requirements

- **macOS**
- **Node.js 18+**
- **Zwift** installed with activities syncing to Strava
- **A Strava API application** (free, takes 2 minutes to create)

## Setup

### 1. Clone, install, and link

```bash
git clone https://github.com/blairj09/zwift-stats.git
cd zwift-stats
npm install
npm link
```

`npm link` registers `zwift-stats` as a global command so you can run it from anywhere without `npm run`.

### 2. Create a Strava API application

1. Go to [strava.com/settings/api](https://www.strava.com/settings/api)
2. Fill in any name and website (these are just labels for your own reference)
3. Set **Authorization Callback Domain** to `localhost`
4. Note your **Client ID** and **Client Secret**

If you already have a Strava API app (e.g. for another integration), you can reuse the same credentials — each OAuth flow generates independent tokens.

### 3. Authenticate with Strava

```bash
zwift-stats auth
```

This prompts for your Client ID and Client Secret, opens your browser to Strava's authorization page, and saves tokens to `~/.config/zwift-stats/`. You only need to do this once.

### 4. Install the daemon

```bash
zwift-stats daemon install
```

This generates a correctly-pathed LaunchAgent plist for your machine, installs it to `~/Library/LaunchAgents/`, and starts it immediately. The daemon auto-starts on every login.

That's it. The next time you finish a Zwift ride and hit **Save**, your Strava description will be updated automatically.

## CLI reference

```bash
zwift-stats auth                        # One-time Strava OAuth setup
zwift-stats process <file>              # Manually process a specific .fit file
zwift-stats process <file> --dry-run    # Preview without updating Strava
zwift-stats backfill                    # Process all historical rides from Zwift logs
zwift-stats backfill --dry-run          # Preview backfill without saving or updating Strava
zwift-stats stats                       # Print lifetime XP/Drops summary statistics
zwift-stats chart                       # Generate charts and open in browser
zwift-stats chart --output <file>       # Write chart HTML to a specific path
zwift-stats daemon install              # Install and start the background daemon
zwift-stats daemon uninstall            # Stop and remove the daemon
zwift-stats daemon status               # Check whether the daemon is running
```

## Backfill

If you installed `zwift-stats` after riding for a while, or if the daemon missed some rides, you can retroactively process historical rides:

```bash
zwift-stats backfill
```

This scans all log files in `~/Documents/Zwift/Logs/` (including rotated `Log (old N).txt` files), discovers every completed ride, and processes any not already in the local database. XP and Drops are read from the log file that recorded each session. Rides whose `.fit` files are no longer on disk are skipped gracefully.

## Stats and charts

Two commands let you explore your accumulated XP and Drops history.

`zwift-stats stats` prints a terminal summary:

```
=== Zwift Stats Summary ===

Lifetime  (10 rides · 2026-05-06 – 2026-06-01)
  Total XP:         14,186   avg  1,419/ride  ·  1,308/hr avg
  Total Drops:     379,027   avg 37,903/ride  ·  34,955/hr avg
  Total time:   10h 50m

Personal Bests
  XP/hr           2,482  ("Zwift - Watopia" — 2026-06-01)
  ...
```

`zwift-stats chart` generates a self-contained HTML file and opens it in your browser with four charts:

- **Cumulative XP over time** — tracks your total XP milestone progress
- **Cumulative Drops over time** — tracks your Drops wealth
- **XP per ride** — bar chart of your most recent 50 rides
- **XP/hr efficiency** — your intensity trend over time

## Personal bests

Personal bests are tracked globally across all rides in a local SQLite database at `~/.local/share/zwift-stats/bests.db`. Four metrics are tracked:

| Metric | Description |
|---|---|
| `xp_per_hour` | Best XP rate — favours intense, focused efforts |
| `drops_per_hour` | Best Drops rate |
| `total_xp` | Most XP earned in a single ride — favours long rides |
| `total_drops` | Most Drops earned in a single ride |

A 🏆 appears inline next to any value that sets a new personal best:

```
XP: 🏆 2,482  (🏆 1,338/hr)   ← both total and rate are new records
XP: 2,482  (🏆 1,338/hr)       ← rate PR only
XP: 🏆 2,482  (1,338/hr)       ← total PR only
XP: 2,482  (1,338/hr)          ← neither
```

## Duplicate activity handling

If Strava has multiple Virtual Ride activities in the same time window (e.g. a duplicate from a different recording device), the tool identifies the Zwift-uploaded activity by its `external_id` field (`zwift-activity-...`), which Strava sets when Zwift uploads the file directly.

## Long-term maintenance

This tool is designed to be set-and-forget:

- **Strava tokens** auto-refresh silently. If you ever revoke access in Strava settings, re-run `zwift-stats auth`.
- **Zwift updates** occasionally change log formatting. If descriptions stop updating, run `zwift-stats process <file> --dry-run` to diagnose. The relevant patterns are in `src/log-parser.ts`.
- **After moving the project** to a different directory, re-run `zwift-stats daemon install` to regenerate the plist with updated paths.

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
  backfill.ts     Scans all Zwift log files and processes historical rides
  summary.ts      Terminal summary statistics
  chart.ts        Generates browser-based XP/Drops charts
```
