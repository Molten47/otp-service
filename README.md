# otp-service

A high-throughput, low-latency OTP (one-time password) delivery service written in Rust. Built as a deep dive into async Rust, not just "send an email" — the point was to design something that behaves correctly under load, fails safely, and takes advantage of Rust's type system rather than working around it.

## What it does

1. A client requests a code for an email address (`POST /otp`).
2. The service validates the address, generates a 6-digit code, hashes it, and queues it for delivery — then responds immediately (`202`-style `queued`), without waiting on the email provider.
3. A background worker pool picks the job off the queue and sends it via [Resend](https://resend.com)'s HTTP API.
4. The client submits the code back (`POST /otp/verify`) to redeem it. Codes are single-use, expire after 5 minutes, and lock out after too many wrong attempts.
5. Resend calls back (`POST /webhooks/resend`) with delivery status — sent, delivered, bounced, complained — so the system knows what actually happened to an email, not just that an API call returned 200.

## Why it's built this way

The naive version of this service sends the email inline inside the HTTP handler. That's wrong for two reasons: it ties your API's latency to a third-party SMTP/HTTP round trip, and it re-establishes a connection per request instead of reusing one. This service decouples *accepting* a request from *fulfilling* it:

```
Client → Axum handler → validate, hash, rate-limit, insert → push onto channel → respond
                                                                    ↓
                                                    Fixed-size worker pool
                                                    pulls jobs → sends via
                                                    pooled reqwest client
```

That's a producer/consumer pipeline built on `tokio::sync::mpsc`, which gives bounded backpressure for free — if workers fall behind, the channel applies pressure back to the producer instead of memory growing unbounded.

## Stack

| Concern | Choice | Why |
|---|---|---|
| Web framework | [Axum](https://github.com/tokio-rs/axum) | Tower-based middleware composition, first-class async |
| Database | Postgres (via [Neon](https://neon.tech), serverless) | Need `expires_at`/`attempts` as real columns, not TTL tricks |
| DB driver | [sqlx](https://github.com/launchbadge/sqlx) | Async-native, compile-time-checked queries, no ORM overhead |
| Email delivery | [Resend](https://resend.com) HTTP API | Reliable transactional delivery with delivery-status webhooks; a personal Gmail SMTP account isn't built for automated sending and has no feedback loop |
| HTTP client | `reqwest` (rustls, HTTP/2) | Connection pooling + multiplexing, same latency win SMTP connection pooling would give, over a plain HTTP API |
| Password/code hashing | `argon2` | OTPs are only ~20 bits of entropy; a fast hash is brute-forceable in microseconds if the DB ever leaks |
| Webhook verification | `svix` | Resend's webhooks are Svix-signed; hand-rolling HMAC verification isn't worth the risk |
| Rate limiting (per-IP) | `governor` | In-memory GCRA limiter, cheap first line of defense before a DB round trip |
| TLS | `rustls` everywhere | Pure Rust, no OpenSSL system dependency, works cleanly in minimal Docker images |

## Architecture decisions worth knowing about

**Every OTP send is queued, never sent synchronously.** The `/otp` handler's job ends at "validated, hashed, inserted, queued." A separate pool of `tokio::spawn` workers, sharing one `reqwest::Client` (and therefore one connection pool), does the actual sending. This is why the endpoint responds in milliseconds regardless of how slow Resend's API is having a moment.

**The OTP code itself never touches disk in plaintext.** Only its Argon2 hash is stored (`otps.code_hash`). The plaintext code exists only transiently: generated in the handler, passed down the channel inside a `SendJob`, used once by a worker to build the email body, then dropped.

**Argon2 hashing runs through `tokio::task::spawn_blocking`, never awaited directly.** Argon2 is deliberately slow (~50-100ms) and CPU-bound. Awaiting it inline in an async handler would block a Tokio worker thread and stall unrelated requests under load. Every call to `hash_code` / `verify_code` goes through `spawn_blocking`.

**Rate limiting is two independent layers, protecting two different things:**
- **Per-IP** (`governor`, in-memory, ~10/min): cheap, fast rejection of flooding before it costs a database round trip. Per-instance, not shared across replicas — that's an acceptable tradeoff since this layer is DoS hygiene, not the real defense.
- **Per-email and global daily cap** (Postgres, inside the insert transaction): the actual defense. One address can't be bombed with codes, and no single attacker can exhaust the (free-tier, capped) Resend quota and lock out every legitimate user for the rest of the day.

**Webhook delivery events are append-only, not a status column.** Resend delivers a *sequence* of events per email (`sent`, then `delivered` or `bounced`, possibly `complained`), which can arrive out of order or be redelivered. Modeling this as a single mutable `status` column invites races and throws away history. Instead, `email_delivery_events` is insert-only, keyed uniquely on the provider's event ID, so a retried webhook delivery is a harmless no-op.

**Expiry is a `WHERE` clause, not a background sweep job.** `otps.expires_at` is checked in the query itself (`expires_at > now()`), so an expired code simply stops matching — no cron job, no cleanup task required for correctness.

**Webhook signatures are verified against the raw request body, before any JSON parsing.** Axum's `Json` extractor would deserialize-then-drop the original bytes, which breaks Svix's signature check (any reformatting invalidates it). The webhook handler takes `Bytes` instead, verifies first, parses second.

## Known limitations (by design, for now)

- **Sandbox sender only.** No custom domain is verified in Resend yet, so outbound mail can only be delivered to the Resend account's own email address. Swapping in a verified domain is a config change, not a code change (`FROM_ADDRESS` in `.env`).
- **Per-IP rate limiting keys on the direct TCP peer**, which is correct for local development but wrong behind a reverse proxy (Render, ngrok) without reading `X-Forwarded-For` — everyone behind the proxy currently shares one bucket. This needs proxy-aware IP extraction before real deployment.
- **The Postgres per-email rate-limit check has a narrow race window** under extreme concurrency (two simultaneous requests for the same address could both pass the count check before either commits). At human-driven request volumes this doesn't matter; a `SERIALIZABLE` isolation level or advisory lock would close it if it ever needed to be airtight.
- **A failed webhook DB write after a successful send is only logged, not retried.** The row won't reflect the true delivery status in that narrow failure window.

## Local setup

### Prerequisites

- Rust 1.85+ (edition 2024)
- A Postgres database (this project uses [Neon](https://neon.tech)'s free tier)
- A [Resend](https://resend.com) account and API key
- [ngrok](https://ngrok.com) (for local webhook testing)
- `sqlx-cli`: `cargo install sqlx-cli --no-default-features --features rustls,postgres`

### Environment

Copy `.env.example` to `.env` and fill in:

```
DATABASE_URL=postgres://user:pass@host/db?sslmode=require   # use the DIRECT (non-pooled) connection string — see note below
RESEND_API_KEY=re_...
RESEND_WEBHOOK_SECRET=whsec_...                              # from the Resend webhook's detail page, created below
FROM_ADDRESS=onboarding@resend.dev                           # swap once a domain is verified
PORT=8081
WORKER_COUNT=4
QUEUE_CAPACITY=1024
OTP_TTL_SECS=300
MAX_ATTEMPTS=5
PER_IP_LIMIT_PER_MIN=10
```

> **Neon connection string note:** use the *direct* connection string (no `-pooler` in the hostname), not the pooled one. `sqlx`'s compile-time query macros issue a `PREPARE` and `DESCRIBE` that expect the same backend connection; PgBouncer's transaction-pooling mode can route them to different backends, producing a `prepared statement does not exist` error that has nothing to do with your queries.

### Run migrations

```bash
sqlx database create   # no-op on Neon, which auto-provisions
sqlx migrate run
```

### Run the server

```bash
cargo run
```

### Wire up webhooks locally

```bash
ngrok http 8081
```

Take the forwarding URL and register `https://<subdomain>.ngrok-free.app/webhooks/resend` as an endpoint in the [Resend Webhooks dashboard](https://resend.com/webhooks), subscribed to `email.sent`, `email.delivered`, `email.bounced`, `email.delivery_delayed`, `email.complained`. Copy the signing secret it generates into `.env` as `RESEND_WEBHOOK_SECRET`, then restart the server.

Note: the free ngrok URL changes every time the tunnel restarts — you'll need to update the endpoint URL in Resend's dashboard each time, or the previous session's webhook calls will 404 against a dead tunnel.

## API

### `POST /otp`

```json
{ "email": "user@example.com" }
```

→ `200 { "status": "queued" }`, `400` on malformed email, `429` if rate-limited.

### `POST /otp/verify`

```json
{ "email": "user@example.com", "code": "123456" }
```

→ `200 { "status": "verified" }`, `400` on wrong/expired/unknown code (deliberately indistinguishable, to avoid leaking which case applied), `429` after too many attempts.

### `POST /webhooks/resend`

Svix-signed callback from Resend. Not meant to be called directly.

## Project layout

```
src/
├── main.rs               # config → state → serve; graceful shutdown
├── lib.rs                # module declarations
├── config.rs             # env → typed Config, fails fast on boot
├── state.rs              # AppState: pool, sender, job channel, limiter
├── middleware/
│   └── rate_limit.rs      # per-IP governor layer
├── otp/
│   ├── generate.rs        # CSPRNG code generation, Argon2 hash/verify
│   ├── store.rs           # all OTP-related SQL lives here
│   └── verify.rs          # verification logic against store + hashing
├── email/
│   ├── mod.rs              # EmailSender trait (object-safe, swappable)
│   └── resend.rs           # Resend HTTP API implementation
├── worker.rs               # mpsc consumer pool
└── routes/
    ├── otp.rs               # POST /otp, POST /otp/verify
    └── webhooks.rs          # POST /webhooks/resend
```

`email/` has no dependency on `sqlx`; `otp/store.rs` is the only file that touches SQL directly. That separation is what makes `EmailSender` trivially swappable (SES, SMTP, or a second provider all just become another `impl`) and keeps the codebase testable without a live database for the parts that don't need one.

## Status

Core pipeline (request → hash → rate-limit → queue → send → verify → webhook) is built and tested end-to-end against real infrastructure. Remaining before this is deployment-ready: automated tests, verified sending domain, proxy-aware IP extraction, and the actual Render deploy.