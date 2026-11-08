-- Login throttling state (spec §4.2, §8.3).
--
-- The account layer for known users lives on `users`; this migration adds the
-- unknown-identifier account rows and the shared IP/global sliding-throttle
-- rows. Throttle keys are HMAC-SHA256 digests: plaintext identifiers and
-- addresses are never stored.

CREATE TABLE auth_login_failures (
    key_hash bytea PRIMARY KEY,
    failed_count integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX auth_login_failures_updated_idx ON auth_login_failures (updated_at);

CREATE TABLE auth_login_throttle (
    scope text NOT NULL CHECK (scope IN ('ip', 'global')),
    key_hash bytea NOT NULL,
    window_start timestamptz NOT NULL,
    failures integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    last_failure_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, key_hash)
);
CREATE INDEX auth_login_throttle_last_failure_idx ON auth_login_throttle (last_failure_at);
