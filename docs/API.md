# FitTune API v1

Base URL: `https://api-fittune.oskarwichtowski.com` in production, `http://localhost:4733` locally.
All endpoints except `/health` and `/api/v1/auth/{register,login}` require
`Authorization: Bearer <token>`.

## Namespaces

- Account and social endpoints are shared by both apps and live directly under `/api/v1`: `auth`, `me`, `users`, `admin/invites`, `friends`, `blocks`
- FitTune (training) endpoints live under `/api/v1/train`
- FitHealth (nutrition) endpoints live under `/api/v1/health`

## Conventions

- JSON in and out. Timestamps are RFC 3339 UTC (`2026-09-27T17:00:00Z`), dates are `YYYY-MM-DD`.
- Weights are always **kilograms**, distances **metres**, durations **seconds**. `weight_unit` /
  `distance_unit` on the user are display preferences only.
- Enum values are `snake_case` strings.
- Ids are UUIDs. Workouts and activities use **client-generated** ids and are written with `PUT`,
  so a request can be retried or replayed safely (e.g. after a dropped gym connection).
- List endpoints that can grow without bound are keyset-paginated:
  `?limit=20&cursor=<next_cursor>` → `{ "items": [...], "next_cursor": "..." | null }`.
- Every response carries an `x-request-id` header (echoed if the client sends one).

### Errors

```json
{ "code": "validation_failed", "message": "Password must contain at least one uppercase letter",
  "fields": { "password": "Password must contain at least one uppercase letter" } }
```

| Status | `code`                | When                                                        |
|--------|-----------------------|-------------------------------------------------------------|
| 400    | `bad_request`         | Malformed JSON, unknown enum value, bad query/path parameter |
| 401    | `unauthorized`        | Missing, invalid or expired token                           |
| 401    | `invalid_credentials` | Wrong login or password                                     |
| 403    | `forbidden`           | Authenticated but not allowed (admin-only actions, a friend's unshared progress) |
| 404    | `not_found`           | Resource does not exist **or belongs to another user**      |
| 409    | `conflict`            | Stale workout revision, id collision, friend request to a user you blocked |
| 422    | `validation_failed`   | Semantically invalid input; `fields` maps field paths to messages (`exercises[0].sets[1].reps`) |
| 429    | `rate_limited`        | Too many wrong invite codes from this client, or too many friend lookups and requests; try again later |
| 500    | `internal_error`      | Unexpected server error (details are logged, not returned)  |
| 413    | `payload_too_large`   | Photo upload exceeds 10 MB |
| 503    | `storage_unavailable` | Private photo storage is not configured |

## Private progress photos

Photos are private to their owner. The client saves a completed workout independently, then may
attach a photo. `PUT` with a client-generated UUID makes retries safe. Both `PUT` and `POST`
accept `multipart/form-data` with one `file` (JPEG, PNG, or WebP, at most 10 MB) and optional
`workout_id` of a completed workout owned by the caller. The API re-encodes the image and strips
metadata before storing the full-size image and thumbnail in RustFS. Images are returned only
through authenticated API endpoints with `Cache-Control: private, no-store`.

| Method | Path | Response |
|--------|------|----------|
| POST | `/api/v1/train/progress-photos` | `201 ProgressPhoto` |
| PUT | `/api/v1/train/progress-photos/{id}` | `201 ProgressPhoto`; replay returns `200` |
| GET | `/api/v1/train/progress-photos?workout_id=&limit=50&offset=0` | `ProgressPhoto[]`, newest first |
| GET | `/api/v1/train/progress-photos/{id}/file?size=full\|thumb` | JPEG image |
| DELETE | `/api/v1/train/progress-photos/{id}` | `204`; deletes both stored images |

`ProgressPhoto`: `{ id, workout_id, width, height, bytes, taken_at, created_at }`.
Deleting an account removes its stored images. Deleting a workout keeps its photo in the user's
gallery and clears the photo's `workout_id`.

## Health

`GET /health` → `200 { "status": "ok", "version": "1.4.2", "database": "ok" }`, or `503` with
`"status": "degraded"` when Postgres is unreachable.

## Auth

Sessions are opaque bearer tokens (not JWTs). They expire after `FITTUNE_SESSION_TTL_HOURS`
(default 30 days) of inactivity; use extends them automatically.

| Method | Path                    | Body                                                               | Response |
|--------|-------------------------|--------------------------------------------------------------------|----------|
| POST   | `/api/v1/auth/register` | `{ username, email, password, display_name?, birthday?, invite_code }` | `201 AuthResponse` |
| POST   | `/api/v1/auth/login`    | `{ login, password }` — `login` is the username **or** email       | `200 AuthResponse` |
| POST   | `/api/v1/auth/logout`   | —                                                                  | `204`, revokes the current token |

`AuthResponse`: `{ "user": User, "session": { "token": "…", "expires_at": "…" } }`

Registration rules: username 3–32 chars of letters, digits, `.`, `_`, `-`; valid email; password
at least 8 characters with an uppercase letter and a special character, not containing the
username; birthday not in the future. Username and email are unique case-insensitively.

Registration is invite-only unless the server runs with `FITTUNE_REGISTRATION=open`.
Without `invite_code` the request fails with `422` and an `invite_code` field error, alongside any other field errors.
A code that does not exist, has expired, was revoked or is used up fails the same way with one generic message, so codes cannot be probed.
Codes ignore case, dashes and spaces.
The code is checked before the account is created, in the same transaction, so a signup that fails for another reason (for example a taken username) does not use it up.
After 10 wrong codes from one client within 15 minutes, registration from that client returns `429` until the window passes.
New accounts are always ordinary users; a `role` in the body is ignored.

## Invites (admin only)

| Method | Path                            | Body | Response |
|--------|---------------------------------|------|----------|
| POST   | `/api/v1/admin/invites`         | `{ note?, expires_in_days?, max_uses? }` | `201 { invite: Invite, code }` |
| GET    | `/api/v1/admin/invites`         | none | `Invite[]`, newest first, at most 200 |
| DELETE | `/api/v1/admin/invites/{id}`    | none | `204`, revokes the invite (idempotent) |

```json
Invite {
  "id": "uuid", "note": "for Ala" | null,
  "status": "active" | "used" | "expired" | "revoked",
  "max_uses": 1, "use_count": 0,
  "expires_at": "…", "revoked_at": "…" | null,
  "created_by": "username" | null, "created_at": "…"
}
```

`expires_in_days` is 1-90 (default 7) and `max_uses` is 1-50 (default 1); `note` is up to 120 characters.
`code` looks like `a1b2-c3d4-e5f6-g7h8-j9k0-mnpq-rs` and is returned only by `POST`; only its SHA-256 digest is stored.

## Profile

| Method | Path                  | Body | Response |
|--------|-----------------------|------|----------|
| GET    | `/api/v1/me`          | — | `User` |
| PATCH  | `/api/v1/me`          | any of `display_name`, `birthday`, `account_type` (nullable), `weight_unit`, `distance_unit` | `User` |
| POST   | `/api/v1/me/password` | `{ current_password, new_password }` | `204`; signs out all other sessions |
| DELETE | `/api/v1/me`          | `{ password }` | `204`; deletes the account and all data |
| GET    | `/api/v1/users?limit&offset` | — | `User[]` — **admin only** |

```json
User {
  "id": "uuid", "username": "oskyy", "email": "oskar@example.com",
  "display_name": "Oskar" | null, "birthday": "2002-07-02" | null,
  "role": "user" | "admin",
  "account_type": "gym_enthusiast" | "professional_trainer" | "nutritionist" | "psychologist" | "physical_therapist" | null,
  "weight_unit": "kg" | "lb", "distance_unit": "km" | "mi",
  "created_at": "…"
}
```

## Exercises

A shared catalog (seeded, admin-managed) plus each user's private custom exercises.

| Method | Path | Notes |
|--------|------|-------|
| GET    | `/api/v1/train/exercises?q=&muscle=&equipment=` | Active catalog + own exercises, sorted by name. `muscle` matches primary or secondary. `instructions` and `instructions_pl` are always `null` here. |
| GET    | `/api/v1/train/exercises/{id}` | Also returns archived exercises (history still references them). |
| POST   | `/api/v1/train/exercises` | `ExerciseInput`; `"global": true` adds to the catalog (admin only). `201` |
| PUT    | `/api/v1/train/exercises/{id}` | Full replace. Owners edit custom exercises; admins edit the catalog. |
| DELETE | `/api/v1/train/exercises/{id}` | Archives (hides from the library, keeps history). `204` |
| GET    | `/api/v1/train/exercises/{id}/history?sessions=30` | Per-session breakdown and all-time records. |
| GET    | `/api/v1/train/exercise-media/{id}/file` | Catalog photo as JPEG or animation as GIF. **No auth**, `Cache-Control: public, max-age=31536000, immutable`. `404` for videos, custom exercises' media and files not stored yet. |

```json
ExerciseInput {
  "name": "Sled Push",
  "tracking": "weight_reps" | "reps" | "duration" | "distance_duration",
  "primary_muscle": Muscle, "secondary_muscles": [Muscle],
  "equipment": "none" | "barbell" | "dumbbell" | "kettlebell" | "machine" | "cable" | "band" | "plate" | "other",
  "requires": [EquipmentItem],                // everything the exercise needs; [] for bodyweight
  "difficulty": "beginner" | "intermediate" | "advanced",
  "video_id": "dQw4w9WgXcQ" | null,           // YouTube id; stored as the exercise's first video
  "instructions": "…" | null
}
EquipmentItem = "barbell" | "ez_bar" | "trap_bar" | "dumbbells" | "kettlebells" | "weight_plates"
              | "flat_bench" | "adjustable_bench" | "preacher_bench" | "back_extension_bench"
              | "squat_rack" | "pull_up_bar" | "dip_station"
              | "leg_press" | "leg_extension" | "leg_curl" | "calf_raise_machine" | "smith_machine"
              | "chest_press_machine" | "pec_deck" | "shoulder_press_machine" | "assisted_pull_up_machine"
              | "strength_machines"
              | "cable_station" | "lat_pulldown" | "seated_row"
              | "treadmill" | "rowing_machine" | "stationary_bike" | "cardio_machines"
              | "resistance_band" | "suspension_trainer" | "stability_ball" | "bosu_ball" | "medicine_ball"
              | "foam_roller" | "plyo_box" | "ab_wheel" | "jump_rope" | "battle_ropes" | "climbing_rope"
              | "sledgehammer_tire"
Muscle = "chest" | "lats" | "upper_back" | "lower_back" | "traps" | "shoulders" | "biceps" | "triceps"
       | "forearms" | "abs" | "quadriceps" | "hamstrings" | "glutes" | "calves" | "full_body" | "cardio"
Exercise = ExerciseInput + { id, instructions_pl, is_custom, archived_at, created_at, updated_at, media: [ExerciseMedia] }
ExerciseMedia = { "id", "kind": "photo" | "animation", "provider": "fittune", "position": 0,
                  "url": "/api/v1/train/exercise-media/{id}/file", "attribution": "© Gym visual - https://gymvisual.com/" }
              | { "id", "kind": "video", "provider": "youtube" | "vimeo", "position": 0, "external_id": "hWbUlkb5Ms4" }
```

`strength_machines` and `cardio_machines` stand for any machine without an item of its own, such as a hack squat or an elliptical.

The catalog is the 1,324 exercises of [hasaneyldrm/exercises-dataset](https://github.com/hasaneyldrm/exercises-dataset) merged into the 47 exercises FitTune started with.
`instructions` of a catalog exercise is English with one step per line, and `instructions_pl` is the Polish translation; custom exercises only have `instructions`.
The dataset does not rate difficulty or say how a set is tracked, so `difficulty`, `tracking` and `requires` of imported exercises are derived from their name and equipment (see `scripts/catalog/README.md`).

`media` lists photos, then animations, then videos, each by `position`.
An imported exercise has one photo (a 180x180 thumbnail for lists) and one animation (a 180x180 GIF of the movement); the three first-catalog exercises without a dataset counterpart keep their start (`0`) and finish (`1`) photos.
A photo's or animation's `url` is relative to the API origin and works in a plain `<img src>`.
`attribution` must be shown wherever the file is: the dataset's media are © Gym visual and not covered by the dataset's MIT licence.
In an `Exercise`, `video_id` is the first YouTube video in `media` (a Vimeo video leaves it `null`); it stays for clients that predate `media`.
Catalog media are copied from their source into object storage in the background on API start, four at a time, so right after a fresh deploy a file can return `404` for a few minutes.
The list leaves out both instruction texts because clients keep the whole library for offline use and the texts would double its size; read them from `GET /exercises/{id}` or the history.
A client must therefore load one exercise before sending it back with `PUT`, or it would erase its instructions.
Responses are gzip-compressed when the client accepts it; the library is about 1.2 MB of JSON before compression.

`equipment` is a display category used for badges and the `equipment=` filter.
`requires` is what matches exercises to places: an exercise can be done at a place when every item it requires is in the place's `equipment`.
`requires` is deduplicated and stored in the order `EquipmentItem` is listed above; `PUT` replaces it like every other field.

`ExerciseHistory`:

```json
{
  "exercise": Exercise,
  "records": {
    "max_weight_kg":          { "value": 110.0, "workout_id": "…", "achieved_at": "…" } | null,
    "best_e1rm_kg":           Record | null,     // Epley, sets of 1–12 reps
    "max_reps":               Record | null,
    "best_session_volume_kg": Record | null,
    "max_duration_seconds":   Record | null,
    "max_distance_m":         Record | null
  },
  "sessions": [{
    "workout_id": "…", "workout_title": "Push Day", "started_at": "…",
    "sets": [{ "kind", "reps", "weight_kg", "duration_seconds", "distance_m", "rpe", "e1rm_kg" }],
    "working_sets": 3, "total_reps": 15, "volume_kg": 1500.0,
    "max_weight_kg": 100.0, "best_e1rm_kg": 116.7, "max_duration_seconds": null, "max_distance_m": null
  }]
}
```

Only completed sets in finished workouts count; warm-up sets are listed but never count towards
metrics or records.

## Workout places

Places belong to the authenticated user and list the `EquipmentItem`s available there.
Home and Gym suggestions are editable client presets; `custom` supports any named setup, and users may have multiple places of the same type.

| Method | Path | Notes |
|--------|------|-------|
| GET | `/api/v1/train/places` | `Place[]`, active places sorted by name |
| PUT | `/api/v1/train/places/{id}` | Client-generated UUID; `PlaceInput` → `201` created / `200` updated, returns `Place` |
| DELETE | `/api/v1/train/places/{id}` | `204`; archives the place without changing workout history |

```json
PlaceInput {
  "name": "Home",
  "kind": "home" | "gym" | "custom",
  "equipment": ["dumbbells", "adjustable_bench", "resistance_band"]
}
Place = PlaceInput + { "id": "uuid", "version_id": "uuid" }
```

Names are trimmed and limited to 1-80 characters.
Equipment accepts any `EquipmentItem`s; bodyweight is always available, so an empty list means bodyweight-only.
Duplicate equipment entries are removed and the list is stored in the `EquipmentItem` order.
Saving changed details creates an immutable version; replaying the current values, in any order, returns the current version.
A `PUT` to a new id creates the place; a `PUT` to an archived place or to another user's place returns `404`.
A user can keep at most 10 active places; creating another returns `409 conflict` until one is archived.
Archived places cannot be edited or listed, but their existing versions remain valid for delayed offline workout uploads by their owner.

## Workouts

A workout is written as one document. The client owns the ids of the workout, its exercises and
sets, and increments `revision` on every local edit. The server stores a write only if its
revision is **greater than or equal to** the stored one (equal = idempotent replay); an older
revision gets `409 conflict`. A workout is *in progress* while `ended_at` is `null`.

| Method | Path | Notes |
|--------|------|-------|
| GET    | `/api/v1/train/workouts?status=in_progress\|completed&limit&cursor` | `Page<WorkoutSummary>`, newest first |
| GET    | `/api/v1/train/workouts/{id}` | `Workout` |
| PUT    | `/api/v1/train/workouts/{id}` | `WorkoutInput` → `201` created / `200` replaced, returns `Workout` |
| DELETE | `/api/v1/train/workouts/{id}` | `204` |

```json
WorkoutInput {
  "title": "Push Day", "notes": null,
  "routine_id": "uuid" | null,               // silently dropped if the routine no longer exists
  "place_version_id": "uuid" | null,         // optional; selects an immutable owned place version
  "started_at": "…", "ended_at": "…" | null,
  "revision": 12,
  "exercises": [{
    "id": "uuid", "exercise_id": "uuid", "notes": null, "rest_seconds": 120,
    "sets": [{
      "id": "uuid", "kind": "warmup" | "normal" | "drop" | "failure",
      "reps": 5, "weight_kg": 100.0, "duration_seconds": null, "distance_m": null,
      "rpe": 8.0, "completed": true
    }]
  }]
}
Workout = WorkoutInput + { id, created_at, updated_at }, and each exercise adds
          { exercise_name, tracking, primary_muscle }
          The response replaces place_version_id with place: Place | null
WorkoutSummary { id, routine_id, title, started_at, ended_at, duration_seconds, exercise_count,
                 set_count, total_reps, volume_kg, exercise_names, place: Place | null }
```

Omitting `place_version_id` preserves an existing selection, so older clients remain compatible; explicit `null` clears it.
Unknown or foreign versions return `422` with a `place_version_id` field error.
Workout responses and history retain the selected version's name, type and equipment after a place is edited or archived.
Equipment is a picker filter, not a restriction on saving exercises.

Limits: 60 exercises, 60 sets per exercise, workouts up to 24 h, reps 0–1000, weight 0–1000 kg,
RPE 1–10. Order in the arrays is the display order.

## Routines

Reusable plans with per-set targets; a workout started from a routine carries its `routine_id`.

| Method | Path | Notes |
|--------|------|-------|
| GET    | `/api/v1/train/routines` | `Routine[]` sorted by name |
| GET    | `/api/v1/train/routines/{id}` | `Routine` |
| POST   | `/api/v1/train/routines` | `RoutineInput` → `201 Routine` |
| PUT    | `/api/v1/train/routines/{id}` | Full replace |
| DELETE | `/api/v1/train/routines/{id}` | `204`; past workouts keep their data |

```json
RoutineInput {
  "name": "Leg Day", "notes": null,
  "exercises": [{ "exercise_id": "uuid", "rest_seconds": 180, "notes": null,
                  "sets": [{ "kind": "normal", "reps": 5, "weight_kg": 120, "duration_seconds": null, "distance_m": null }] }]
}
Routine = RoutineInput + { id, last_performed_at, created_at, updated_at }, and each exercise adds
          { id, exercise_name, tracking, primary_muscle }
```

## Activities

Endurance sessions (Strava-style). Client-generated ids, `PUT` upsert.

| Method | Path | Notes |
|--------|------|-------|
| GET    | `/api/v1/train/activities?kind=&limit&cursor` | `Page<Activity>`, newest first |
| GET    | `/api/v1/train/activities/{id}` | `Activity` |
| PUT    | `/api/v1/train/activities/{id}` | `ActivityInput` → `201` / `200` |
| DELETE | `/api/v1/train/activities/{id}` | `204` |

```json
ActivityInput {
  "kind": "run" | "ride" | "walk" | "hike" | "swim" | "row" | "other",
  "title": "Easy run", "notes": null, "started_at": "…",
  "duration_seconds": 1800, "distance_m": 5000.0, "elevation_gain_m": null,
  "avg_heart_rate": 145, "calories": null, "perceived_effort": 4     // 1–10
}
Activity = ActivityInput + { id, created_at, updated_at }
```

## Stats

Period endpoints take `from` and `to` (inclusive dates) and `tz` (IANA name, default `UTC`) so
days and weeks follow the user's local calendar. Weeks start on Monday. Periods are capped at
three years.

| Method | Path | Response |
|--------|------|----------|
| GET | `/api/v1/train/stats/overview?from&to&tz` | `{ from, to, current: Totals, previous: Totals, streak_weeks }` — `previous` is the equally long period right before `from` |
| GET | `/api/v1/train/stats/timeline?from&to&tz&bucket=week\|month` | `[{ bucket: "2026-09-14", ...Totals }]`, zero-filled |
| GET | `/api/v1/train/stats/muscles?from&to&tz` | `[{ muscle, sets, volume_kg }]` by primary muscle |
| GET | `/api/v1/train/stats/records` | `[{ exercise_id, exercise_name, tracking, primary_muscle, max_weight_kg, best_e1rm_kg, max_reps, max_duration_seconds, max_distance_m, sessions, last_performed_at }]` |

```json
Totals { "workouts": 3, "workout_seconds": 11700, "sets": 42, "reps": 380, "volume_kg": 18250.0,
         "activities": 2, "activity_seconds": 5400, "activity_distance_m": 16000.0 }
```

## Friends

Users find each other by exact username (case-insensitive); there is no directory or partial search.
Friends see each other only as `PublicUser { id, username, display_name }`, never email, birthday or role.
A user with a block in either direction answers `404` like an unknown username.
Lookups and friend requests share a limit of 300 per user per hour (`429 rate_limited`).

| Method | Path | Response |
|--------|------|----------|
| GET | `/api/v1/users/lookup?username=` | `{ user: PublicUser, relationship }`, `relationship` is `self`, `none`, `outgoing`, `incoming` or `friends` |
| GET | `/api/v1/friends` | `Friend[]` by username |
| GET | `/api/v1/friends/requests` | `{ incoming: FriendRequest[], outgoing: FriendRequest[] }`, newest first |
| POST | `/api/v1/friends/requests` | `{ username }` → `201 { user, relationship: "outgoing" }`; `200` with the current relationship when already sent or friends; `200 "friends"` when they had already asked you; `409` when you blocked them |
| POST | `/api/v1/friends/requests/{user_id}/accept` | `Friend`; only the addressee can accept |
| DELETE | `/api/v1/friends/requests/{user_id}` | `204`; declines an incoming or cancels an outgoing request |
| DELETE | `/api/v1/friends/{user_id}` | `204`; either side can remove, access ends immediately |
| GET | `/api/v1/blocks` | `[{ user: PublicUser, created_at }]` |
| PUT | `/api/v1/blocks/{user_id}` | `204`; idempotent, also deletes the friendship or pending request |
| DELETE | `/api/v1/blocks/{user_id}` | `204`; does not restore the friendship |

```json
Friend { "user": PublicUser, "since": "…",
         "sharing": { "workouts": true, "activities": false, "stats": true, "personal_records": false } }
FriendRequest { "user": PublicUser, "created_at": "…" }
```

### Sharing and progress

Nothing is shared until the user opts in.
Progress photos are never visible to friends.
Friend endpoints answer `404` for anyone who is not a friend and `403` when the friend does not share that resource.

| Method | Path | Response |
|--------|------|----------|
| GET | `/api/v1/me/sharing` | `Sharing`, all `false` by default |
| PUT | `/api/v1/me/sharing` | full `Sharing` → `Sharing`; unknown fields are rejected |
| GET | `/api/v1/friends/feed?limit&cursor` | `Page<FeedEntry>` from every friend, for the kinds each one shares |
| GET | `/api/v1/friends/{user_id}` | `Friend` |
| GET | `/api/v1/friends/{user_id}/feed?limit&cursor` | `Page<FeedEntry>` of that friend |
| GET | `/api/v1/friends/{user_id}/stats/overview?from&to&tz` | same as `/stats/overview`; needs `stats` |
| GET | `/api/v1/friends/{user_id}/records` | same as `/stats/records`; needs `personal_records` |

Feed entries are finished workouts and activities, newest first, without notes, place, routine or health data.

```json
FeedEntry { "user": PublicUser, "type": "workout", "id": "uuid", "title": "Push Day", "started_at": "…",
            "ended_at": "…", "duration_seconds": 3600, "exercise_count": 1, "set_count": 2,
            "total_reps": 10, "volume_kg": 1000.0, "exercise_names": ["Barbell Bench Press"] }
FeedEntry { "user": PublicUser, "type": "activity", "id": "uuid", "kind": "run", "title": "Easy run",
            "started_at": "…", "duration_seconds": 1800, "distance_m": 5000.0, "elevation_gain_m": null }
```

## Changes from the legacy Node API

| Legacy (Express + MongoDB)                  | Now |
|---------------------------------------------|-----|
| `POST /api/v1/users/create`                 | `POST /api/v1/auth/register` (no `confPasswd`; confirm on the client) |
| `POST /api/v1/users/login`                  | `POST /api/v1/auth/login` (same `login` + `password` body) |
| `POST /api/v1/users/renewToken`             | Removed — sessions slide forward on use |
| `POST /api/v1/users/getAllUsers` (public!)  | `GET /api/v1/users`, admin only |
| `POST /api/v1/train/exercises/create`             | `POST /api/v1/train/exercises` (`ytVideoID` → `video_id`, `muscleGroup[]` → `primary_muscle` + `secondary_muscles`) |
| `GET /api/v1/train/exercises/getAll`              | `GET /api/v1/train/exercises` |
| Playlists (model only, never routed)        | Routines |
| 30-minute JWTs, token also in a cookie      | Opaque bearer sessions stored hashed server-side |
| Roles `user` / `admin` / `superadmin`       | `user` / `admin` (`superadmin` had no distinct permission) |

## FitHealth (`/api/v1/health`)

Nutrition values are per 100 of the product's `unit`: `g`, or `ml` for drinks (EU labels give drinks per 100 ml). Entry `amount` and `serving_amount` are in that unit. Diary days are the user's local date; pass `tz` (IANA name) like the stats endpoints.

| Method | Path | Result |
| --- | --- | --- |
| GET | `/api/v1/health/products?q=&limit=20` | `Product[]`: name or brand contains `q`, the caller's recently logged products first |
| GET | `/api/v1/health/products/{id}` | `Product` |
| POST | `/api/v1/health/products` | `201 Product`; products are shared by all users |
| PUT | `/api/v1/health/products/{id}` | `Product`; any user may correct a product |
| GET | `/api/v1/health/meals` | `Meal[]` in order; creates the default meals on first use |
| POST | `/api/v1/health/meals` | `201 Meal`, `{ name }`, at most 10 |
| PATCH | `/api/v1/health/meals/{id}` | `Meal`, `{ name }` |
| PUT | `/api/v1/health/meals/order` | `Meal[]`, `{ ids }` listing every active meal once |
| DELETE | `/api/v1/health/meals/{id}` | `204`; archives, past entries keep it; `409` for the last meal |
| GET | `/api/v1/health/days/{date}?tz=` | `Day`: meals with entries and totals, day totals, targets, training, weight, `missing` |
| PUT | `/api/v1/health/entries/{id}` | `201` or `200 Entry`, `{ date, meal_id, product_id, amount }`, the amount in the product's unit |
| DELETE | `/api/v1/health/entries/{id}` | `204` |
| GET | `/api/v1/health/profile` | `Profile`, all fields `null` before setup |
| PUT | `/api/v1/health/profile` | `Profile`: `sex`, `height_cm`, `activity`, `goal`, `pace_kg_per_week`, optional overrides `energy_kcal`, `protein_g`, `fat_g`, `carbs_g` |
| GET | `/api/v1/health/weights?limit=30` | `Weight[]`, newest first |
| PUT | `/api/v1/health/weights/{date}` | `Weight`, `{ weight_kg }` |
| DELETE | `/api/v1/health/weights/{date}` | `204` |

- Entries copy the product's name and values when logged, so later product edits do not change past days
- Targets use Mifflin-St Jeor with a daily-life factor, the goal pace (7700 kcal per kg) and a 1200 kcal floor, then add the day's FitTune training: recorded activity calories, or `(MET - 1) * kg * hours` from activity kind or running speed, and 5 MET for finished workouts (at most 3 hours)
- `activity` in the profile describes daily life without exercise, since training is added per day
- `missing` lists what stops targets from being calculated: `profile`, `birthday` (set through `PATCH /api/v1/me`), `weight`
