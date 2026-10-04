# /// script
# requires-python = ">=3.11"
# ///
"""Generates the exercise catalog migration from hasaneyldrm/exercises-dataset.

The dataset has no tracking, difficulty or equipment requirements, and names its muscles and
equipment differently, so this script derives them. Everything it cannot map is an error instead
of a silent default. See README.md next to this file.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path(__file__).resolve().parent / ".cache"
OUTPUT = ROOT / "migrations" / "20261004000002_exercise_catalog_import.sql"

# Pinned so the migration and the media the backfill downloads never change underneath us
DATASET_COMMIT = "7455efae41b330c265e7cd4b78dfa848e7ce5ebd"
DATASET_RAW = f"https://raw.githubusercontent.com/hasaneyldrm/exercises-dataset/{DATASET_COMMIT}"
ATTRIBUTION = "© Gym visual - https://gymvisual.com/"
REF_PREFIX = "gymvisual:"

# Exercises seeded by 20260927000002 and the dataset row each one is merged with. A merged
# exercise keeps its id, name, tracking, muscles, difficulty and requirements, so history,
# routines and the app's routine templates are unaffected; it gains the dataset's instructions
# and media. None keeps the exercise as it is because the dataset has no real counterpart
EXISTING: dict[str, str | None] = {
    "Barbell Bench Press": "0025",
    "Incline Dumbbell Press": "0314",  # dumbbell incline bench press
    "Dumbbell Fly": "0308",
    "Cable Crossover": "0155",  # cable cross-over variation
    "Push-Up": "0662",
    "Chest Dip": "0251",
    "Barbell Back Squat": "0043",  # barbell full squat
    "Front Squat": "0042",
    "Leg Press": "0739",  # sled 45° leg press
    "Bulgarian Split Squat": "0410",  # dumbbell single leg split squat
    "Leg Extension": "0585",
    "Walking Lunge": "1460",
    "Conventional Deadlift": "0032",
    "Romanian Deadlift": "0085",
    "Lying Leg Curl": "0586",
    "Hip Thrust": "3562",  # barbell glute bridge two legs on bench
    "Kettlebell Swing": "0549",
    "Standing Calf Raise": "0605",
    "Pull-Up": "0652",
    "Chin-Up": "1326",
    "Lat Pulldown": "0198",  # cable pulldown
    "One-Arm Dumbbell Row": "0292",
    "Barbell Row": "0027",
    "Seated Cable Row": "0861",
    "Barbell Shrug": "0095",
    "Overhead Press": "1457",  # barbell standing wide military press
    "Seated Dumbbell Shoulder Press": "0405",
    "Lateral Raise": "0334",
    "Rear Delt Fly": "0378",
    "Face Pull": "0203",  # cable rear delt row (with rope)
    "Barbell Curl": "0031",
    "Hammer Curl": "0313",
    "Incline Dumbbell Curl": "0318",
    "Triceps Pushdown": "0201",
    "Skull Crusher": "0060",
    "Overhead Triceps Extension": "0430",  # dumbbell standing triceps extension
    "Close-Grip Bench Press": "0030",
    "Wrist Curl": "0369",  # dumbbell over bench wrist curl
    "Plank": None,
    "Hanging Leg Raise": "0472",
    "Cable Crunch": "0175",  # cable kneeling crunch
    "Ab Wheel Rollout": "0857",
    "Burpee": "1160",
    "Treadmill Run": None,
    "Rowing Machine": None,
    "Stationary Bike": "2138",
    "Jump Rope": "2612",
}

PRIMARY_MUSCLE = {
    "abs": "abs",
    "pectorals": "chest",
    "biceps": "biceps",
    "glutes": "glutes",
    "delts": "shoulders",
    "triceps": "triceps",
    "upper back": "upper_back",
    "lats": "lats",
    "calves": "calves",
    "quads": "quadriceps",
    "forearms": "forearms",
    "cardiovascular system": "cardio",
    "hamstrings": "hamstrings",
    "spine": "lower_back",
    "traps": "traps",
    "levator scapulae": "traps",
    # FitTune has no group for these; the nearest one that the muscle map can show
    "adductors": "glutes",
    "abductors": "glutes",
    "serratus anterior": "chest",
}

# None drops a muscle FitTune has no group for
SECONDARY_MUSCLE: dict[str, str | None] = {
    "shoulders": "shoulders",
    "deltoids": "shoulders",
    "rear deltoids": "shoulders",
    "rotator cuff": "shoulders",
    "forearms": "forearms",
    "wrists": "forearms",
    "wrist flexors": "forearms",
    "wrist extensors": "forearms",
    "grip muscles": "forearms",
    "hands": "forearms",
    "biceps": "biceps",
    "brachialis": "biceps",
    "triceps": "triceps",
    "quadriceps": "quadriceps",
    "hip flexors": "quadriceps",
    "hamstrings": "hamstrings",
    "glutes": "glutes",
    "groin": "glutes",
    "inner thighs": "glutes",
    "calves": "calves",
    "soleus": "calves",
    "core": "abs",
    "abdominals": "abs",
    "obliques": "abs",
    "lower abs": "abs",
    "chest": "chest",
    "upper chest": "chest",
    "lower back": "lower_back",
    "rhomboids": "upper_back",
    "upper back": "upper_back",
    "back": "upper_back",
    "trapezius": "traps",
    "traps": "traps",
    "latissimus dorsi": "lats",
    "lats": "lats",
    "ankles": None,
    "ankle stabilizers": None,
    "feet": None,
    "shins": None,
    "sternocleidomastoid": None,
}

# The category shown in the library filter
EQUIPMENT = {
    "body weight": "none",
    "assisted": "none",
    "dumbbell": "dumbbell",
    "cable": "cable",
    "barbell": "barbell",
    "ez barbell": "barbell",
    "olympic barbell": "barbell",
    "trap bar": "barbell",
    "leverage machine": "machine",
    "smith machine": "machine",
    "sled machine": "machine",
    "upper body ergometer": "machine",
    "skierg machine": "machine",
    "stationary bike": "machine",
    "elliptical machine": "machine",
    "stepmill machine": "machine",
    "band": "band",
    "resistance band": "band",
    "kettlebell": "kettlebell",
    "weighted": "plate",
    "stability ball": "other",
    "medicine ball": "other",
    "rope": "other",
    "roller": "other",
    "bosu ball": "other",
    "wheel roller": "other",
    "hammer": "other",
    "tire": "other",
}

# What an exercise needs before its name is looked at
BASE_REQUIRES = {
    "body weight": [],
    "assisted": [],
    "rope": [],
    "dumbbell": ["dumbbells"],
    "cable": ["cable_station"],
    "barbell": ["barbell"],
    "olympic barbell": ["barbell"],
    "ez barbell": ["ez_bar"],
    "trap bar": ["trap_bar"],
    "smith machine": ["smith_machine"],
    "upper body ergometer": ["cardio_machines"],
    "skierg machine": ["cardio_machines"],
    "elliptical machine": ["cardio_machines"],
    "stepmill machine": ["cardio_machines"],
    "stationary bike": ["stationary_bike"],
    "band": ["resistance_band"],
    "resistance band": ["resistance_band"],
    "kettlebell": ["kettlebells"],
    "weighted": ["weight_plates"],
    "stability ball": ["stability_ball"],
    "medicine ball": ["medicine_ball"],
    "roller": ["foam_roller"],
    "bosu ball": ["bosu_ball"],
    "wheel roller": ["ab_wheel"],
    "hammer": ["sledgehammer_tire"],
    "tire": ["sledgehammer_tire"],
}

# First match wins
LEVERAGE_MACHINES = [
    (r"assisted", "assisted_pull_up_machine"),
    (r"leg press|calf press", "leg_press"),
    (r"leg extension", "leg_extension"),
    (r"leg curl", "leg_curl"),
    (r"calf", "calf_raise_machine"),
    (r"chest press", "chest_press_machine"),
    (r"seated fly|reverse fly", "pec_deck"),
    (r"shoulder press|military press", "shoulder_press_machine"),
    (r"pulldown", "lat_pulldown"),
    (r"seated row", "seated_row"),
    (r"treadmill", "treadmill"),
    (r"bike", "stationary_bike"),
    (r"cross trainer", "cardio_machines"),
]

# Exercises without equipment of their own that are done with something a place has to have
FREE_WEIGHTS = {"barbell", "olympic barbell", "ez barbell", "trap bar", "dumbbell", "kettlebell"}
BENCH_USERS = FREE_WEIGHTS | {"cable", "smith machine", "band", "resistance band", "weighted", "medicine ball"}

WEIGHTED = FREE_WEIGHTS | {"cable", "leverage machine", "smith machine", "sled machine", "weighted", "medicine ball"}

ACRONYMS = {"ez": "EZ", "jm": "JM", "sz": "SZ", "pov": "POV", "ii": "II"}
SMALL_WORDS = {"a", "and", "at", "for", "from", "in", "of", "on", "the", "to", "with"}


@dataclass
class Exercise:
    ref: str
    name: str
    merge_into: str | None
    tracking: str
    primary_muscle: str
    secondary_muscles: list[str]
    equipment: str
    requires: list[str]
    difficulty: str
    instructions: str
    instructions_pl: str
    image: str
    gif: str


def equipment_items() -> list[str]:
    """Variants of the Rust EquipmentItem enum, in canonical order"""
    source = (ROOT / "src" / "equipment.rs").read_text()
    body = re.search(r"pub enum EquipmentItem \{(.*?)\n\}", source, re.S)
    if not body:
        sys.exit("EquipmentItem enum not found in src/equipment.rs")
    variants = re.findall(r"^\s+([A-Z][A-Za-z]+),", body.group(1), re.M)
    return [re.sub(r"(?<!^)(?=[A-Z])", "_", variant).lower() for variant in variants]


def has(pattern: str, name: str) -> bool:
    return re.search(pattern, name) is not None


def display_name(raw: str) -> str:
    """Title case in the style of the existing catalog, e.g. `One-Arm Dumbbell Row`"""
    # The dataset mangled the degree sign in some names
    text = raw.replace("в°", "°").replace("_", " ")
    text = re.sub(r"\s+", " ", text).strip()

    def word(match: re.Match[str]) -> str:
        token = match.group(0)
        if token in ACRONYMS:
            return ACRONYMS[token]
        if token in SMALL_WORDS and match.start() > 0:
            return token
        # `v. 2` marks a variation and reads as a version, not a word
        if token == "v" and text[match.end() : match.end() + 1] == ".":
            return token
        return token[:1].upper() + token[1:]

    return re.sub(r"[a-z]+", word, text)


def tracking(name: str, equipment: str, target: str) -> str:
    if has(r"jump rope|battling ropes", name):
        return "duration"
    if target == "cardiovascular system" and has(r"\b(run|walk|walking|bike|cycle|ergometer)\b|trainer|stepmill", name) and not has(r"lunge", name):
        return "distance_duration"
    if has(r"ski ergometer|hands bike", name):
        return "distance_duration"
    static = has(r"\bstretch|\bpose\b|\bhold\b|planche|front lever$|back lever$", name) and not has(r"push-up|lunge", name)
    plank = has(r"plank", name) and not has(r"push-up|\btap\b|twist|leg lift|adduction|fly", name)
    if static or plank or equipment == "roller" and not has(r"crunch|body saw", name):
        return "duration"
    if equipment == "rope" and not has(r"rope climb|london bridge", name):
        return "duration"
    return "weight_reps" if equipment in WEIGHTED else "reps"


def difficulty(name: str, equipment: str, tracked: str) -> str:
    """An approximation: the dataset does not rate its exercises"""
    if has(
        r"planche|muscle.up|one arm (chin-up|pull-up|pull up|dip\b|push-up|push up)|one hand pull|handstand"
        r"|front lever|back lever|pistol|snatch|\bclean\b(?!.grip)|impossible|korean|archer (pull|push)"
        r"|rope climb|london bridge|tire flip|sissy",
        name,
    ):
        return "advanced"
    if tracked == "duration" or equipment in {
        "leverage machine", "sled machine", "cable", "band", "resistance band", "assisted",
        "roller", "dumbbell", "stability ball", "medicine ball", "bosu ball", "stationary bike", "elliptical machine", "stepmill machine",
        "upper body ergometer", "skierg machine",
    }:  # fmt: skip
        return "beginner"
    if equipment == "body weight" and not has(r"pull-up|pull up|\bchin|\bdips?\b|hanging|inverted|pike|decline|one leg|l-", name):
        return "beginner"
    return "intermediate"


def requires(name: str, equipment: str) -> set[str]:
    items: set[str] = set()
    if equipment == "leverage machine":
        items.add(next((item for pattern, item in LEVERAGE_MACHINES if has(pattern, name)), "strength_machines"))
    elif equipment == "sled machine":
        items.add("strength_machines" if has(r"hack|squat", name) and not has(r"leg press", name) else "leg_press")
    elif equipment == "rope":
        if has(r"battling", name):
            items.add("battle_ropes")
        elif has(r"jump rope", name):
            items.add("jump_rope")
        elif has(r"rope climb|london bridge", name):
            items.add("climbing_rope")
    else:
        items.update(BASE_REQUIRES[equipment])

    if equipment == "cable":
        if has(r"pulldown", name) and not has(r"straight arm|standing|kneeling|cross-over", name):
            items = {"lat_pulldown"}
        elif has(r"seated.*row", name):
            items = {"seated_row"}

    machine = equipment in {"leverage machine", "sled machine"}
    if has(r"exercise ball|stability ball", name):
        items.add("stability_ball")
    if has(r"bosu", name):
        items.add("bosu_ball")
    if has(r"medicine ball", name):
        items.add("medicine_ball")
    if has(r"wheel roll", name):
        items.add("ab_wheel")
    if has(r"suspended|suspension|with straps|\bring\b", name):
        items.add("suspension_trainer")
    if has(r"\bbox\b|step-up|step up", name):
        items.add("plyo_box")
    if has(r"pre?acher", name) and not machine:
        items.add("preacher_bench")
    if has(r"hyperextension", name) and equipment == "body weight" and not has(r"bench", name):
        items.add("back_extension_bench")
    if not machine:
        if has(r"pull-up|pull up|pull-ups|\bchin|hanging|muscle.up|front lever|back lever|arm slingers", name) and not has(
            r"bench pull-ups|pull-up cable machine|pulldown", name
        ):
            items.add("pull_up_bar")
        if has(r"inverted row", name) and "suspension_trainer" not in items:
            items.update({"barbell", "squat_rack"})
        if has(r"\bdips?\b", name) and not has(r"bench|ring dips|exercise ball|elbow dips|scapula|floor", name):
            items.add("dip_station")
        if has(r"parallel bars|captains chair|dip cage", name):
            items.add("dip_station")

    if equipment in BENCH_USERS or equipment == "body weight":
        push_up = has(r"push.up|plank|bridge", name)
        if has(r"incline|decline", name) and not push_up and not has(r"decline bridge", name):
            items.add("adjustable_bench")
        elif has(r"bench|spider curl", name) and not has(r"exercise ball", name):
            items.add("flat_bench")
        elif equipment in BENCH_USERS - {"band", "resistance band", "medicine ball"}:
            floor = has(r"floor|exercise ball|stability ball|preacher|peacher|twist|leg raise|crunch|sit-up", name)
            if has(r"\blying\b|\bseated\b", name) and not floor and equipment != "cable":
                items.add("flat_bench")
    if equipment in {"barbell", "olympic barbell"}:
        if has(r"squat|bench press|standing.*military|good morning", name) and not has(r"hack squat|zercher|jump", name):
            items.add("squat_rack")
    return items


def muscles(row: dict) -> tuple[str, list[str]]:
    target = row["target"]
    if target not in PRIMARY_MUSCLE:
        sys.exit(f"{row['id']}: unknown target muscle {target!r}")
    primary = PRIMARY_MUSCLE[target]
    secondary: list[str] = []
    for source in [row["muscle_group"], *row["secondary_muscles"]]:
        if source not in SECONDARY_MUSCLE:
            sys.exit(f"{row['id']}: unknown secondary muscle {source!r}")
        muscle = SECONDARY_MUSCLE[source]
        if muscle and muscle != primary and muscle not in secondary:
            secondary.append(muscle)
    return primary, secondary


def steps(row: dict, language: str) -> str:
    lines = [step.strip() for step in row["instruction_steps"].get(language, []) if step.strip()]
    text = "\n".join(lines) or row["instructions"].get(language, "").strip()
    if not text:
        sys.exit(f"{row['id']}: no {language} instructions")
    return text


def load(source: Path | None) -> list[dict]:
    if source is None:
        source = CACHE / f"exercises-{DATASET_COMMIT[:12]}.json"
        if not source.exists():
            CACHE.mkdir(exist_ok=True)
            with urllib.request.urlopen(f"{DATASET_RAW}/data/exercises.json", timeout=60) as response:
                source.write_bytes(response.read())
    return json.loads(source.read_text())


def build(rows: list[dict]) -> list[Exercise]:
    items = equipment_items()
    by_id = {row["id"]: row for row in rows}
    if len(by_id) != len(rows):
        sys.exit("duplicate dataset ids")
    merges = {dataset_id: name for name, dataset_id in EXISTING.items() if dataset_id}
    if len(merges) != sum(1 for dataset_id in EXISTING.values() if dataset_id):
        sys.exit("two existing exercises merge with the same dataset row")
    for dataset_id in merges:
        if dataset_id not in by_id:
            sys.exit(f"EXISTING refers to unknown dataset id {dataset_id}")

    # Names must be unique among active catalog exercises, including the ones that stay
    taken = Counter(name.lower() for name in EXISTING)
    exercises = []
    for row in sorted(rows, key=lambda row: row["id"]):
        raw, equipment = row["name"].strip().lower(), row["equipment"]
        if equipment not in EQUIPMENT:
            sys.exit(f"{row['id']}: unknown equipment {equipment!r}")
        merge_into = merges.get(row["id"])
        name = merge_into or display_name(raw)
        if merge_into is None:
            taken[name.lower()] += 1
            if taken[name.lower()] > 1:
                name = f"{name} ({taken[name.lower()]})"
                taken[name.lower()] += 1
        tracked = tracking(raw, equipment, row["target"])
        primary, secondary = muscles(row)
        needs = requires(raw, equipment)
        unknown = needs - set(items)
        if unknown:
            sys.exit(f"{row['id']}: {sorted(unknown)} are not EquipmentItem variants")
        exercises.append(
            Exercise(
                ref=REF_PREFIX + row["id"],
                name=name,
                merge_into=merge_into,
                tracking=tracked,
                primary_muscle=primary,
                secondary_muscles=secondary,
                equipment=EQUIPMENT[equipment],
                requires=sorted(needs, key=items.index),
                difficulty=difficulty(raw, equipment, tracked),
                instructions=steps(row, "en"),
                instructions_pl=steps(row, "pl"),
                image=row["image"],
                gif=row["gif_url"],
            )
        )
        if not has(r"^images/[\w-]+\.jpg$", row["image"]) or not has(r"^videos/[\w-]+\.gif$", row["gif_url"]):
            sys.exit(f"{row['id']}: unexpected media paths")

    names = Counter(exercise.name.lower() for exercise in exercises)
    duplicates = [name for name, count in names.items() if count > 1]
    if duplicates:
        sys.exit(f"duplicate names after formatting: {duplicates}")
    return exercises


def quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def array(values: list[str]) -> str:
    return "ARRAY[" + ", ".join(quote(value) for value in values) + "]::TEXT[]"


def render(exercises: list[Exercise]) -> str:
    values = ",\n".join(
        "    ("
        + ", ".join(
            [
                quote(e.ref),
                quote(e.name),
                quote(e.merge_into) if e.merge_into else "NULL",
                quote(e.tracking),
                quote(e.primary_muscle),
                array(e.secondary_muscles),
                quote(e.equipment),
                array(e.requires),
                quote(e.difficulty),
                quote(e.instructions),
                quote(e.instructions_pl),
                quote(e.image),
                quote(e.gif),
            ]
        )
        + ")"
        for e in exercises
    )
    return f"""-- Generated by scripts/catalog/build.py from hasaneyldrm/exercises-dataset@{DATASET_COMMIT[:12]}.
-- Do not edit: change the script and add a new migration instead.
--
-- Exercise data and instructions are MIT licensed. The media are (c) Gym visual and only
-- referenced here; the API copies them into object storage on start and serves them with the
-- attribution stored on each row

CREATE TEMP TABLE catalog_import (
    ref                TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    merge_into         TEXT,
    tracking           TEXT NOT NULL,
    primary_muscle     TEXT NOT NULL,
    secondary_muscles  TEXT[] NOT NULL,
    equipment          TEXT NOT NULL,
    requires           TEXT[] NOT NULL,
    difficulty         TEXT NOT NULL,
    instructions       TEXT NOT NULL,
    instructions_pl    TEXT NOT NULL,
    image              TEXT NOT NULL,
    gif                TEXT NOT NULL,
    exercise_id        UUID
) ON COMMIT DROP;

INSERT INTO catalog_import (ref, name, merge_into, tracking, primary_muscle, secondary_muscles,
                            equipment, requires, difficulty, instructions, instructions_pl,
                            image, gif)
VALUES
{values};

-- Rows merge with an existing exercise when they are mapped to one of the first catalog's
-- exercises (by its name-derived id, so a renamed or archived one still matches), when their
-- own name-derived id is already taken, or when an admin added a catalog exercise of that name
UPDATE catalog_import i SET exercise_id = e.id
FROM exercises e
WHERE e.id = md5('fittune:exercise:' || lower(COALESCE(i.merge_into, i.name)))::uuid;

UPDATE catalog_import i SET exercise_id = e.id
FROM exercises e
WHERE i.exercise_id IS NULL AND e.owner_id IS NULL AND e.archived_at IS NULL
  AND lower(e.name) = lower(i.name);

-- A merged exercise keeps everything it has and gains the source reference and instructions
UPDATE exercises e SET catalog_ref = i.ref,
                       instructions = COALESCE(e.instructions, i.instructions),
                       instructions_pl = i.instructions_pl,
                       updated_at = now()
FROM catalog_import i
WHERE e.id = i.exercise_id;

INSERT INTO exercises (id, owner_id, name, tracking, primary_muscle, secondary_muscles, equipment,
                       requires, difficulty, instructions, instructions_pl, catalog_ref)
SELECT md5('fittune:exercise:' || lower(name))::uuid, NULL, name, tracking, primary_muscle,
       secondary_muscles, equipment, requires, difficulty, instructions, instructions_pl, ref
FROM catalog_import
WHERE exercise_id IS NULL;

-- Every catalog exercise shows the same kind of demo, so the start and finish photos of merged
-- exercises make way. Their files stay in object storage, unreferenced
DELETE FROM exercise_media m
USING exercises e
WHERE m.exercise_id = e.id AND e.catalog_ref IS NOT NULL AND m.kind = 'photo';

-- A 180x180 thumbnail for lists and the animated demo. Ids are derived from the source row so
-- they are stable across environments, and the storage key contains the id so a replaced file
-- gets a new URL
INSERT INTO exercise_media (id, exercise_id, kind, provider, storage_key, source_url, position,
                            attribution)
SELECT ids.media_id, e.id, media.kind, 'fittune',
       'catalog/' || ids.media_id || media.extension,
       '{DATASET_RAW}/' || media.path, 0, {quote(ATTRIBUTION)}
FROM catalog_import i
JOIN exercises e ON e.catalog_ref = i.ref
CROSS JOIN LATERAL (VALUES ('photo', i.image, '.jpg'), ('animation', i.gif, '.gif'))
    AS media (kind, path, extension)
CROSS JOIN LATERAL (
    SELECT md5('fittune:exercise-media:' || i.ref || ':' || media.kind || ':0')::uuid AS media_id
) AS ids;
"""


def report(exercises: list[Exercise]) -> None:
    new = [e for e in exercises if e.merge_into is None]
    print(f"{len(exercises)} exercises: {len(new)} new, {len(exercises) - len(new)} merged")
    for label, counter in [
        ("tracking", Counter(e.tracking for e in new)),
        ("difficulty", Counter(e.difficulty for e in new)),
        ("primary muscle", Counter(e.primary_muscle for e in new)),
        ("equipment", Counter(e.equipment for e in new)),
        ("requires", Counter(item for e in new for item in e.requires)),
    ]:
        print(f"{label}: " + ", ".join(f"{key} {count}" for key, count in counter.most_common()))
    unrestricted = [e.name for e in new if e.equipment != "none" and not e.requires]
    print(f"need equipment but require nothing ({len(unrestricted)}): " + "; ".join(unrestricted))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, help="exercises.json to read instead of downloading the pinned one")
    parser.add_argument("--list", action="store_true", help="print every derived exercise instead of writing the migration")
    args = parser.parse_args()

    exercises = build(load(args.source))
    if args.list:
        for e in exercises:
            print(f"{e.ref}\t{e.name}\t{e.tracking}\t{e.difficulty}\t{e.equipment}\t{','.join(e.requires)}")
        return
    OUTPUT.write_text(render(exercises))
    report(exercises)
    print(f"wrote {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
