-- Workout places: where a user trains and the equipment available there.
-- A place is edited by adding a version, so workouts keep the exact setup they were logged
-- with, including workouts recorded offline before the place was edited or archived.

CREATE TABLE workout_places (
    id           UUID PRIMARY KEY,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    archived_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX workout_places_user_id_idx ON workout_places (user_id) WHERE archived_at IS NULL;

CREATE TABLE workout_place_versions (
    version_id  UUID PRIMARY KEY,
    id          UUID NOT NULL REFERENCES workout_places (id) ON DELETE CASCADE,
    revision    BIGINT GENERATED ALWAYS AS IDENTITY,
    name        TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    kind        TEXT NOT NULL CHECK (kind IN ('home', 'gym', 'custom')),
    equipment   TEXT[] NOT NULL CHECK (
                    cardinality(equipment) <= 8 AND
                    equipment <@ ARRAY['barbell', 'dumbbell', 'kettlebell', 'machine', 'cable', 'band', 'plate', 'other']::TEXT[]),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX workout_place_versions_latest_idx ON workout_place_versions (id, revision DESC);

ALTER TABLE workouts ADD COLUMN place_version_id UUID
    REFERENCES workout_place_versions (version_id) ON DELETE SET NULL;

CREATE INDEX workouts_place_version_id_idx ON workouts (place_version_id);
