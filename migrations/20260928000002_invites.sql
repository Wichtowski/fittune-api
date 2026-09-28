-- Invite-only registration. Admins issue codes; only a SHA-256 digest of each code is stored,
-- and a code is consumed in the same transaction that creates the account

CREATE TABLE invites (
    id          UUID PRIMARY KEY,
    code_hash   BYTEA NOT NULL UNIQUE,
    note        TEXT CHECK (char_length(note) <= 120),
    max_uses    INTEGER NOT NULL DEFAULT 1 CHECK (max_uses BETWEEN 1 AND 50),
    use_count   INTEGER NOT NULL DEFAULT 0 CHECK (use_count BETWEEN 0 AND max_uses),
    expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at  TIMESTAMPTZ,
    created_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX invites_created_at_idx ON invites (created_at DESC);

-- Which invite an account was created with, for auditing
ALTER TABLE users ADD COLUMN invite_id UUID REFERENCES invites (id) ON DELETE SET NULL;
CREATE INDEX users_invite_id_idx ON users (invite_id);
