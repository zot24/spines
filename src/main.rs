use std::net::SocketAddr;

use spines::http::AppState;
use spines::{open, scan, Config, Error, Result};

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("spines: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("mcp") => {
            let cfg = Config::from_env()?;
            let store = spines::Store::open_readonly(&cfg.data_dir)?;
            spines::mcp::stdio(&store)
        }
        Some("openapi") => {
            print!("{}", spines::http::openapi_json());
            Ok(())
        }
        Some(other) => Err(Error::msg(format!("unknown command {other}"))),
        None => serve().await,
    }
}

async fn serve() -> Result<()> {
    let cfg = Config::from_env()?;
    let store = open(&cfg)?;
    let force = std::env::var("SPINES_FORCE_RESCAN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let reports = scan(&store, &cfg, force)?;
    let bind = cfg.bind;
    let app = spines::http::router(AppState::new(store, cfg, reports));
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(Error::from)?;
    let addr: SocketAddr = listener.local_addr().map_err(Error::from)?;
    eprintln!("listen {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(Error::msg)?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(term) => term,
                Err(_) => {
                    let _ = ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
