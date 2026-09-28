-- Photos and videos demonstrating an exercise. Photos live in object storage under
-- `storage_key`; videos are embedded from their provider by `external_id`.
CREATE TABLE exercise_media (
    id           UUID PRIMARY KEY,
    exercise_id  UUID NOT NULL REFERENCES exercises (id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('photo', 'video')),
    provider     TEXT NOT NULL CHECK (provider IN ('fittune', 'youtube', 'vimeo')),
    external_id  TEXT,
    storage_key  TEXT UNIQUE,
    -- Where the startup backfill downloads a stored photo from while it is missing
    source_url   TEXT CHECK (source_url LIKE 'https://%'),
    position     INTEGER NOT NULL CHECK (position >= 0),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        (kind = 'photo' AND provider = 'fittune' AND storage_key IS NOT NULL AND external_id IS NULL)
        OR (kind = 'video' AND provider IN ('youtube', 'vimeo') AND external_id IS NOT NULL
            AND storage_key IS NULL AND source_url IS NULL)
    ),
    UNIQUE (exercise_id, kind, position)
);

-- Catalog photos: start (0) and finish (1) frames from the public domain Free Exercise DB,
-- pinned to one revision. Ids are derived from the exercise name so they are stable across
-- environments, and the storage key contains the id so a replaced photo gets a new URL.
INSERT INTO exercise_media (id, exercise_id, kind, provider, storage_key, source_url, position)
SELECT ids.media_id, e.id, 'photo', 'fittune', 'catalog/' || ids.media_id || '.jpg',
       'https://raw.githubusercontent.com/yuhonas/free-exercise-db/a859101d633a01c4a1a920d6a8ce41dabba0705f/exercises/'
           || photos.folder || '/' || frame || '.jpg',
       frame
FROM (VALUES
    ('Barbell Bench Press',            'Barbell_Bench_Press_-_Medium_Grip'),
    ('Incline Dumbbell Press',         'Incline_Dumbbell_Press'),
    ('Dumbbell Fly',                   'Dumbbell_Flyes'),
    ('Cable Crossover',                'Cable_Crossover'),
    ('Chest Dip',                      'Dips_-_Chest_Version'),
    ('Push-Up',                        'Pushups'),
    ('Barbell Back Squat',             'Barbell_Full_Squat'),
    ('Front Squat',                    'Front_Barbell_Squat'),
    ('Leg Press',                      'Leg_Press'),
    ('Leg Extension',                  'Leg_Extensions'),
    ('Conventional Deadlift',          'Barbell_Deadlift'),
    ('Romanian Deadlift',              'Romanian_Deadlift'),
    ('Lying Leg Curl',                 'Lying_Leg_Curls'),
    ('Hip Thrust',                     'Barbell_Hip_Thrust'),
    ('Standing Calf Raise',            'Standing_Calf_Raises'),
    ('Pull-Up',                        'Pullups'),
    ('Chin-Up',                        'Chin-Up'),
    ('Lat Pulldown',                   'Wide-Grip_Lat_Pulldown'),
    ('One-Arm Dumbbell Row',           'One-Arm_Dumbbell_Row'),
    ('Barbell Row',                    'Bent_Over_Barbell_Row'),
    ('Seated Cable Row',               'Seated_Cable_Rows'),
    ('Barbell Shrug',                  'Barbell_Shrug'),
    ('Overhead Press',                 'Barbell_Shoulder_Press'),
    ('Seated Dumbbell Shoulder Press', 'Seated_Dumbbell_Press'),
    ('Lateral Raise',                  'Side_Lateral_Raise'),
    ('Rear Delt Fly',                  'Seated_Bent-Over_Rear_Delt_Raise'),
    ('Face Pull',                      'Face_Pull'),
    ('Barbell Curl',                   'Barbell_Curl'),
    ('Hammer Curl',                    'Hammer_Curls'),
    ('Incline Dumbbell Curl',          'Incline_Dumbbell_Curl'),
    ('Triceps Pushdown',               'Triceps_Pushdown'),
    ('Skull Crusher',                  'Lying_Triceps_Press'),
    ('Overhead Triceps Extension',     'Standing_Dumbbell_Triceps_Extension'),
    ('Close-Grip Bench Press',         'Close-Grip_Barbell_Bench_Press'),
    ('Wrist Curl',                     'Palms-Up_Dumbbell_Wrist_Curl_Over_A_Bench'),
    ('Plank',                          'Plank'),
    ('Hanging Leg Raise',              'Hanging_Leg_Raise'),
    ('Cable Crunch',                   'Cable_Crunch'),
    ('Ab Wheel Rollout',               'Ab_Roller'),
    ('Treadmill Run',                  'Running_Treadmill'),
    ('Rowing Machine',                 'Rowing_Stationary'),
    ('Stationary Bike',                'Bicycling_Stationary'),
    ('Jump Rope',                      'Rope_Jumping')
) AS photos (name, folder)
CROSS JOIN generate_series(0, 1) AS frame
CROSS JOIN LATERAL (
    SELECT md5('fittune:exercise-media:' || lower(photos.name) || ':photo:' || frame)::uuid AS media_id
) AS ids
JOIN exercises e ON e.owner_id IS NULL AND lower(e.name) = lower(photos.name);

-- Catalog videos
INSERT INTO exercise_media (id, exercise_id, kind, provider, external_id, position)
SELECT md5('fittune:exercise-media:' || lower(videos.name) || ':video:0')::uuid, e.id, 'video',
       videos.provider, videos.external_id, 0
FROM (VALUES
    ('Barbell Bench Press',   'youtube', 'hWbUlkb5Ms4'),
    ('Barbell Back Squat',    'youtube', '8060FZiT5TA'),
    ('Conventional Deadlift', 'youtube', 'ZaTM37cfiDs'),
    ('Pull-Up',               'youtube', 'aNUSgyWRJYA'),
    ('Barbell Curl',          'vimeo',   '278191577')
) AS videos (name, provider, external_id)
JOIN exercises e ON e.owner_id IS NULL AND lower(e.name) = lower(videos.name);

-- YouTube ids set on exercises so far. The column is no longer read and is dropped once the
-- app reads `media`; until then an older API binary can still run against this schema.
-- A catalog video above takes precedence over a column value for the same exercise.
INSERT INTO exercise_media (id, exercise_id, kind, provider, external_id, position)
SELECT gen_random_uuid(), id, 'video', 'youtube', video_id, 0
FROM exercises
WHERE video_id IS NOT NULL
ON CONFLICT (exercise_id, kind, position) DO NOTHING;
