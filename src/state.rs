use crate::email::EmailSender;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::middleware::rate_limit::IpRateLimiter;

#[derive(Clone)]
// src/state.rs — add this field
pub struct AppState {
    pub db: PgPool,
    pub sender: Arc<dyn EmailSender>,
    pub job_tx: mpsc::Sender<SendJob>,
    pub max_attempts: i16,
    pub resend_webhook_secret: crate::config::Secret,
    pub ip_limiter: Arc<IpRateLimiter>,
}

#[derive(Debug, Clone)]
pub struct SendJob {
    pub otp_id: uuid::Uuid,
    pub email: String,
    pub code: String,
}