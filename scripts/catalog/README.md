# Exercise catalog import

Generates `migrations/20261004000002_exercise_catalog_import.sql` from [hasaneyldrm/exercises-dataset](https://github.com/hasaneyldrm/exercises-dataset) at the commit pinned in `build.py`.

```bash
make catalog                                  # same as: uv run scripts/catalog/build.py
uv run scripts/catalog/build.py --list        # print every derived exercise instead of writing
uv run scripts/catalog/build.py --source f    # read a local exercises.json instead of downloading
```

Requirements: `uv`.
The first run downloads `exercises.json` (about 17 MB) into `scripts/catalog/.cache/` (gitignored).

## The migration is immutable once released

sqlx checks the checksum of applied migrations, so a generated file that has reached any shared database must not be regenerated.
The script exists so the mapping is reviewable and reproducible, and as the starting point for a follow-up migration.
To correct imported exercises later, write a new migration that addresses them by `catalog_ref` (`gymvisual:<dataset id>`), never by name: admins can rename catalog exercises.

## What the dataset has and what is derived

Taken as is: name, instructions in English and Polish (one step per line), the thumbnail and the animation.
Mapped through tables in `build.py`: target and secondary muscles to FitTune's 16 muscle groups, equipment to the 9 display categories.
The script stops on any value the tables do not know.

The dataset has no counterpart for three fields, so they are derived from the name and equipment:

| Field | Rule of thumb |
|---|---|
| `tracking` | weights are `weight_reps`, bodyweight and bands are `reps`, stretches, poses, planks and static holds are `duration`, running, cycling and cardio machines are `distance_duration` |
| `difficulty` | machines, cables, bands, dumbbells and stretches are `beginner`; barbells, kettlebells, added weight and pull-up or dip variations are `intermediate`; olympic lifts, planches, levers, muscle-ups and one-arm bodyweight work are `advanced` |
| `requires` | the equipment itself, plus what the name implies: a bench for incline, decline, lying and seated free-weight work, a rack for barbell squats and presses, a pull-up bar, a dip station, a stability ball and so on |

`difficulty` is the weakest of the three: it is a guess about a whole class of exercises, not a rating.

Every exercise that needs equipment requires at least one `EquipmentItem`, because an exercise that requires nothing is offered at a place with no equipment.
The six exceptions are stretches done with a strap or a towel, which the script reports on every run.
Machines without an item of their own require `strength_machines` or `cardio_machines`.

## Merging with the first catalog

`EXISTING` in `build.py` maps each of the 47 exercises FitTune started with to the dataset row showing the same movement, or to `None` when there is none (`Plank`, `Treadmill Run`, `Rowing Machine`).
A merged exercise keeps its id, name, tracking, muscles, difficulty and requirements, so workout history, routines, the app's routine templates and the dev fixtures are unaffected.
It gains `catalog_ref`, Polish instructions, English instructions if it had none, and the dataset's media in place of its start and finish photos.

The migration also merges a dataset row into an existing exercise when an admin already added a catalog exercise of the same name, or when the row's name-derived id is taken by an archived exercise, instead of failing on the unique name index.

## Media licence

The exercise data and instruction text are MIT licensed.
The thumbnails and animations are © Gym visual and are not covered by that licence: the dataset redistributes them with permission at 180x180, and reuse needs permission from Gym visual.
The migration stores the attribution on every media row, the API returns it, and the app shows it next to the media.
