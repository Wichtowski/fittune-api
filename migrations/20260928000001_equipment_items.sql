-- Specific equipment items replace equipment categories for matching exercises to places.
-- Exercises list every item they need in `requires`; a place lists the items it has, and an
-- exercise is doable there when all of its required items are present. The `equipment`
-- category on exercises stays for display and library filters.

-- In the declaration order of the Rust EquipmentItem enum, which is also the canonical order
-- the API stores lists in
CREATE FUNCTION equipment_items()
RETURNS TEXT[]
LANGUAGE SQL IMMUTABLE PARALLEL SAFE
AS $$
    SELECT ARRAY[
        'barbell', 'ez_bar', 'dumbbells', 'kettlebells',
        'flat_bench', 'adjustable_bench', 'squat_rack', 'pull_up_bar', 'dip_station',
        'leg_press', 'leg_extension', 'leg_curl', 'calf_raise_machine', 'smith_machine',
        'chest_press_machine', 'pec_deck', 'shoulder_press_machine', 'assisted_pull_up_machine',
        'cable_station', 'lat_pulldown', 'seated_row',
        'treadmill', 'rowing_machine', 'stationary_bike',
        'resistance_band', 'ab_wheel', 'jump_rope'
    ]::TEXT[]
$$;

CREATE FUNCTION is_equipment_items(items TEXT[])
RETURNS BOOLEAN
LANGUAGE SQL IMMUTABLE PARALLEL SAFE
AS $$
    SELECT items <@ equipment_items()
$$;

-- Exercises: what each one needs

ALTER TABLE exercises ADD COLUMN requires TEXT[] NOT NULL DEFAULT '{}'
    CHECK (is_equipment_items(requires));

-- Custom exercises only have a category, so they get its obvious item. Custom machine and
-- other exercises stay unrestricted until their owner picks what they need
UPDATE exercises SET requires = CASE equipment
        WHEN 'barbell' THEN ARRAY['barbell']
        WHEN 'dumbbell' THEN ARRAY['dumbbells']
        WHEN 'kettlebell' THEN ARRAY['kettlebells']
        WHEN 'cable' THEN ARRAY['cable_station']
        WHEN 'band' THEN ARRAY['resistance_band']
        ELSE '{}'
    END
WHERE owner_id IS NOT NULL;

-- Catalog exercises, matched by the same name-derived ids as the catalog seed
UPDATE exercises e SET requires = r.requires
FROM (VALUES
    ('Barbell Bench Press',            ARRAY['barbell', 'flat_bench', 'squat_rack']),
    ('Incline Dumbbell Press',         ARRAY['dumbbells', 'adjustable_bench']),
    ('Dumbbell Fly',                   ARRAY['dumbbells', 'flat_bench']),
    ('Cable Crossover',                ARRAY['cable_station']),
    ('Push-Up',                        ARRAY[]::TEXT[]),
    ('Chest Dip',                      ARRAY['dip_station']),
    ('Barbell Back Squat',             ARRAY['barbell', 'squat_rack']),
    ('Front Squat',                    ARRAY['barbell', 'squat_rack']),
    ('Leg Press',                      ARRAY['leg_press']),
    ('Bulgarian Split Squat',          ARRAY['dumbbells', 'flat_bench']),
    ('Leg Extension',                  ARRAY['leg_extension']),
    ('Walking Lunge',                  ARRAY['dumbbells']),
    ('Conventional Deadlift',          ARRAY['barbell']),
    ('Romanian Deadlift',              ARRAY['barbell']),
    ('Lying Leg Curl',                 ARRAY['leg_curl']),
    ('Hip Thrust',                     ARRAY['barbell', 'flat_bench']),
    ('Kettlebell Swing',               ARRAY['kettlebells']),
    ('Standing Calf Raise',            ARRAY['calf_raise_machine']),
    ('Pull-Up',                        ARRAY['pull_up_bar']),
    ('Chin-Up',                        ARRAY['pull_up_bar']),
    ('Lat Pulldown',                   ARRAY['lat_pulldown']),
    ('One-Arm Dumbbell Row',           ARRAY['dumbbells', 'flat_bench']),
    ('Barbell Row',                    ARRAY['barbell']),
    ('Seated Cable Row',               ARRAY['seated_row']),
    ('Barbell Shrug',                  ARRAY['barbell']),
    ('Overhead Press',                 ARRAY['barbell', 'squat_rack']),
    ('Seated Dumbbell Shoulder Press', ARRAY['dumbbells', 'adjustable_bench']),
    ('Lateral Raise',                  ARRAY['dumbbells']),
    ('Rear Delt Fly',                  ARRAY['dumbbells']),
    ('Face Pull',                      ARRAY['cable_station']),
    ('Barbell Curl',                   ARRAY['barbell']),
    ('Hammer Curl',                    ARRAY['dumbbells']),
    ('Incline Dumbbell Curl',          ARRAY['dumbbells', 'adjustable_bench']),
    ('Triceps Pushdown',               ARRAY['cable_station']),
    ('Skull Crusher',                  ARRAY['barbell', 'flat_bench']),
    ('Overhead Triceps Extension',     ARRAY['dumbbells']),
    ('Close-Grip Bench Press',         ARRAY['barbell', 'flat_bench', 'squat_rack']),
    ('Wrist Curl',                     ARRAY['dumbbells']),
    ('Plank',                          ARRAY[]::TEXT[]),
    ('Hanging Leg Raise',              ARRAY['pull_up_bar']),
    ('Cable Crunch',                   ARRAY['cable_station']),
    ('Ab Wheel Rollout',               ARRAY['ab_wheel']),
    ('Burpee',                         ARRAY[]::TEXT[]),
    ('Treadmill Run',                  ARRAY['treadmill']),
    ('Rowing Machine',                 ARRAY['rowing_machine']),
    ('Stationary Bike',                ARRAY['stationary_bike']),
    ('Jump Rope',                      ARRAY['jump_rope'])
) AS r (name, requires)
WHERE e.id = md5('fittune:exercise:' || lower(r.name))::uuid;

-- Places: convert category lists to items. Categories expand to every item they covered, and
-- gyms also get the benches, racks and bars a commercial gym has, so their exercises stay
-- available. Home and custom places only get what they explicitly listed

ALTER TABLE workout_place_versions DROP CONSTRAINT workout_place_versions_equipment_check;

UPDATE workout_place_versions v SET equipment = ARRAY(
    SELECT item FROM (
        SELECT unnest(CASE category
            WHEN 'barbell' THEN ARRAY['barbell', 'ez_bar']
            WHEN 'dumbbell' THEN ARRAY['dumbbells']
            WHEN 'kettlebell' THEN ARRAY['kettlebells']
            WHEN 'machine' THEN ARRAY['leg_press', 'leg_extension', 'leg_curl', 'calf_raise_machine',
                                      'smith_machine', 'chest_press_machine', 'pec_deck',
                                      'shoulder_press_machine', 'assisted_pull_up_machine',
                                      'treadmill', 'rowing_machine', 'stationary_bike']
            WHEN 'cable' THEN ARRAY['cable_station', 'lat_pulldown', 'seated_row']
            WHEN 'band' THEN ARRAY['resistance_band']
            WHEN 'other' THEN ARRAY['ab_wheel', 'jump_rope']
            ELSE '{}'
        END) AS item
        FROM unnest(v.equipment) AS category
        UNION
        SELECT unnest(ARRAY['flat_bench', 'adjustable_bench', 'squat_rack', 'pull_up_bar', 'dip_station'])
        WHERE v.kind = 'gym'
    ) items
    ORDER BY array_position(equipment_items(), item)
);

ALTER TABLE workout_place_versions ADD CONSTRAINT workout_place_versions_equipment_check
    CHECK (cardinality(equipment) <= 27 AND is_equipment_items(equipment));
