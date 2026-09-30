use axum::{routing::get, Router};
use otp_service::{config::Config, email::resend::ResendSender, state::{AppState, SendJob}};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tokio::sync::mpsc;




#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok(); // fine if missing on Render, where env vars are set directly
    tracing_subscriber::fmt::init();

    let config = Config::from_env()?;


    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(config.database_url.expose())
        .await?;

    sqlx::migrate!().run(&db).await?;

    let http_client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()?;

    let sender = Arc::new(ResendSender::new(
        http_client,
        config.resend_api_key.expose().to_string(),
        config.from_address.clone(),
    ));

    let (job_tx, job_rx) = mpsc::channel::<SendJob>(config.queue_capacity);
    let ip_limiter = otp_service::middleware::rate_limit::build_limiter(config.per_ip_limit_per_min);
    let state = AppState {
        db: db.clone(),
        sender: sender.clone(),
        job_tx: job_tx.clone(),
        max_attempts: config.max_attempts,
        resend_webhook_secret: config.resend_webhook_secret.clone(),
        ip_limiter,
    };


   
    let worker_handles = otp_service::worker::spawn_pool(config.worker_count, job_rx, db.clone(), sender.clone());
   

    let app = Router::new()
    .route("/health", get(|| async { "ok" }))
    .merge(otp_service::routes::otp::router(state.clone()))
    .merge(otp_service::routes::webhooks::router())
    .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

// At this point, Axum has stopped accepting new connections and all
// in-flight HTTP requests have finished. Now drain the worker pool:
// drop our own sender handle so the channel can actually close once
// every clone held by in-flight handlers is also dropped.
drop(job_tx);

tracing::info!("waiting for {} worker(s) to finish in-flight sends", worker_handles.len());
for handle in worker_handles {
    let _ = handle.await;
}

tracing::info!("shutdown complete");

Ok(())
    
}


async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, draining in-flight work");
}