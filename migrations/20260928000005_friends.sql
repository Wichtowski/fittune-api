-- Friends: requests, accepted friendships, blocks and what each user shares with friends.
-- Declined, cancelled and removed friendships are deleted, so a pair has at most one row

CREATE TABLE friendships (
    requester_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    addressee_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    status        TEXT NOT NULL CHECK (status IN ('pending', 'accepted')),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    responded_at  TIMESTAMPTZ,
    PRIMARY KEY (requester_id, addressee_id),
    CHECK (requester_id <> addressee_id),
    CHECK ((status = 'accepted') = (responded_at IS NOT NULL))
);

CREATE UNIQUE INDEX friendships_pair_key
    ON friendships (LEAST(requester_id, addressee_id), GREATEST(requester_id, addressee_id));
CREATE INDEX friendships_addressee_idx ON friendships (addressee_id);

-- Directional, so each side can block and unblock on its own
CREATE TABLE user_blocks (
    blocker_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker_id, blocked_id),
    CHECK (blocker_id <> blocked_id)
);

CREATE INDEX user_blocks_blocked_idx ON user_blocks (blocked_id);

-- A missing row means nothing is shared
CREATE TABLE friend_sharing (
    user_id           UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    workouts          BOOLEAN NOT NULL DEFAULT false,
    activities        BOOLEAN NOT NULL DEFAULT false,
    stats             BOOLEAN NOT NULL DEFAULT false,
    personal_records  BOOLEAN NOT NULL DEFAULT false,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
