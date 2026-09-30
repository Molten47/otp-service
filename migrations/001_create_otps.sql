create extension if not exists "uuid-ossp";

create table otps (
    id                   uuid        primary key default uuid_generate_v4(),
    email                text        not null,
    code_hash            text        not null,
    expires_at           timestamptz not null,
    attempts             smallint    not null default 0,
    consumed_at          timestamptz,
    created_at           timestamptz not null default now(),

    -- filled in by the worker, after the send is accepted
    provider_message_id  text,
    sent_at              timestamptz,
    send_error           text,

    -- for rate limiting and abuse forensics
    requester_ip         inet
);

-- verification path: newest live OTP for an address
create index otps_email_live_idx
    on otps (email, created_at desc)
    where consumed_at is null;

-- webhook path: map provider ids back to our rows
create unique index otps_provider_message_id_idx
    on otps (provider_message_id)
    where provider_message_id is not null;

-- rate limiting: count recent requests per address
create index otps_email_created_at_idx on otps (email, created_at);

create table email_delivery_events (
    provider_event_id text        primary key,
    otp_id            uuid        references otps (id) on delete set null,
    event_type        text        not null,
    payload           jsonb       not null,
    occurred_at       timestamptz not null,
    received_at       timestamptz not null default now()
);

create index email_delivery_events_otp_id_idx on email_delivery_events (otp_id);