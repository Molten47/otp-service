use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::generate::verify_code;

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("no active code for this email")]
    NotFound,
    #[error("code has expired")]
    Expired,
    #[error("too many attempts")]
    TooManyAttempts,
    #[error("incorrect code")]
    Incorrect,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("hashing task panicked: {0}")]
    TaskPanicked(String),
}

struct LiveOtp {
    id: Uuid,
    code_hash: String,
    attempts: i16,
}

/// Verifies a submitted code against the newest live OTP for this email.
/// On success, marks the row consumed. On failure, increments attempts.
pub async fn verify(
    pool: &PgPool,
    email: &str,
    submitted_code: &str,
    max_attempts: i16,
) -> Result<(), VerifyError> {
    let row = sqlx::query_as!(
        LiveOtp,
        r#"
        select id, code_hash, attempts
        from otps
        where email = $1
          and consumed_at is null
          and expires_at > now()
        order by created_at desc
        limit 1
        "#,
        email,
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Err(VerifyError::NotFound);
    };

    if row.attempts >= max_attempts {
        return Err(VerifyError::TooManyAttempts);
    }

    let code_hash = row.code_hash.clone();
    let submitted = submitted_code.to_string();
    let matches = tokio::task::spawn_blocking(move || verify_code(&submitted, &code_hash))
        .await
        .map_err(|e| VerifyError::TaskPanicked(e.to_string()))?
        .map_err(|e| VerifyError::TaskPanicked(e.to_string()))?;

    if !matches {
        sqlx::query!(
            "update otps set attempts = attempts + 1 where id = $1",
            row.id,
        )
        .execute(pool)
        .await?;
        return Err(VerifyError::Incorrect);
    }

    sqlx::query!(
        "update otps set consumed_at = $1 where id = $2",
        Utc::now(),
        row.id,
    )
    .execute(pool)
    .await?;

    Ok(())
}