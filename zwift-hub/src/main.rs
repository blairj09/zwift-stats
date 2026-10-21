mod companion;
mod db;
mod fit;
mod import;
mod logparse;
mod server;
mod service;
mod watch;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use import::Paths;

#[derive(Parser)]
#[command(name = "zwift-hub", about = "Import your Zwift logs and rides into DuckDB and explore them locally")]
struct Cli {
    /// DuckDB database file
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    /// Zwift's documents folder (contains Logs/ and Activities/)
    #[arg(long, global = true)]
    zwift_dir: Option<PathBuf>,
    /// Database from the earlier zwift-stats tool, imported for rides whose logs are gone
    #[arg(long, global = true)]
    legacy_db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Archive new game logs and import logs, FIT files and legacy rides
    Import,
    /// Import, then start the explorer at http://127.0.0.1:<port> and keep importing as Zwift
    /// writes new logs and rides
    Serve {
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// Skip the import on start
        #[arg(long)]
        no_import: bool,
        /// Don't watch the Zwift folders for new logs and rides
        #[arg(long)]
        no_watch: bool,
        /// Open the explorer in the default browser
        #[arg(long)]
        open: bool,
    },
    /// Import, then keep importing as Zwift writes new logs and rides, without the explorer
    Watch,
    /// Run the explorer and watcher in the background at login (macOS)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Sign in to Zwift for Companion data (profile, activities, followers, events). Saves a
    /// refresh token, never your password
    Login,
    /// Forget the saved Zwift sign-in
    Logout,
    /// Fetch Companion data from Zwift now (`serve` also does this every 30 minutes)
    Sync,
    /// Run a read-only SQL query against the database and print the result
    Sql { query: String },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Install and start the background service
    Install {
        #[arg(long, default_value_t = 7878)]
        port: u16,
    },
    /// Stop and remove the background service
    Uninstall,
    /// Show whether the background service is running
    Status,
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("zwift-hub")
}

pub fn print_report(r: &import::ImportReport) {
    println!(
        "Imported: {} new log(s) archived ({} total), {} activities with XP, {} new FIT file(s) ({} total, {} unreadable), {} legacy ride(s)",
        r.logs_archived, r.logs_total, r.activities, r.fit_new, r.fit_total, r.fit_failed, r.legacy_rides
    );
}

pub fn print_sync(r: &companion::SyncReport) {
    println!(
        "Synced from Zwift: profile, {} activities, {} followers, {} following, {} upcoming events",
        r.activities, r.followers, r.followees, r.events
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = data_dir();
    let paths = Paths {
        zwift_dir: cli
            .zwift_dir
            .or_else(|| dirs::document_dir().map(|d| d.join("Zwift")))
            .context("could not find the Zwift documents folder; pass --zwift-dir")?,
        legacy_db: cli
            .legacy_db
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share/zwift-stats/bests.db"))),
        data_dir: data_dir.clone(),
    };
    let db_path = cli.db.unwrap_or_else(|| data_dir.join("zwift.duckdb"));

    match cli.command {
        Command::Login => return companion::login(&data_dir).await,
        Command::Logout => return companion::logout(&data_dir),
        _ => {}
    }

    if let Command::Service { action } = cli.command {
        return match action {
            ServiceAction::Install { port } => {
                let mut args = vec!["serve".to_string(), "--port".into(), port.to_string()];
                args.extend(["--zwift-dir".into(), paths.zwift_dir.display().to_string()]);
                args.extend(["--db".into(), db_path.display().to_string()]);
                if let Some(legacy) = &paths.legacy_db {
                    args.extend(["--legacy-db".into(), legacy.display().to_string()]);
                }
                service::install(args, &data_dir)
            }
            ServiceAction::Uninstall => service::uninstall(),
            ServiceAction::Status => service::status(),
        };
    }

    // DuckDB allows one process per database file, so a running `serve` holds it
    let conn = db::open(&db_path).with_context(|| {
        format!(
            "opening {} (if Zwift Hub is already running, use its Explore tab at http://127.0.0.1:7878)",
            db_path.display()
        )
    })?;

    match cli.command {
        Command::Import => print_report(&import::run(&conn, &paths)?),
        Command::Serve { port, no_import, no_watch, open } => {
            if open {
                let url = format!("http://127.0.0.1:{port}");
                let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                let _ = std::process::Command::new(opener).arg(&url).spawn();
            }
            server::serve(conn, paths, port, !no_import, !no_watch).await?;
        }
        Command::Watch => {
            print_report(&import::run(&conn, &paths)?);
            let conn = std::sync::Mutex::new(conn);
            let p = paths.clone();
            let _watcher = watch::spawn(&paths, move || match import::run(&conn.lock().unwrap(), &p) {
                Ok(r) => print_report(&r),
                Err(e) => eprintln!("import failed: {e:#}"),
            })?;
            tokio::signal::ctrl_c().await?;
        }
        Command::Sync => {
            let fetched = companion::fetch(&data_dir).await?;
            print_sync(&companion::store(&conn, &fetched)?);
        }
        Command::Service { .. } | Command::Login | Command::Logout => unreachable!(),
        Command::Sql { query } => {
            anyhow::ensure!(server::is_read_only(&query), "only a single read-only query is allowed");
            let res = db::query_json(&conn, query.trim().trim_end_matches(';'), 200)?;
            println!("{}", res.columns.join("\t"));
            for row in res.rows {
                let cells: Vec<String> = row
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    })
                    .collect();
                println!("{}", cells.join("\t"));
            }
        }
    }
    Ok(())
}
