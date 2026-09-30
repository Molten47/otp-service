// src/otp/store.rs
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::net::IpAddr;
use uuid::Uuid;

pub struct NewOtp<'a> {
    pub email: &'a str,
    pub code_hash: &'a str,
    pub expires_at: DateTime<Utc>,
    pub requester_ip: IpAddr,
}

/// Enforces per-email and global rate limits, and inserts the row, in one
/// round trip. Returns `None` if a limit was hit — caller maps that to 429.
pub async fn insert_with_rate_limit(
    pool: &PgPool,
    otp: NewOtp<'_>,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        with recent_for_email as (
            select count(*) as cnt from otps
            where email = $1 and created_at > now() - interval '1 hour'
        ),
        recent_global as (
            select count(*) as cnt from otps
            where created_at > now() - interval '1 day'
        )
    
        insert into otps (email, code_hash, expires_at, requester_ip)
        select $1, $2, $3, $4::inet
        where (select cnt from recent_for_email) < 3
          and (select cnt from recent_global) < 90
        returning id
        "#,
        otp.email,
        otp.code_hash,
        otp.expires_at,
        otp.requester_ip.to_string() as _,
    )
    .fetch_optional(pool)
    .await
}

pub async fn mark_sent(pool: &PgPool, otp_id: Uuid, provider_message_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update otps set provider_message_id = $1, sent_at = now() where id = $2",
        provider_message_id,
        otp_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_send_error(pool: &PgPool, otp_id: Uuid, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "update otps set send_error = $1 where id = $2",
        error,
        otp_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}