-- Sessions and refresh tokens (spec §4.2, §5.2–§5.3).
--
-- Sessions are server-side records: `auth_sessions.id` is the access token
-- `sid` and the boundary of a refresh-token family. Refresh tokens are stored
-- only as SHA-256 hashes; the raw value never reaches the database.

CREATE TABLE auth_sessions (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    idle_expires_at timestamptz NOT NULL,
    absolute_expires_at timestamptz NOT NULL,
    authenticated_at timestamptz NOT NULL,
    revoked_at timestamptz,
    revoke_reason text,
    ip inet,
    user_agent text
);
CREATE INDEX auth_sessions_user_active_idx
    ON auth_sessions (user_id, last_seen_at)
    WHERE revoked_at IS NULL;

CREATE TABLE auth_refresh_tokens (
    token_hash bytea PRIMARY KEY,
    session_id uuid NOT NULL REFERENCES auth_sessions(id) ON DELETE RESTRICT,
    rotation_counter bigint NOT NULL,
    issued_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at timestamptz,
    replaced_by_hash bytea UNIQUE,
    revoked_at timestamptz,
    revoked_reason text,
    CONSTRAINT auth_refresh_tokens_counter_nonneg CHECK (rotation_counter >= 0)
);
CREATE INDEX auth_refresh_tokens_session_idx
    ON auth_refresh_tokens (session_id, rotation_counter);
