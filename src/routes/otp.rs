
use crate::{
    otp::{
        generate::{generate_code, hash_code},
        store::{self, NewOtp},
    },
    state::{AppState, SendJob},
};
use axum::{
    extract::{ConnectInfo, State},
    routing::post,
    Json, Router,
    middleware
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use crate::
    {otp::verify::{verify, VerifyError},
    middleware::rate_limit::per_ip_rate_limit};

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/otp",
            post(request_otp).layer(middleware::from_fn_with_state(
                state.ip_limiter.clone(),
                per_ip_rate_limit,
            )),
        )
        .route("/otp/verify", post(verify_otp))
}


#[derive(Deserialize)]
pub struct RequestOtp {
    email: String,
}

#[derive(Serialize)]
pub struct RequestOtpResponse {
    status: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum OtpError {
    #[error("invalid email address")]
    InvalidEmail,
    #[error("too many requests, try again later")]
    RateLimited,
    #[error("invalid or expired code")]
    VerificationFailed,
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl axum::response::IntoResponse for OtpError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let status = match &self {
            OtpError::InvalidEmail => StatusCode::BAD_REQUEST,
            OtpError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            OtpError::VerificationFailed => StatusCode::BAD_REQUEST,
            OtpError::Internal(e) => {
                tracing::error!(error = ?e, "internal error handling otp request");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, self.to_string()).into_response()
    }
}

async fn request_otp(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<RequestOtp>,
) -> Result<Json<RequestOtpResponse>, OtpError> {
    let email = payload.email.trim().to_lowercase();

    if !email_address::EmailAddress::is_valid(&email) {
        return Err(OtpError::InvalidEmail);
    }

    let code = generate_code();
    let code_for_hash = code.clone();
    let hash = tokio::task::spawn_blocking(move || hash_code(&code_for_hash))
        .await
        .map_err(|e| anyhow::anyhow!("hash task panicked: {e}"))?
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let expires_at = Utc::now() + Duration::minutes(5);

    let otp_id = store::insert_with_rate_limit(
        &state.db,
        NewOtp {
            email: &email,
            code_hash: &hash,
            expires_at,
            requester_ip: addr.ip(),
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))?
    .ok_or(OtpError::RateLimited)?;

    state
        .job_tx
        .send(SendJob { otp_id, email, code })
        .await
        .map_err(|_| anyhow::anyhow!("worker channel closed"))?;

    Ok(Json(RequestOtpResponse { status: "queued" }))
}

#[derive(Deserialize)]
pub struct VerifyOtp {
    email: String,
    code: String,
}

#[derive(Serialize)]
pub struct VerifyOtpResponse {
    status: &'static str,
}

async fn verify_otp(
    State(state): State<AppState>,
    Json(payload): Json<VerifyOtp>,
) -> Result<Json<VerifyOtpResponse>, OtpError> {
    let email = payload.email.trim().to_lowercase();
    let code = payload.code.trim();

    match verify(&state.db, &email, code, state.max_attempts).await {
        Ok(()) => Ok(Json(VerifyOtpResponse { status: "verified" })),
        Err(VerifyError::NotFound)
        | Err(VerifyError::Expired)
        | Err(VerifyError::Incorrect) => Err(OtpError::VerificationFailed),
        Err(VerifyError::TooManyAttempts) => Err(OtpError::RateLimited),
        Err(e) => Err(OtpError::Internal(anyhow::anyhow!(e))),
    }
}