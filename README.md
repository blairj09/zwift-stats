# zwift-stats

> **macOS only.** This tool relies on the macOS LaunchAgent system for background execution and expects Zwift's default file paths on macOS (`~/Documents/Zwift/`). It will not work on Windows or Linux.

Automatically appends Zwift XP and Drops stats to your Strava activity descriptions after every ride, tracks personal bests, and builds a local dashboard of XP and Drops per ride, per week and per route, including XP/hour, XP/km, Drops/hour and Drops/km.

```
--- Zwift Stats ---
XP: 🏆 2,482  (🏆 1,338/hr · 49/km)
Drops: 🏆 52,385  (🏆 28,236/hr · 1,040/km)
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
zwift-stats refresh                     # Fill distance/climbing/power for stored rides from FIT files, rebuild bests
zwift-stats refresh --names             # Also fetch ride names from Strava (read-only)
zwift-stats stats                       # Print lifetime, weekly and monthly XP/Drops summary
zwift-stats dashboard                   # Generate the dashboard and open it in your browser
zwift-stats dashboard --output <file>   # Write dashboard HTML to a specific path (alias: chart)
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

## Upgrading from an earlier version

Earlier versions stored only XP, Drops and duration. After upgrading, run once:

```bash
zwift-stats refresh --names
```

This re-reads each stored ride's `.fit` file to add its real start date, distance, climbing, average power and heart rate, fetches ride names from Strava for the dashboard's per-route table (read-only, at most 80 per run to stay inside Strava's rate limit; run it again later if it says so), and rebuilds personal bests from your ride history. It never edits Strava activities. New rides get all of this automatically.

## Stats and dashboard

`zwift-stats stats` prints a terminal summary: lifetime totals with per-hour and per-km rates, personal bests, the last 30 days, the last 8 weeks (rides, km, XP, XP/hr, XP/km, Drops, Drops/hr, Drops/km) and the last 6 months.

`zwift-stats dashboard` writes a self-contained HTML page to `~/.local/share/zwift-stats/dashboard.html` and opens it. It has:

- **Range and name filters**: last 4 weeks to all time, plus a text filter (e.g. `Alpe` or `Makuri`) that applies to everything below
- **Totals**: XP, Drops, XP/hr, XP/km, Drops/hr, Drops/km, distance and time for the range, each with this week and last week
- **Per week**: any of those metrics as weekly bars, so you can see a given week at a glance
- **Per ride**: any rate per ride with a 5-ride rolling average
- **Cumulative XP and Drops** for the range
- **By ride name**: every route you've ridden with its average rates, sortable, to find which rides pay best for XP or Drops
- **Rides**: a sortable table of every ride with distance, time, climbing, power and all rates, linked to Strava

Rates for a week or route are totals divided by total time or distance, so a two-hour ride weighs more than a twenty-minute one. Per-km rates only use rides that have a distance.

## Personal bests

Personal bests are tracked globally across all rides in a local SQLite database at `~/.local/share/zwift-stats/bests.db`. Six metrics are tracked:

| Metric | Description |
|---|---|
| `xp_per_hour` | Best XP rate — favours intense, focused efforts |
| `drops_per_hour` | Best Drops rate |
| `xp_per_km` | Best XP per km (rides of 5 km or more) |
| `drops_per_km` | Best Drops per km (rides of 5 km or more) — favours climbing |
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
  fit-parser.ts   Extracts start time, duration, distance, climbing, power and HR from .fit files
  pipeline.ts     Orchestrates one ride end-to-end
  strava.ts       Strava API client (token refresh, activity match, update)
  bests.ts        SQLite personal bests store
  format.ts       Builds the description block string
  daemon.ts       Installs/uninstalls the macOS LaunchAgent
  backfill.ts     Scans all Zwift log files and processes historical rides
  refresh.ts      Re-reads FIT files (and Strava names) for stored rides, rebuilds bests
  summary.ts      Terminal summary statistics
  chart.ts        Writes the dashboard from the rides table
  dashboard.html  Dashboard page template
```
