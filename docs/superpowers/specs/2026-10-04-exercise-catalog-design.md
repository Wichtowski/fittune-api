# Exercise catalog from exercises-dataset

## Goal

Replace the 47 hand-seeded catalog exercises and their start and finish photos with the 1,324 exercises of [hasaneyldrm/exercises-dataset](https://github.com/hasaneyldrm/exercises-dataset), which come with an animated demo, a thumbnail and step-by-step instructions in English and Polish.
Nothing that refers to the existing catalog may break: workout history, routines, the app's routine templates and the dev fixtures.

## Decisions

- **Merge, not replace.**
  44 of the 47 existing exercises are mapped by hand to the dataset row showing the same movement and keep their id, name and curated fields.
  The other 1,280 rows are added.
  `Plank`, `Treadmill Run` and `Rowing Machine` have no counterpart and stay as they are.
- **Generated migration.**
  `scripts/catalog/build.py` writes `20261004000002_exercise_catalog_import.sql` from the dataset at a pinned commit.
  The catalog stays where it was (in migrations, with name-derived ids), production needs no extra step, and every test database has the real catalog.
  An import command or a startup sync were rejected: both make the catalog something that may or may not be there after `migrate`.
- **English and Polish only.**
  The dataset has ten languages; the app has two.
  `instructions` stays the English string so app builds that are already installed keep parsing responses, and `instructions_pl` is added next to it.
- **Media are used with attribution.**
  The dataset's MIT licence does not cover its images and GIFs, which are © Gym visual.
  Using them was decided by the project owner, who handles permission.
  The attribution is stored per media row and returned by the API so no client hardcodes it.

## Schema (`20261004000001`)

- `exercises.catalog_ref TEXT UNIQUE` (`gymvisual:<dataset id>`) and `exercises.instructions_pl TEXT`.
- `exercise_media.kind` gains `animation` (a GIF, stored like a photo), and the table gains `attribution` and `stored_at`.
- `equipment_items()` grows from 27 to 42 items, and existing places are extended: gyms get what a commercial gym has, and every place with a barbell gets `weight_plates`.
  Without that, the new exercises would be hidden at every existing place.

## Derived fields

The dataset has no tracking, difficulty or per-item requirements.
`build.py` derives them from the name and equipment; the rules and their limits are in `scripts/catalog/README.md`.
`difficulty` is kept non-null because making it nullable would break response parsing in installed app builds.
Muscles map onto the existing 16 groups, so the app's anatomy pipeline is untouched; adductors and abductors map to `glutes` and serratus anterior to `chest` for lack of a closer group.

## Media

Every imported exercise gets a `photo` (180x180 JPEG) and an `animation` (180x180 GIF) at position 0, with a `source_url` on `raw.githubusercontent.com` pinned to the dataset commit.
Merged exercises lose their Free Exercise DB photos so the whole catalog looks the same; their YouTube and Vimeo videos stay.

The startup backfill now handles 2,654 files instead of 86:

- it only lists rows with `stored_at IS NULL`, so a restart does not issue thousands of `head_object` calls
- it copies four files at a time
- a photo must decode as JPEG and an animation as GIF before it is stored
- the file endpoint serves a row only once it is marked stored, and unmarks a row whose file the bucket lost, so the next start copies it again

## API

- `Exercise` gains `instructions_pl`; `ExerciseMedia` gains kind `animation` and `attribution`.
- `media` is ordered photos, animations, videos.
- `GET /train/exercise-media/{id}/file` serves `image/jpeg` or `image/gif`.
- `GET /train/exercises` returns `null` for both instruction texts.
  The app persists the whole library in `localStorage` for offline use, and with the texts it is 2.4 million characters, close to Safari's quota on its own.
  Without them it is 1.2 million, and the exercise screen already loads its exercise through the history endpoint.
- Responses are gzip-compressed.

## Rollout

1. App release first.
   Installed builds validate `media[].kind` and `requires[]` against closed enums, so they must learn `animation` and the new equipment items before the API returns them.
2. API release: migrations run on start, then the backfill downloads about 138 MB into object storage.
3. The app's CSP already allows images from the API origin.

## Not done

- No new muscle groups (adductors, abductors, serratus).
- No pagination of the library; it stays one compressed response.
- The app's query cache stays in `localStorage`; moving it to IndexedDB is the next step if the library grows further.
- The unreferenced Free Exercise DB photos of merged exercises are left in object storage.
