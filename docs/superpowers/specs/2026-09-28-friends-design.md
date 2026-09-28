# Friends and shared progress

Tracks fittune-api#9 and fittune-app#6.

## Goal

Users add each other by exact username and see each other's progress.
Each user decides what friends can see, and nothing is shared until they opt in.

## Storage

Postgres, no new database.
The friend graph is one hop deep, and everything a friend can see (workouts, activities, stats) is already relational.

## Data model

Migration `20260928000005_friends.sql`:

- `friendships (requester_id, addressee_id, status, created_at, responded_at)`
  - `status` is `pending` or `accepted`; `responded_at` is set exactly when accepted
  - Primary key `(requester_id, addressee_id)` plus a unique index on `(LEAST, GREATEST)`, so a pair has at most one row whichever way the request went
  - Declining, cancelling and removing delete the row
- `user_blocks (blocker_id, blocked_id, created_at)`
  - One row per direction, so each side can block and unblock independently
- `friend_sharing (user_id, workouts, activities, stats, personal_records, updated_at)`
  - A missing row means everything is private
- All three cascade on user deletion

Progress photos are never visible to friends.
Measurements do not exist yet, so there is nothing to share.

## Rules

- A friend is someone with an `accepted` row and no block in either direction
- Every friend read goes through `friends::access`, which returns `404` for non-friends and `403` for friends who do not share that resource
- The feed uses the same query to find which friends share workouts or activities
- Friends only see `{ id, username, display_name }` of each other, never email, birthday or role
- Workouts are shown as summaries without place, routine or notes
- Activities are shown without notes, heart rate, calories and perceived effort
- Only finished workouts are visible

## Concurrency

Every write that touches a pair (request, block) takes a transaction-scoped advisory lock on the unordered pair first.
That serialises crossed requests and a block racing a request.

- A request to someone who already sent one to you accepts theirs
- Repeating a request you already sent is a no-op that returns the current state
- A request involving a block, in either direction, answers `404` like an unknown username, so a block is not revealed
- Blocking deletes the friendship or pending request in the same transaction

## Rate limits

The app is used by a small invited group, so the limit is only a guard against runaway clients.
Username lookups and friend requests share a per-user window of 300 per hour.

## API

See `docs/API.md`, section "Friends".

## App

- Friends page: search by username, incoming and outgoing requests, friend list, feed
- Friend profile: overview tiles, recent sessions and records, each shown only when shared
- Profile settings: a "Sharing with friends" section with one toggle per resource
- Friend queries are not persisted to the offline cache, and a `403` or `404` from a friend endpoint drops that friend's cached data
