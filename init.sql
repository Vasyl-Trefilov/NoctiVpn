-- 1. Setup Extensions & Enums
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TYPE sub_status AS ENUM ('active', 'expired', 'banned');

-- 2. Tariffs (The Plans: 1, 2, 3, 4)
-- This maps directly to your Xray "userLevel"
CREATE TABLE tariffs (
    id              SMALLINT PRIMARY KEY, -- 1, 2, 3, 4
    name            TEXT NOT NULL,        -- "Start", "Pro", etc.
    price           NUMERIC(10, 2) NOT NULL,
    speed_limit_mbps INT NOT NULL,        -- 1, 7, 25, 50
    xray_level      INT NOT NULL          -- Maps to Xray config userLevel
);

-- Seed the tariffs immediately so they exist for constraints
INSERT INTO tariffs (id, name, price, speed_limit_mbps, xray_level) VALUES
(1, 'Basic', 29.00, 1, 1),
(2, 'Standard', 79.00, 7, 2),
(3, 'Pro', 279.00, 25, 3),
(4, 'Ultra', 399.00, 50, 4);

-- 3. Servers (Nodes)
CREATE TABLE servers (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug           TEXT NOT NULL UNIQUE,      -- e.g. "de-helsinki-1"
    ip_address     INET NOT NULL,             -- 1.2.3.4
    domain         TEXT NOT NULL,             -- "vpn1.example.com"
    api_secret     TEXT NOT NULL DEFAULT encode(gen_random_bytes(32), 'hex'),

    -- Xray Connection Info
    api_port       INT NOT NULL DEFAULT 8080,
    grpc_port      INT,                       -- If you use gRPC api later
    public_key     TEXT NOT NULL,             -- Reality Public Key
    short_ids      TEXT[] NOT NULL DEFAULT '{}',
    
    -- Capacity Management
    max_users      INT NOT NULL DEFAULT 100,  -- Hard limit for this server
    is_enabled     BOOLEAN NOT NULL DEFAULT true,
    
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 4. Users (Telegram info)
CREATE TABLE users (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tg_id       BIGINT NOT NULL UNIQUE,
    username    TEXT,
    full_name   TEXT,
    language    VARCHAR(5) DEFAULT 'en',
    balance     NUMERIC(10, 2) DEFAULT 0.00,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 5. Subscriptions (The Link between User, Server, and Tariff)
CREATE TABLE subscriptions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    server_id   UUID NOT NULL REFERENCES servers(id),
    tariff_id   SMALLINT NOT NULL REFERENCES tariffs(id),
    
    -- This is the UUID that goes into Xray Config
    xray_uuid   UUID NOT NULL DEFAULT gen_random_uuid(),
    email       TEXT NOT NULL, -- "user_1mbit_uuid" for identification in logs
    
    status      sub_status NOT NULL DEFAULT 'active',
    expire_date TIMESTAMPTZ NOT NULL,
    
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    
    -- Ensure Xray UUID is unique across the whole system to prevent collisions
    CONSTRAINT uq_xray_uuid UNIQUE (xray_uuid)
);

-- Indexes for performance
CREATE INDEX idx_subs_user ON subscriptions(user_id);
CREATE INDEX idx_subs_server ON subscriptions(server_id) WHERE status = 'active';
CREATE INDEX idx_subs_expiry ON subscriptions(expire_date);

-- 6. View: Server Load (Real-time Analytics)
-- This answers your question: "How many users are on this server?"
CREATE OR REPLACE VIEW view_server_load AS
SELECT 
    s.id, 
    s.slug, 
    s.ip_address,
    s.max_users,
    COUNT(sub.id) AS current_users,
    (s.max_users - COUNT(sub.id)) AS slots_available,
    ROUND((COUNT(sub.id)::numeric / s.max_users::numeric) * 100, 1) AS load_percentage
FROM servers s
LEFT JOIN subscriptions sub ON s.id = sub.server_id AND sub.status = 'active'
GROUP BY s.id;

-- 7. Auto-Update Timestamp Function
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_updated_at BEFORE UPDATE ON users FOR EACH ROW EXECUTE PROCEDURE set_updated_at();
CREATE TRIGGER servers_updated_at BEFORE UPDATE ON servers FOR EACH ROW EXECUTE PROCEDURE set_updated_at();
CREATE TRIGGER subscriptions_updated_at BEFORE UPDATE ON subscriptions FOR EACH ROW EXECUTE PROCEDURE set_updated_at();