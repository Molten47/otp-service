
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use serde::Deserialize;
use svix::webhooks::Webhook;

pub fn router() -> Router<AppState> {
    Router::new().route("/webhooks/resend", post(handle_webhook))
}

#[derive(Deserialize)]
struct ResendEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: ResendEventData,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
struct ResendEventData {
    email_id: String,
}

async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    let wh = Webhook::new(state.resend_webhook_secret.expose())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    wh.verify(&body, &headers)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;


    wh.verify(&body, &headers)
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // 2. Only now, having proven the body is authentic, parse it.
    let event: ResendEvent =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

   let event_id = headers
    .get("svix-id")
    .and_then(|v| v.to_str().ok())
    .ok_or(StatusCode::BAD_REQUEST)?;

    let payload_json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    // 3. Idempotent insert: the unique index on provider_event_id makes
    //    a retried delivery a harmless no-op instead of a duplicate row.
    let result = sqlx::query!(
        r#"
        insert into email_delivery_events
            (provider_event_id, otp_id, event_type, payload, occurred_at)
        select $1,
               (select id from otps where provider_message_id = $2),
               $3, $4, $5
        on conflict (provider_event_id) do nothing
        "#,
        event_id,
        event.data.email_id,
        event.event_type,
        payload_json,
        event.created_at,
    )
    .execute(&state.db)
    .await;

    if let Err(e) = result {
        tracing::error!(error = %e, "failed to record webhook event");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    tracing::info!(event_type = %event.event_type, email_id = %event.data.email_id, "webhook processed");

    Ok(StatusCode::OK)
}

