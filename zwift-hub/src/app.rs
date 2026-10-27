//! The Mac app: a native window (WebKit, the engine Safari uses) showing the explorer. Pages and
//! API calls go through the `zwifthub://` scheme straight to the same routes the web server
//! uses, so the app opens no network port at all.

use std::borrow::Cow;
use std::path::Path;

use anyhow::Result;
use muda::{Menu, PredefinedMenuItem, Submenu};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use tower::ServiceExt;
use wry::WebViewBuilder;

use crate::import::Paths;
use crate::server;

fn menu() -> Result<Menu> {
    let menu = Menu::new();
    let app = Submenu::with_items(
        "Zwift Stats",
        true,
        &[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )?;
    // Without these items, ⌘C, ⌘V and ⌘A don't reach the page
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )?;
    let view = Submenu::with_items("View", true, &[&PredefinedMenuItem::fullscreen(None)])?;
    let window = Submenu::with_items(
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::close_window(None)],
    )?;
    menu.append_items(&[&app, &edit, &view, &window])?;
    Ok(menu)
}

fn error_page(message: &str) -> String {
    let message = message.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        "<!doctype html><meta charset=utf-8><body style=\"font: 15px -apple-system, sans-serif; padding: 40px; \
         color: #333\"><h2>Zwift Stats couldn't start</h2><pre style=\"white-space: pre-wrap\">{message}</pre>"
    )
}

/// What the window shows: the app's own routes, an already running Zwift Stats, or an error.
enum Content {
    Routes(server::Running),
    Url(String),
    Html(String),
}

fn already_running(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), std::time::Duration::from_millis(300)).is_ok()
}

pub fn run(db_path: &Path, paths: Paths) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    let content = match crate::db::open(db_path) {
        Ok(conn) => {
            let started = rt.block_on(async {
                let running = server::start(conn, paths, false, true)?;
                running.import_in_background();
                anyhow::Ok(running)
            });
            match started {
                Ok(running) => Content::Routes(running),
                Err(e) => Content::Html(error_page(&format!("{e:#}"))),
            }
        }
        // DuckDB lets one process open the database; if that's `zwift-hub serve`, show it
        Err(_) if already_running(7878) => Content::Url("http://127.0.0.1:7878".into()),
        Err(e) => Content::Html(error_page(&format!("Opening {}: {e:#}", db_path.display()))),
    };

    let event_loop = EventLoop::new();
    let menu = menu()?;
    menu.init_for_nsapp();
    let window = WindowBuilder::new()
        .with_title("Zwift Stats")
        .with_inner_size(LogicalSize::new(1280.0, 860.0))
        .with_min_inner_size(LogicalSize::new(720.0, 520.0))
        .build(&event_loop)?;

    // Links that open a new window (View on Strava) go to the default browser instead
    let builder = WebViewBuilder::new().with_new_window_req_handler(|url, _| {
        if url.starts_with("https://") {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        }
        wry::NewWindowResponse::Deny
    });
    let (builder, running) = match content {
        Content::Routes(running) => {
            let router = running.router.clone();
            let handle = rt.handle().clone();
            let builder = builder
                .with_asynchronous_custom_protocol("zwifthub".into(), move |_id, request, responder| {
                    let router = router.clone();
                    handle.spawn(async move {
                        let response = router.oneshot(request.map(axum::body::Body::from)).await.unwrap_or_else(|e| match e {});
                        let (parts, body) = response.into_parts();
                        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap_or_default();
                        responder.respond(http::Response::from_parts(parts, Cow::Owned(bytes.to_vec())));
                    });
                })
                .with_url("zwifthub://localhost/");
            (builder, Some(running))
        }
        Content::Url(url) => (builder.with_url(url), None),
        Content::Html(html) => (builder.with_html(html), None),
    };
    let webview = builder.build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        // Owned here so the server routes, watcher and runtime live as long as the window
        let _ = (&rt, &running, &webview, &menu);
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    })
}

