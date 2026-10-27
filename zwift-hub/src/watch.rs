//! Watches Zwift's Logs/ and Activities/ folders and re-imports once they settle. Log.txt is
//! written continuously while Zwift runs, so changes are batched: an import runs after a quiet
//! spell, or at the latest a few minutes after the first change, which keeps a ride's XP current
//! while it is in progress without importing on every line.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::import::Paths;

const QUIET: Duration = Duration::from_secs(15);
const MAX_WAIT: Duration = Duration::from_secs(180);

fn relevant(event: &Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        (name.starts_with("Log") && name.ends_with(".txt"))
            || (name.ends_with(".fit") && name != "inProgressActivity.fit")
    })
}

/// Starts watching and calls `on_change` from a background thread after each batch of changes.
/// The returned watcher must be kept alive for as long as watching should continue.
pub fn spawn(paths: &Paths, on_change: impl Fn() + Send + 'static) -> Result<RecommendedWatcher> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| match res {
        Ok(event) if relevant(&event) => {
            let _ = tx.send(());
        }
        Ok(_) => {}
        Err(e) => eprintln!("watch error: {e}"),
    })?;

    let mut watching = Vec::new();
    for dir in [paths.zwift_dir.join("Logs"), paths.zwift_dir.join("Activities")] {
        match watch_dir(&mut watcher, &dir) {
            Ok(()) => watching.push(dir.display().to_string()),
            Err(e) => eprintln!("not watching {}: {e}", dir.display()),
        }
    }
    anyhow::ensure!(!watching.is_empty(), "no Zwift folders to watch in {}", paths.zwift_dir.display());
    println!("Watching {} for new logs and rides", watching.join(" and "));

    std::thread::spawn(move || {
        // Blocks until the first change of a batch; ends when the watcher is dropped
        while rx.recv().is_ok() {
            let first = Instant::now();
            loop {
                let left = MAX_WAIT.saturating_sub(first.elapsed());
                match rx.recv_timeout(QUIET.min(left)) {
                    Ok(()) if !left.is_zero() => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    _ => break,
                }
            }
            on_change();
        }
    });
    Ok(watcher)
}

fn watch_dir(watcher: &mut RecommendedWatcher, dir: &Path) -> notify::Result<()> {
    watcher.watch(dir, RecursiveMode::NonRecursive)
}
