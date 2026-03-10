//! Steps 7.1, 7.3, 7.7: Axum static file server, SSE live reload endpoint,
//! and dev command orchestration.
//!
//! The dev server:
//! - Serves files from `dist/` with proper MIME types
//! - Provides `/_reload` SSE endpoint for live reload
//! - Mounts `/_proxy/{source}/*rest` reverse proxy routes for each configured source
//! - Watches the project for changes and triggers rebuilds

use axum::{
    Router,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use eyre::Result;
use futures::stream::Stream;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::services::ServeDir;

use crate::config::SiteConfig;

use super::proxy::{self, ProxyState};
use super::rebuild::DevBuildState;
use super::watcher::{self, RebuildScope};

/// The `eigen dev` command entry point.
///
/// 1. Loads config and performs initial build (with live-reload injection).
/// 2. Starts file watcher in a background thread.
/// 3. Starts rebuild loop in a dedicated background thread (not an async
///    task, since the build uses `reqwest::blocking::Client` which cannot
///    run inside an async runtime).
/// 4. Starts Axum HTTP server.
pub async fn dev_command(project_root: &Path, port: u16) -> Result<()> {
    // Canonicalize the project root.
    let project_root = std::fs::canonicalize(project_root)?;

    // Perform the initial build on a blocking thread so that
    // `reqwest::blocking::Client` is not created inside the async runtime.
    let build_root = project_root.clone();
    let build_state = tokio::task::spawn_blocking(move || -> Result<DevBuildState> {
        tracing::info!("Performing initial build...");
        let state = DevBuildState::new(&build_root)?;
        tracing::info!("Initial build complete.");
        Ok(state)
    })
    .await
    .map_err(|e| eyre::eyre!("Initial build task panicked: {}", e))??;

    // Load config for proxy routes.
    let config = crate::config::load_config(&project_root)?;

    // Broadcast channel for rebuild signals from the watcher.
    let (rebuild_tx, _) = broadcast::channel::<RebuildScope>(16);

    // Broadcast channel for reload signals to SSE clients.
    let (reload_tx, _) = broadcast::channel::<()>(16);

    // Start file watcher in a background thread.
    let watcher_tx = rebuild_tx.clone();
    let watcher_root = project_root.clone();
    std::thread::spawn(move || {
        if let Err(e) = watcher::watch(&watcher_root, watcher_tx) {
            eprintln!("File watcher error: {}", e);
        }
    });

    // Start rebuild loop in a dedicated OS thread.
    //
    // The build pipeline uses `reqwest::blocking::Client` which panics if
    // run inside a Tokio async context.  By using a plain thread with a
    // `std::sync::mpsc` channel we keep all blocking I/O off the async
    // runtime entirely.
    let mut rebuild_rx = rebuild_tx.subscribe();
    let reload_signal = reload_tx.clone();

    // Bridge: async task receives from the broadcast channel and forwards
    // to a std mpsc channel consumed by the rebuild thread.
    let (sync_tx, sync_rx) = std::sync::mpsc::channel::<RebuildScope>();

    tokio::spawn(async move {
        loop {
            match rebuild_rx.recv().await {
                Ok(scope) => {
                    if sync_tx.send(scope).is_err() {
                        break; // rebuild thread gone
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if sync_tx.send(RebuildScope::Full).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    std::thread::spawn(move || {
        let mut build_state = build_state;
        while let Ok(scope) = sync_rx.recv() {
            match build_state.rebuild(scope) {
                Ok(()) => {
                    // Signal all SSE clients to reload.
                    let _ = reload_signal.send(());
                }
                Err(e) => {
                    if e.has_error_page {
                        // A template error page was written to dist/ — reload
                        // the browser so the user sees it instead of stale content.
                        eprintln!("  Waiting for next change...\n");
                        let _ = reload_signal.send(());
                    } else {
                        eprintln!("\n  Build error: {:#}", e.report);
                        eprintln!("  Waiting for next change...\n");
                    }
                }
            }
        }
    });

    // Build Axum router.
    let dist_dir = project_root.join("dist");
    let app = build_router(dist_dir, &config, reload_tx)?;

    // Bind and serve.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("Dev server running at http://127.0.0.1:{}", port);
    eprintln!("Press Ctrl+C to stop.\n");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service())
        .await
        .map_err(|e| eyre::eyre!("Server error: {}", e))?;

    Ok(())
}

/// Build the Axum router with:
/// - `/_reload` SSE endpoint
/// - `/_proxy/{source}/*rest` proxy routes
/// - Static file serving from `dist/` as the fallback
fn build_router(
    dist_dir: PathBuf,
    config: &SiteConfig,
    reload_tx: broadcast::Sender<()>,
) -> Result<Router> {
    let mut app = Router::new();

    // SSE live-reload endpoint.
    let sse_tx = reload_tx.clone();
    app = app.route(
        "/_reload",
        get(move || {
            let rx = sse_tx.subscribe();
            sse_handler(rx)
        }),
    );

    // CMS proxy routes — one per configured source.
    let client = reqwest::Client::new();
    for (name, source) in &config.sources {
        let proxy_state = ProxyState {
            source: source.clone(),
            client: client.clone(),
        };

        let path = format!("/_proxy/{}/{{*rest}}", name);
        app = app.route(
            &path,
            get(proxy::proxy_handler).with_state(proxy_state),
        );

        tracing::info!("  Proxy: /_proxy/{}/* → {}", name, source.url);
    }

    // Static file serving from dist/ as the fallback.
    let serve_dir = ServeDir::new(&dist_dir);
    app = app.fallback_service(serve_dir);

    Ok(app)
}

/// SSE handler that sends a `"reload"` event whenever a rebuild completes.
async fn sse_handler(
    rx: broadcast::Receiver<()>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(()) => Some(Ok(Event::default().event("reload").data(""))),
        Err(_) => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
