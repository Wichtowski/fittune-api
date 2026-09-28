CREATE TABLE progress_photos (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    workout_id UUID REFERENCES workouts (id) ON DELETE SET NULL,
    storage_key TEXT NOT NULL UNIQUE,
    width INTEGER NOT NULL CHECK (width > 0),
    height INTEGER NOT NULL CHECK (height > 0),
    bytes INTEGER NOT NULL CHECK (bytes > 0),
    taken_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX progress_photos_user_taken_idx ON progress_photos (user_id, taken_at DESC, id DESC);
CREATE INDEX progress_photos_workout_idx ON progress_photos (workout_id);
