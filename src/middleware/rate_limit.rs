
use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use governor::{Quota, RateLimiter};
use governor::state::keyed::DefaultKeyedStateStore;
use governor::clock::DefaultClock;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

pub type IpRateLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

pub fn build_limiter(per_minute: u32) -> Arc<IpRateLimiter> {
    let per_minute = NonZeroU32::new(per_minute).unwrap_or(NonZeroU32::new(10).unwrap());
    Arc::new(RateLimiter::keyed(Quota::per_minute(per_minute)))
}

pub async fn per_ip_rate_limit(
    State(limiter): State<Arc<IpRateLimiter>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let result = limiter.check_key(&addr.ip());
    tracing::debug!(ip = %addr.ip(), allowed = result.is_ok(), "rate limit check");
    match result {
        Ok(_) => Ok(next.run(request).await),
        Err(_) => Err(StatusCode::TOO_MANY_REQUESTS),
    }
}