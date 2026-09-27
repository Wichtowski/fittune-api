# FitTune API v1

Base URL: `https://api-fittune.oskarwichtowski.com` in production, `http://localhost:4733` locally.
All endpoints except `/health` and `/api/v1/auth/{register,login}` require
`Authorization: Bearer <token>`.

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
| 403    | `forbidden`           | Authenticated but not allowed (admin-only actions)          |
| 404    | `not_found`           | Resource does not exist **or belongs to another user**      |
| 409    | `conflict`            | Stale workout revision, id collision                        |
| 422    | `validation_failed`   | Semantically invalid input; `fields` maps field paths to messages (`exercises[0].sets[1].reps`) |
| 500    | `internal_error`      | Unexpected server error (details are logged, not returned)  |

## Health

`GET /health` → `200 { "status": "ok", "version": "1.4.2", "database": "ok" }`, or `503` with
`"status": "degraded"` when Postgres is unreachable.

## Auth

Sessions are opaque bearer tokens (not JWTs). They expire after `FITTUNE_SESSION_TTL_HOURS`
(default 30 days) of inactivity; use extends them automatically.

| Method | Path                    | Body                                                               | Response |
|--------|-------------------------|--------------------------------------------------------------------|----------|
| POST   | `/api/v1/auth/register` | `{ username, email, password, display_name?, birthday? }`          | `201 AuthResponse` |
| POST   | `/api/v1/auth/login`    | `{ login, password }` — `login` is the username **or** email       | `200 AuthResponse` |
| POST   | `/api/v1/auth/logout`   | —                                                                  | `204`, revokes the current token |

`AuthResponse`: `{ "user": User, "session": { "token": "…", "expires_at": "…" } }`

Registration rules: username 3–32 chars of letters, digits, `.`, `_`, `-`; valid email; password
at least 8 characters with an uppercase letter and a special character, not containing the
username; birthday not in the future. Username and email are unique case-insensitively.

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
| GET    | `/api/v1/exercises?q=&muscle=&equipment=` | Active catalog + own exercises, sorted by name. `muscle` matches primary or secondary. |
| GET    | `/api/v1/exercises/{id}` | Also returns archived exercises (history still references them). |
| POST   | `/api/v1/exercises` | `ExerciseInput`; `"global": true` adds to the catalog (admin only). `201` |
| PUT    | `/api/v1/exercises/{id}` | Full replace. Owners edit custom exercises; admins edit the catalog. |
| DELETE | `/api/v1/exercises/{id}` | Archives (hides from the library, keeps history). `204` |
| GET    | `/api/v1/exercises/{id}/history?sessions=30` | Per-session breakdown and all-time records. |

```json
ExerciseInput {
  "name": "Sled Push",
  "tracking": "weight_reps" | "reps" | "duration" | "distance_duration",
  "primary_muscle": Muscle, "secondary_muscles": [Muscle],
  "equipment": "none" | "barbell" | "dumbbell" | "kettlebell" | "machine" | "cable" | "band" | "plate" | "other",
  "requires": [EquipmentItem],                // everything the exercise needs; [] for bodyweight
  "difficulty": "beginner" | "intermediate" | "advanced",
  "video_id": "dQw4w9WgXcQ" | null,           // YouTube id
  "instructions": "…" | null
}
EquipmentItem = "barbell" | "ez_bar" | "dumbbells" | "kettlebells"
              | "flat_bench" | "adjustable_bench" | "squat_rack" | "pull_up_bar" | "dip_station"
              | "leg_press" | "leg_extension" | "leg_curl" | "calf_raise_machine" | "smith_machine"
              | "chest_press_machine" | "pec_deck" | "shoulder_press_machine" | "assisted_pull_up_machine"
              | "cable_station" | "lat_pulldown" | "seated_row"
              | "treadmill" | "rowing_machine" | "stationary_bike"
              | "resistance_band" | "ab_wheel" | "jump_rope"
Muscle = "chest" | "lats" | "upper_back" | "lower_back" | "traps" | "shoulders" | "biceps" | "triceps"
       | "forearms" | "abs" | "quadriceps" | "hamstrings" | "glutes" | "calves" | "full_body" | "cardio"
Exercise = ExerciseInput + { id, is_custom, archived_at, created_at, updated_at }
```

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
| GET | `/api/v1/places` | `Place[]`, active places sorted by name |
| PUT | `/api/v1/places/{id}` | Client-generated UUID; `PlaceInput` → `201` created / `200` updated, returns `Place` |
| DELETE | `/api/v1/places/{id}` | `204`; archives the place without changing workout history |

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
| GET    | `/api/v1/workouts?status=in_progress\|completed&limit&cursor` | `Page<WorkoutSummary>`, newest first |
| GET    | `/api/v1/workouts/{id}` | `Workout` |
| PUT    | `/api/v1/workouts/{id}` | `WorkoutInput` → `201` created / `200` replaced, returns `Workout` |
| DELETE | `/api/v1/workouts/{id}` | `204` |

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
| GET    | `/api/v1/routines` | `Routine[]` sorted by name |
| GET    | `/api/v1/routines/{id}` | `Routine` |
| POST   | `/api/v1/routines` | `RoutineInput` → `201 Routine` |
| PUT    | `/api/v1/routines/{id}` | Full replace |
| DELETE | `/api/v1/routines/{id}` | `204`; past workouts keep their data |

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
| GET    | `/api/v1/activities?kind=&limit&cursor` | `Page<Activity>`, newest first |
| GET    | `/api/v1/activities/{id}` | `Activity` |
| PUT    | `/api/v1/activities/{id}` | `ActivityInput` → `201` / `200` |
| DELETE | `/api/v1/activities/{id}` | `204` |

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
| GET | `/api/v1/stats/overview?from&to&tz` | `{ from, to, current: Totals, previous: Totals, streak_weeks }` — `previous` is the equally long period right before `from` |
| GET | `/api/v1/stats/timeline?from&to&tz&bucket=week\|month` | `[{ bucket: "2026-09-14", ...Totals }]`, zero-filled |
| GET | `/api/v1/stats/muscles?from&to&tz` | `[{ muscle, sets, volume_kg }]` by primary muscle |
| GET | `/api/v1/stats/records` | `[{ exercise_id, exercise_name, tracking, primary_muscle, max_weight_kg, best_e1rm_kg, max_reps, max_duration_seconds, max_distance_m, sessions, last_performed_at }]` |

```json
Totals { "workouts": 3, "workout_seconds": 11700, "sets": 42, "reps": 380, "volume_kg": 18250.0,
         "activities": 2, "activity_seconds": 5400, "activity_distance_m": 16000.0 }
```

## Changes from the legacy Node API

| Legacy (Express + MongoDB)                  | Now |
|---------------------------------------------|-----|
| `POST /api/v1/users/create`                 | `POST /api/v1/auth/register` (no `confPasswd`; confirm on the client) |
| `POST /api/v1/users/login`                  | `POST /api/v1/auth/login` (same `login` + `password` body) |
| `POST /api/v1/users/renewToken`             | Removed — sessions slide forward on use |
| `POST /api/v1/users/getAllUsers` (public!)  | `GET /api/v1/users`, admin only |
| `POST /api/v1/exercises/create`             | `POST /api/v1/exercises` (`ytVideoID` → `video_id`, `muscleGroup[]` → `primary_muscle` + `secondary_muscles`) |
| `GET /api/v1/exercises/getAll`              | `GET /api/v1/exercises` |
| Playlists (model only, never routed)        | Routines |
| 30-minute JWTs, token also in a cookie      | Opaque bearer sessions stored hashed server-side |
| Roles `user` / `admin` / `superadmin`       | `user` / `admin` (`superadmin` had no distinct permission) |
