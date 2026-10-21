mod db;
mod fit;
mod import;
mod logparse;
mod server;

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
    /// Import, then start the explorer at http://127.0.0.1:<port>
    Serve {
        #[arg(long, default_value_t = 7878)]
        port: u16,
        /// Skip the import on start
        #[arg(long)]
        no_import: bool,
        /// Open the explorer in the default browser
        #[arg(long)]
        open: bool,
    },
    /// Run a read-only SQL query against the database and print the result
    Sql { query: String },
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("zwift-hub")
}

fn print_report(r: &import::ImportReport) {
    println!(
        "Imported: {} new log(s) archived ({} total), {} activities with XP, {} new FIT file(s) ({} total, {} unreadable), {} legacy ride(s)",
        r.logs_archived, r.logs_total, r.activities, r.fit_new, r.fit_total, r.fit_failed, r.legacy_rides
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
    let conn = db::open(&db_path).with_context(|| format!("opening {}", db_path.display()))?;

    match cli.command {
        Command::Import => print_report(&import::run(&conn, &paths)?),
        Command::Serve { port, no_import, open } => {
            if !no_import {
                print_report(&import::run(&conn, &paths)?);
            }
            if open {
                let url = format!("http://127.0.0.1:{port}");
                let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
                let _ = std::process::Command::new(opener).arg(&url).spawn();
            }
            server::serve(conn, paths, port).await?;
        }
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
