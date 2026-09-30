use std::{env, time::Duration};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: Secret,
    pub resend_api_key: Secret,
    pub resend_webhook_secret: Secret,
    pub from_address: String,
    pub port: u16,
    pub worker_count: usize,
    pub queue_capacity: usize,
    pub otp_ttl: Duration,
    pub max_attempts: i16,
    pub per_ip_limit_per_min: u32,
}

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

// The entire reason this newtype exists: a stray `tracing::info!("{config:?}")`
// must never put your API key in a log aggregator.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    Missing(&'static str),
    #[error("invalid value for {key}: {source}")]
    Invalid {
        key: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: Secret(required("DATABASE_URL")?),
            resend_api_key: Secret(required("RESEND_API_KEY")?),
            resend_webhook_secret: Secret(required("RESEND_WEBHOOK_SECRET")?),
            from_address: required("FROM_ADDRESS")?,
            port: parse_or("PORT", 8080)?,
            worker_count: parse_or("WORKER_COUNT", 4)?,
            queue_capacity: parse_or("QUEUE_CAPACITY", 1024)?,
            otp_ttl: Duration::from_secs(parse_or("OTP_TTL_SECS", 300)?),
            max_attempts: parse_or("MAX_ATTEMPTS", 5)?,
            per_ip_limit_per_min: parse_or("PER_IP_LIMIT_PER_MIN", 10)?,
        })
    }
}

fn required(key: &'static str) -> Result<String, ConfigError> {
    env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or(ConfigError::Missing(key))
}

fn parse_or<T>(key: &'static str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(key) {
        Err(_) => Ok(default),
        Ok(raw) => raw.trim().parse().map_err(|e| ConfigError::Invalid {
            key,
            source: Box::new(e),
        }),
    }
}