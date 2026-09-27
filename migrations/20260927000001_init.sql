-- FitTune core schema.
--
-- Enumerations are stored as TEXT guarded by CHECK constraints; the Rust
-- domain enums are the source of truth for their values.

CREATE TABLE users (
    id              UUID PRIMARY KEY,
    username        TEXT NOT NULL,
    email           TEXT NOT NULL,
    password_hash   TEXT NOT NULL,
    display_name    TEXT,
    birthday        DATE,
    role            TEXT NOT NULL DEFAULT 'user' CHECK (role IN ('user', 'admin')),
    account_type    TEXT CHECK (account_type IN (
                        'gym_enthusiast', 'professional_trainer', 'nutritionist',
                        'psychologist', 'physical_therapist')),
    weight_unit     TEXT NOT NULL DEFAULT 'kg' CHECK (weight_unit IN ('kg', 'lb')),
    distance_unit   TEXT NOT NULL DEFAULT 'km' CHECK (distance_unit IN ('km', 'mi')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX users_username_key ON users (lower(username));
CREATE UNIQUE INDEX users_email_key ON users (lower(email));

CREATE TABLE sessions (
    id            UUID PRIMARY KEY,
    user_id       UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash    BYTEA NOT NULL UNIQUE,
    user_agent    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);

-- Exercises with owner_id NULL form the shared catalog; others are a user's custom exercises.
-- Exercises are archived instead of deleted so workout history keeps its references.
CREATE TABLE exercises (
    id                 UUID PRIMARY KEY,
    owner_id           UUID REFERENCES users (id) ON DELETE CASCADE,
    name               TEXT NOT NULL,
    tracking           TEXT NOT NULL CHECK (tracking IN ('weight_reps', 'reps', 'duration', 'distance_duration')),
    primary_muscle     TEXT NOT NULL,
    secondary_muscles  TEXT[] NOT NULL DEFAULT '{}',
    equipment          TEXT NOT NULL DEFAULT 'none',
    difficulty         TEXT NOT NULL DEFAULT 'beginner' CHECK (difficulty IN ('beginner', 'intermediate', 'advanced')),
    video_id           TEXT,
    instructions       TEXT,
    archived_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX exercises_owner_name_key ON exercises (owner_id, lower(name)) NULLS NOT DISTINCT
    WHERE archived_at IS NULL;
CREATE INDEX exercises_owner_id_idx ON exercises (owner_id);

-- Routines are reusable workout plans. Per-set targets are only ever read
-- together with their routine, so they live in a JSONB column.
CREATE TABLE routines (
    id          UUID PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    notes       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX routines_user_id_idx ON routines (user_id);

CREATE TABLE routine_exercises (
    id            UUID PRIMARY KEY,
    routine_id    UUID NOT NULL REFERENCES routines (id) ON DELETE CASCADE,
    exercise_id   UUID NOT NULL REFERENCES exercises (id),
    position      INTEGER NOT NULL,
    rest_seconds  INTEGER,
    notes         TEXT,
    sets          JSONB NOT NULL DEFAULT '[]',
    UNIQUE (routine_id, position)
);

-- Workouts use client-generated ids so an in-progress workout can be created
-- offline and synchronised later with idempotent full-document writes.
CREATE TABLE workouts (
    id          UUID PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    routine_id  UUID REFERENCES routines (id) ON DELETE SET NULL,
    title       TEXT NOT NULL,
    notes       TEXT,
    started_at  TIMESTAMPTZ NOT NULL,
    ended_at    TIMESTAMPTZ,
    revision    BIGINT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (ended_at IS NULL OR ended_at >= started_at)
);

CREATE INDEX workouts_user_started_idx ON workouts (user_id, started_at DESC, id DESC);
CREATE INDEX workouts_routine_id_idx ON workouts (routine_id);

CREATE TABLE workout_exercises (
    id            UUID PRIMARY KEY,
    workout_id    UUID NOT NULL REFERENCES workouts (id) ON DELETE CASCADE,
    exercise_id   UUID NOT NULL REFERENCES exercises (id),
    position      INTEGER NOT NULL,
    rest_seconds  INTEGER,
    notes         TEXT,
    UNIQUE (workout_id, position)
);

CREATE INDEX workout_exercises_exercise_id_idx ON workout_exercises (exercise_id);

CREATE TABLE workout_sets (
    id                   UUID PRIMARY KEY,
    workout_exercise_id  UUID NOT NULL REFERENCES workout_exercises (id) ON DELETE CASCADE,
    position             INTEGER NOT NULL,
    kind                 TEXT NOT NULL DEFAULT 'normal' CHECK (kind IN ('warmup', 'normal', 'drop', 'failure')),
    reps                 INTEGER,
    weight_kg            DOUBLE PRECISION,
    duration_seconds     INTEGER,
    distance_m           DOUBLE PRECISION,
    rpe                  DOUBLE PRECISION,
    completed            BOOLEAN NOT NULL DEFAULT false,
    UNIQUE (workout_exercise_id, position)
);

CREATE TABLE activities (
    id                UUID PRIMARY KEY,
    user_id           UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('run', 'ride', 'walk', 'hike', 'swim', 'row', 'other')),
    title             TEXT NOT NULL,
    notes             TEXT,
    started_at        TIMESTAMPTZ NOT NULL,
    duration_seconds  INTEGER NOT NULL CHECK (duration_seconds > 0),
    distance_m        DOUBLE PRECISION,
    elevation_gain_m  DOUBLE PRECISION,
    avg_heart_rate    INTEGER,
    calories          INTEGER,
    perceived_effort  INTEGER,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX activities_user_started_idx ON activities (user_id, started_at DESC, id DESC);

-- Epley estimated one-rep max. Single reps are taken as-is; above 12 reps the
-- estimate is too unreliable to be useful, so it is omitted.
CREATE FUNCTION estimated_1rm(weight_kg DOUBLE PRECISION, reps INTEGER)
RETURNS DOUBLE PRECISION
LANGUAGE SQL IMMUTABLE PARALLEL SAFE
AS $$
    SELECT CASE
        WHEN weight_kg IS NULL OR reps IS NULL OR weight_kg <= 0 OR reps <= 0 THEN NULL
        WHEN reps = 1 THEN weight_kg
        WHEN reps <= 12 THEN weight_kg * (1 + reps / 30.0)
        ELSE NULL
    END
$$;
