-- Schema for the exercise catalog imported from hasaneyldrm/exercises-dataset by the next
-- migration: a stable reference to the source row, Polish instructions, animated demos and the
-- equipment the larger catalog needs

-- Source row of a catalog exercise, e.g. `gymvisual:0025`. Later corrections address exercises
-- by it instead of by their name, which an admin may change
ALTER TABLE exercises ADD COLUMN catalog_ref TEXT UNIQUE;

-- `instructions` holds the English text and what users write for custom exercises
ALTER TABLE exercises ADD COLUMN instructions_pl TEXT;

-- Equipment: the same list, in the same order, as the Rust EquipmentItem enum
CREATE OR REPLACE FUNCTION equipment_items()
RETURNS TEXT[]
LANGUAGE SQL IMMUTABLE PARALLEL SAFE
AS $$
    SELECT ARRAY[
        'barbell', 'ez_bar', 'trap_bar', 'dumbbells', 'kettlebells', 'weight_plates',
        'flat_bench', 'adjustable_bench', 'preacher_bench', 'back_extension_bench', 'squat_rack',
        'pull_up_bar', 'dip_station',
        'leg_press', 'leg_extension', 'leg_curl', 'calf_raise_machine', 'smith_machine',
        'chest_press_machine', 'pec_deck', 'shoulder_press_machine', 'assisted_pull_up_machine',
        'strength_machines',
        'cable_station', 'lat_pulldown', 'seated_row',
        'treadmill', 'rowing_machine', 'stationary_bike', 'cardio_machines',
        'resistance_band', 'suspension_trainer', 'stability_ball', 'bosu_ball', 'medicine_ball',
        'foam_roller', 'plyo_box', 'ab_wheel', 'jump_rope', 'battle_ropes', 'climbing_rope',
        'sledgehammer_tire'
    ]::TEXT[]
$$;

ALTER TABLE workout_place_versions DROP CONSTRAINT workout_place_versions_equipment_check;

-- Places saved before these items existed could not list them, so the new catalog exercises
-- would all be hidden there. Gyms get what a commercial gym has, and any place with a barbell
-- has plates for it. Rows are updated in place like the item conversion before: the versions
-- record what was available, and nobody could have said these were missing
UPDATE workout_place_versions v SET equipment = ARRAY(
    SELECT item FROM (
        SELECT unnest(v.equipment) AS item
        UNION
        SELECT unnest(ARRAY['weight_plates', 'preacher_bench', 'back_extension_bench',
                            'strength_machines', 'cardio_machines', 'stability_ball',
                            'medicine_ball', 'foam_roller', 'plyo_box'])
        WHERE v.kind = 'gym'
        UNION
        SELECT 'weight_plates' WHERE 'barbell' = ANY (v.equipment)
    ) items
    ORDER BY array_position(equipment_items(), item)
)
WHERE v.kind = 'gym' OR 'barbell' = ANY (v.equipment);

ALTER TABLE workout_place_versions ADD CONSTRAINT workout_place_versions_equipment_check
    CHECK (cardinality(equipment) <= 42 AND is_equipment_items(equipment));

-- Media: animations are GIFs stored like photos. `stored_at` is set once the file is in object
-- storage, so the backfill only looks at missing files and the API only serves stored ones
ALTER TABLE exercise_media
    ADD COLUMN attribution TEXT,
    ADD COLUMN stored_at TIMESTAMPTZ,
    DROP CONSTRAINT exercise_media_kind_check,
    DROP CONSTRAINT exercise_media_check,
    ADD CONSTRAINT exercise_media_kind_check CHECK (kind IN ('photo', 'animation', 'video')),
    ADD CONSTRAINT exercise_media_check CHECK (
        (kind IN ('photo', 'animation') AND provider = 'fittune' AND storage_key IS NOT NULL
            AND external_id IS NULL)
        OR (kind = 'video' AND provider IN ('youtube', 'vimeo') AND external_id IS NOT NULL
            AND storage_key IS NULL AND source_url IS NULL AND stored_at IS NULL)
    );

UPDATE exercise_media SET attribution = 'Free Exercise DB' WHERE source_url IS NOT NULL;

CREATE INDEX exercise_media_missing_idx ON exercise_media (storage_key)
    WHERE source_url IS NOT NULL AND stored_at IS NULL;
