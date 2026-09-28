# Exercise media in the API

## Goal

The app hardcoded which catalog exercises have demo photos (Free Exercise DB on GitHub) and
videos (YouTube, Vimeo). Media now live in the database, catalog photos are stored in RustFS and
served by the API, and videos stay embedded from their provider. Photos stay out of the frontend
bundle.

## Data model

`exercise_media` (migration `20260928000004`):

| column | notes |
|---|---|
| `id` | UUID; catalog rows use `md5('fittune:exercise-media:' \|\| lower(name) \|\| ':' \|\| kind \|\| ':' \|\| position)` |
| `exercise_id` | FK to `exercises`, cascade delete |
| `kind` | `photo` or `video` |
| `provider` | `fittune` (photo in RustFS), `youtube`, `vimeo` |
| `external_id` | provider video id; videos only |
| `storage_key` | `catalog/<id>.jpg`; photos only, unique |
| `source_url` | `https://` URL the backfill copies a photo from |
| `position` | order within a kind; photos `0` = start, `1` = finish |

A CHECK ties each kind to its provider and fields; `(exercise_id, kind, position)` is unique.
The migration seeds 43 catalog exercises × 2 photos and 5 videos, and copies existing
`exercises.video_id` values into YouTube rows. The column is no longer read or written and is
dropped by a later migration once the new app is live.

## Backfill

On `serve`, a background task (skipped when photo storage is not configured) lists rows with a
`source_url`, skips those whose `storage_key` exists (`PhotoStore::exists`, S3 `head_object`),
downloads the rest over HTTPS only (15 s timeout, 5 MB cap), requires a decodable JPEG and stores
it unchanged. Failures are logged per photo and retried on the next start. Photos are processed
one at a time: 86 small files do not need concurrency.

## API

- Every `Exercise` gets `media: [ExerciseMedia]` (photos, then videos, by position), loaded with
  one query per request. `video_id` is derived from the first YouTube video.
- `POST`/`PUT /exercises` still take `video_id`: it upserts the first video as YouTube, or
  removes a YouTube first video when `null` (a Vimeo one is left alone), in the same transaction
  as the exercise.
- `GET /api/v1/exercise-media/{id}/file` serves stored photos of catalog exercises without auth,
  with `Cache-Control: public, max-age=31536000, immutable`; everything else is `404`.

## App

`media` in the exercise schema defaults to `[]`, the hardcoded maps are removed, photos load from
`API_BASE_URL + url`, videos from the first video in `media` with `video_id` as fallback. The
app's CSP (platform-edge) must allow images from the API origin.

## Rollout

1. platform-edge: add the API origin to the app's `img-src`.
2. API release: migration and backfill run on start.
3. App release.
4. Later: migration dropping `exercises.video_id`.
