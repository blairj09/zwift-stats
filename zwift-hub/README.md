# zwift-hub

A single Rust binary that imports everything Zwift leaves on your Mac into a local [DuckDB](https://duckdb.org) database and gives you a browser-based explorer for it: XP and Drops per ride, per week and per route, with per-hour and per-km rates, where your XP comes from, your level and Drops balance over time, and a SQL console.

Nothing leaves your machine. The server listens on `127.0.0.1` only.

## Quick start

```bash
cd zwift-hub
cargo build --release          # first build compiles DuckDB, which takes a few minutes
./target/release/zwift-hub serve --open
```

`serve` imports first, opens http://127.0.0.1:7878, and then watches your Zwift folders: new rides and log lines are imported automatically (within about 15 seconds of Zwift going quiet, and every few minutes during a ride), and an open explorer page refreshes itself.

## Run it in the background

```bash
./target/release/zwift-hub service install     # starts now and at every login
./target/release/zwift-hub service status
./target/release/zwift-hub service uninstall
```

This installs a LaunchAgent (`~/Library/LaunchAgents/com.github.blairj09.zwift-hub.plist`) that runs `serve`, so the database stays current and the explorer is always at http://127.0.0.1:7878. Its output goes to `~/Library/Application Support/zwift-hub/zwift-hub.log`. Re-run `install` after rebuilding to pick up the new binary.

macOS protects `~/Documents`. If the log shows "Operation not permitted" for the Zwift folder, add the `zwift-hub` binary under System Settings > Privacy & Security > Full Disk Access (or Files and Folders), then run `service install` again.

DuckDB lets one process open the database at a time, so while the service runs, `zwift-hub import` and `zwift-hub sql` will report the file is in use. Use the explorer's Explore tab instead, or `service uninstall` first.

## Companion data (optional)

```bash
./target/release/zwift-hub login     # asks for your Zwift email and password once
./target/release/zwift-hub logout    # forgets the sign-in
```

Signing in adds a **Companion** tab: your profile (total XP, Drops balance, FTP, lifetime distance), who you follow that is riding right now, Ride Ons on your recent rides, and upcoming events. `serve` (and the background service) refreshes it every 30 minutes, or press **Sync now**; `zwift-hub sync` does the same when nothing else is running.

`login` sends your password to Zwift's sign-in server once and saves only the refresh token it gets back, in `~/Library/Application Support/zwift-hub/zwift-token.json`, readable only by you. This is Zwift's unofficial API (the one the Companion app uses), so it is undocumented and can change or break without notice. The data lands in `zwift_profile`, `zwift_activities`, `zwift_follows` and `zwift_events`, each with a `raw` JSON column.

## Where the data comes from

| Source | What it gives | How far back |
|---|---|---|
| Game log `~/Documents/Zwift/Logs/Log*.txt` | Per ride: XP, Drops, route, world, ride type, event, level and totals before and after, streak, Ride Ons, and every XP grant with its source (distance, workout, route badge, power-ups, …) | Zwift keeps only its last 10 logs |
| FIT files `~/Documents/Zwift/Activities/*.fit` | Distance, time, climbing, calories, average/max/normalized power, heart rate, cadence | Every ride you've saved |
| Earlier `zwift-stats` database `~/.local/share/zwift-stats/bests.db` | XP and Drops for rides it processed | Since you installed it |

**Zwift deletes old logs**, so `import` copies each log into an archive (`~/Library/Application Support/zwift-hub/logs/`, one file per game session, refreshed as the live log grows) before parsing it. From the first import on, no ride's XP is lost to rotation. Rides older than your oldest log still show their FIT metrics, and their XP if the earlier `zwift-stats` tool recorded it.

## Commands

```bash
zwift-hub import                           # archive new logs, import logs, FIT files and legacy rides
zwift-hub serve [--port 7878] [--open]     # import, start the explorer, and keep importing
zwift-hub serve --no-import --no-watch     # start without importing or watching
zwift-hub watch                            # keep importing, without the explorer
zwift-hub service install|status|uninstall # run serve in the background at login (macOS)
zwift-hub login | logout | sync            # Companion data from Zwift's API
zwift-hub sql "SELECT route, sum(xp) FROM rides GROUP BY ALL ORDER BY 2 DESC"
```

Global options: `--db <file>` (default `~/Library/Application Support/zwift-hub/zwift.duckdb`), `--zwift-dir <dir>` (default `~/Documents/Zwift`), `--legacy-db <file>`.

## The explorer

- **Overview**: totals and rates for the selected range with this week and last week, any metric per week, any rate per ride with a rolling average, and XP by source
- **Rides**: every ride, sortable; click one for its details, including its XP breakdown
- **Routes**: rates by route and by ride type, to see which rides pay best for XP or Drops
- **Progress**: total XP, level and Drops balance over time, and cumulative earnings
- **Companion**: profile, friends riding now, Ride Ons and upcoming events (after `zwift-hub login`)
- **Explore**: read-only SQL against the database, with example queries and a quick chart of the result

## Tables

- `rides` (view): one row per ride across all sources, with `xp_per_hour`, `xp_per_km`, `drops_per_hour`, `drops_per_km`, `drops_per_m_climbed` and `earnings_source` (`log`, `legacy` or `fit` when no XP is known)
- `activities`: one row per activity parsed from the game log
- `xp_sources`: XP per activity per grant type
- `fit_files`: session summary of every FIT file
- `legacy_rides`: rows imported from the earlier tool
- `log_files`: archived logs
- `zwift_profile`, `zwift_activities`, `zwift_follows`, `zwift_events`: Companion data from Zwift's API

## Development

```bash
cargo test
cargo run -- serve --zwift-dir /path/to/Zwift --db /tmp/zwift.duckdb
```

`assets/` is embedded into the binary at build time (`index.html` and a vendored copy of Chart.js 4.4.1, MIT).
