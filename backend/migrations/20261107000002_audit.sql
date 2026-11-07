-- Audit log and tamper-evident HMAC chain (spec §4.2/§8.5).

CREATE TABLE audit_logs (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    occurred_at timestamptz NOT NULL DEFAULT now(),
    actor_user_id uuid,
    actor_label text NOT NULL,
    action text NOT NULL,
    target_type text NOT NULL,
    target_id uuid,
    target_label text,
    result text NOT NULL CHECK (result IN ('success', 'failure', 'denied')),
    severity text NOT NULL CHECK (severity IN ('info', 'warning', 'error', 'critical')),
    ip inet,
    user_agent text,
    request_id uuid NOT NULL,
    details jsonb NOT NULL DEFAULT '{}'::jsonb,
    chain_seq bigint NOT NULL UNIQUE CHECK (chain_seq > 0),
    prev_hash bytea NOT NULL CHECK (octet_length(prev_hash) = 32),
    row_hash bytea NOT NULL CHECK (octet_length(row_hash) = 32),
    hmac_key_id text NOT NULL CHECK (hmac_key_id <> ''),
    CONSTRAINT audit_logs_actor_label_not_empty CHECK (actor_label <> '')
);
CREATE INDEX audit_logs_occurred_at_idx ON audit_logs (occurred_at DESC);
CREATE INDEX audit_logs_actor_idx ON audit_logs (actor_user_id, occurred_at DESC);
CREATE INDEX audit_logs_action_idx ON audit_logs (action, occurred_at DESC);
CREATE INDEX audit_logs_target_idx ON audit_logs (target_type, target_id, occurred_at DESC);
CREATE INDEX audit_logs_severity_idx ON audit_logs (severity, occurred_at DESC);
CREATE INDEX audit_logs_ip_idx ON audit_logs (ip, occurred_at DESC);
CREATE INDEX audit_logs_request_id_idx ON audit_logs (request_id);

CREATE TABLE audit_chain_head (
    id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    last_seq bigint NOT NULL CHECK (last_seq >= 0),
    last_hash bytea NOT NULL CHECK (octet_length(last_hash) = 32),
    updated_at timestamptz NOT NULL
);

-- 32 zero bytes: the anchor before the first audit row.
INSERT INTO audit_chain_head (id, last_seq, last_hash, updated_at)
VALUES (1, 0, decode(repeat('00', 32), 'hex'), now());
