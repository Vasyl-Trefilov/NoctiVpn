-- Enable UUID extension
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Users: Telegram users with a generated UUID for Xray
CREATE TABLE users (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tg_id      BIGINT NOT NULL UNIQUE,
    uuid       UUID NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    is_active  BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_users_tg_id ON users (tg_id);
CREATE INDEX idx_users_uuid ON users (uuid);
CREATE INDEX idx_users_is_active ON users (is_active) WHERE is_active = true;

-- Servers: Proxy servers (Xray nodes)
CREATE TABLE servers (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug       TEXT NOT NULL UNIQUE,
    api_secret TEXT NOT NULL,
    domain     TEXT NOT NULL,
    public_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_servers_slug ON servers (slug);

-- Subscriptions: User plans and expiration
CREATE TABLE subscriptions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    plan_id     TEXT NOT NULL,
    expire_date TIMESTAMPTZ NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'expired', 'cancelled')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id)
);

CREATE INDEX idx_subscriptions_user_id ON subscriptions (user_id);
CREATE INDEX idx_subscriptions_expire_date ON subscriptions (expire_date);
CREATE INDEX idx_subscriptions_status ON subscriptions (status);

-- Trigger to refresh updated_at
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE PROCEDURE set_updated_at();
CREATE TRIGGER servers_updated_at
    BEFORE UPDATE ON servers
    FOR EACH ROW EXECUTE PROCEDURE set_updated_at();
CREATE TRIGGER subscriptions_updated_at
    BEFORE UPDATE ON subscriptions
    FOR EACH ROW EXECUTE PROCEDURE set_updated_at();
