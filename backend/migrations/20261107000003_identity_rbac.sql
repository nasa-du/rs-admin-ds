-- Identity and RBAC schema (spec §4.2).
--
-- The five tables below are the persistence layer for users, roles and the
-- code-owned permission catalog. The catalog itself is synced from domain code
-- at startup (spec §4.4); this migration only defines its mirror.

CREATE TABLE users (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    username text NOT NULL,
    email text NOT NULL,
    display_name text NOT NULL,
    phone text,
    password_hash text NOT NULL,
    status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    must_change_password boolean NOT NULL DEFAULT false,
    temporary_password_expires_at timestamptz,
    token_version bigint NOT NULL DEFAULT 0,
    failed_login_count integer NOT NULL DEFAULT 0,
    locked_until timestamptz,
    last_failed_login_at timestamptz,
    last_login_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT users_username_lower CHECK (username = lower(username)),
    CONSTRAINT users_email_lower CHECK (email = lower(email)),
    CONSTRAINT users_username_format CHECK (username ~ '^[a-z0-9][a-z0-9._-]{2,31}$'),
    CONSTRAINT users_display_name_len CHECK (char_length(display_name) BETWEEN 1 AND 64),
    CONSTRAINT users_phone_len CHECK (phone IS NULL OR char_length(phone) <= 32)
);
CREATE UNIQUE INDEX users_username_uq ON users (username);
CREATE UNIQUE INDEX users_email_uq ON users (email);
CREATE INDEX users_username_trgm ON users USING gin (username gin_trgm_ops);
CREATE INDEX users_email_trgm ON users USING gin (email gin_trgm_ops);
CREATE INDEX users_display_name_trgm ON users USING gin (display_name gin_trgm_ops);

CREATE TABLE roles (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    name text NOT NULL,
    description text,
    is_builtin boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz,
    CONSTRAINT roles_name_len CHECK (char_length(name) BETWEEN 1 AND 64),
    CONSTRAINT roles_description_len CHECK (description IS NULL OR char_length(description) <= 500),
    CONSTRAINT roles_builtin_not_deleted CHECK (NOT is_builtin OR deleted_at IS NULL)
);
CREATE UNIQUE INDEX roles_name_active_uq ON roles (lower(name)) WHERE deleted_at IS NULL;

CREATE TABLE permissions (
    key text PRIMARY KEY,
    group_key text NOT NULL,
    display_name_zh text NOT NULL,
    display_name_en text NOT NULL
);

CREATE TABLE role_permissions (
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE RESTRICT,
    permission_key text NOT NULL REFERENCES permissions(key) ON DELETE RESTRICT,
    PRIMARY KEY (role_id, permission_key)
);

CREATE TABLE user_roles (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE RESTRICT,
    assigned_at timestamptz NOT NULL DEFAULT now(),
    assigned_by uuid REFERENCES users(id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, role_id)
);
CREATE INDEX user_roles_role_idx ON user_roles (role_id);
