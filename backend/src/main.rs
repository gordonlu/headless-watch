use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use headless_watch::cdp::probe_tabs;
use headless_watch::core::{resolve_web_dir, Core, PssResult};
use headless_watch::model::VERSION;
use headless_watch::procfs::read_pss;
use headless_watch::server::{broadcast_state, build_app, read_index_template, AppState};
use headless_watch::settings::Settings;
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "headless-watch", version = VERSION)]
struct Cli {
    #[arg(long, default_value_t = 8765, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
    #[arg(long)]
    interval: Option<f64>,
    #[arg(long)]
    cpu_alert_cores: Option<f64>,
    #[arg(long)]
    cpu_alert_duration: Option<f64>,
    #[arg(long)]
    leftover_after: Option<f64>,
    #[arg(long)]
    pss_interval: Option<f64>,
    #[arg(long)]
    cdp_interval: Option<f64>,
    #[arg(long)]
    no_notify: bool,
    #[arg(long)]
    verbose: bool,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    web_dir: Option<PathBuf>,
}

fn default_data_dir() -> Result<PathBuf> {
    if let Ok(value) = std::env::var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(value).join("headless-watch"));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/share/headless-watch"))
}

fn apply_cli_settings(cli: &Cli, settings: &mut Settings) {
    if let Some(value) = cli.interval {
        settings.interval_s = value;
    }
    if let Some(value) = cli.cpu_alert_cores {
        settings.cpu_alert_cores = value;
    }
    if let Some(value) = cli.cpu_alert_duration {
        settings.cpu_alert_duration_s = value;
    }
    if let Some(value) = cli.leftover_after {
        settings.leftover_after_s = value;
    }
    if let Some(value) = cli.pss_interval {
        settings.pss_interval_s = value;
    }
    if let Some(value) = cli.cdp_interval {
        settings.cdp_interval_s = value;
    }
    if cli.no_notify {
        settings.notifications = false;
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(if cli.verbose {
                "info,headless_watch=debug"
            } else {
                "warn"
            })
        }))
        .with_target(false)
        .init();

    let data_dir = cli.data_dir.clone().unwrap_or(default_data_dir()?);
    let web_dir = resolve_web_dir(cli.web_dir.clone())?;
    let index_template = Arc::<str>::from(read_index_template(&web_dir)?);
    let mut core = Core::open(&data_dir, web_dir.clone(), None, None)?;
    apply_cli_settings(&cli, &mut core.settings);
    core.settings = core.settings.clone().normalized();
    core.settings.validate().map_err(anyhow::Error::msg)?;
    // Apply CLI overrides before the first scan so --no-notify is effective
    // from the very first discovery.
    let initial_notices = core.initialize();
    let token = Arc::<str>::from(core.token.clone());
    let port = cli.port;
    let (tx, _) = broadcast::channel::<String>(16);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let state = AppState {
        core: Arc::new(Mutex::new(core)),
        tx: tx.clone(),
        shutdown: shutdown_rx,
        port,
        token,
        index_template,
        web_dir,
    };

    for notify in initial_notices {
        headless_watch::notify::send(&notify.title, &notify.body);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let scanner = {
        let state = state.clone();
        let stop = stop.clone();
        thread::spawn(move || scanner_loop(state, stop))
    };
    let side = {
        let state = state.clone();
        let stop = stop.clone();
        thread::spawn(move || side_loop(state, stop))
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot create async runtime")?;
    let result = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .with_context(|| {
                format!("cannot bind 127.0.0.1:{port}; stop the other process or choose --port")
            })?;
        println!("Headless Watch {VERSION}: http://127.0.0.1:{port}");
        println!(
            "Local-only · SQLite WAL · default scan {}s · Press Ctrl+C to exit",
            state
                .core
                .lock()
                .map(|core| core.settings.interval_s)
                .unwrap_or(1.0)
        );
        let app = build_app(state.clone());
        let serve_shutdown = shutdown_tx.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_signal().await;
                let _ = serve_shutdown.send(true);
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    eprintln!("Headless Watch: forcing exit after shutdown timeout.");
                    std::process::exit(0);
                });
            })
            .await
            .context("HTTP server failed")
    });
    stop.store(true, Ordering::SeqCst);
    let _ = scanner.join();
    let _ = side.join();
    result
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    eprintln!("\nHeadless Watch shutting down.");
}

fn publish(state: &AppState, notices: Vec<headless_watch::tracker::Notify>) {
    broadcast_state(state);
    for notify in notices {
        headless_watch::notify::send(&notify.title, &notify.body);
    }
}

fn scanner_loop(state: AppState, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let (notices, interval) = {
            let Ok(core) = state.core.lock() else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };
            let mut core = core;
            let notices = core.scan_once();
            (notices, core.settings.interval_s)
        };
        publish(&state, notices);
        let mut remaining = Duration::from_secs_f64(interval.max(0.5));
        while !stop.load(Ordering::Relaxed) && remaining > Duration::ZERO {
            let step = remaining.min(Duration::from_millis(250));
            thread::sleep(step);
            remaining = remaining.saturating_sub(step);
        }
    }
}

fn side_loop(state: AppState, stop: Arc<AtomicBool>) {
    let mut next_pss = Instant::now();
    let mut next_cdp = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= next_pss {
            let (targets, pss_interval) = {
                let Ok(core) = state.core.lock() else {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                };
                (core.pss_targets(), core.settings.pss_interval_s)
            };
            let mut results = Vec::new();
            for (session_id, processes) in targets {
                let mut values = Vec::new();
                let mut errors = Vec::new();
                for (pid, _start_ticks) in processes.iter().copied() {
                    match read_pss(pid, std::path::Path::new("/proc")) {
                        Ok(value) => values.push((pid, value)),
                        Err(err) => errors.push(format!("{pid}: {err}")),
                    }
                }
                results.push(PssResult {
                    session_id,
                    values,
                    errors,
                    sampled_at: headless_watch::util::now_wall(),
                });
            }
            if !results.is_empty() {
                let Ok(mut core) = state.core.lock() else {
                    continue;
                };
                core.apply_pss_results(results);
            }
            if !stop.load(Ordering::Relaxed) {
                publish(&state, Vec::new());
            }
            next_pss = Instant::now() + Duration::from_secs_f64(pss_interval.max(5.0));
        }
        if Instant::now() >= next_cdp {
            let (targets, cdp_interval) = {
                let Ok(core) = state.core.lock() else {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                };
                (core.cdp_targets(), core.settings.cdp_interval_s)
            };
            let mut results = Vec::new();
            for (session_id, port) in targets {
                let probe = probe_tabs(port, Duration::from_millis(350));
                results.push((session_id, port, probe, headless_watch::util::now_wall()));
            }
            if !results.is_empty() {
                let Ok(mut core) = state.core.lock() else {
                    continue;
                };
                core.apply_cdp_results(results);
            }
            if !stop.load(Ordering::Relaxed) {
                publish(&state, Vec::new());
            }
            next_cdp = Instant::now() + Duration::from_secs_f64(cdp_interval.max(2.0));
        }
        thread::sleep(Duration::from_millis(250));
    }
}
