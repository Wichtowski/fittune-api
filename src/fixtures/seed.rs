//! Writes the fixture plan through the API as each fixture account.

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    client::{Api, text},
    plan::{
        self, ACCOUNTS, Account, Clock, INVITES, InviteState, Kind, PASSWORD, PlannedWorkout,
        fixture_id,
    },
};
use crate::{
    AppState,
    auth::signup::{self, NewUser},
    users::{self, model::Role},
};

const OK: &[StatusCode] = &[StatusCode::OK];
const SAVED: &[StatusCode] = &[StatusCode::OK, StatusCode::CREATED];
const CREATED: &[StatusCode] = &[StatusCode::CREATED];

/// What a run wrote, for the command output.
#[derive(Debug, Default)]
pub struct Summary {
    pub accounts_created: usize,
    pub routines: usize,
    pub workouts: usize,
    pub activities: usize,
    /// Fixture workouts left alone because the app saved a newer revision of them
    pub kept_workouts: Vec<String>,
    /// Only known when this run created the invite; the API never shows a code twice
    pub active_invite_code: Option<String>,
}

struct Session {
    token: String,
    user_id: Uuid,
}

/// Creates or updates every fixture. Safe to run any number of times.
pub async fn seed(state: &AppState, clock: Clock) -> Result<Summary> {
    let api = Api::new(state);
    let mut summary = Summary::default();
    let mut sessions = Vec::new();

    let admin = ensure_admin(state, &api, &mut summary).await?;
    for account in ACCOUNTS.iter().filter(|a| a.kind != Kind::Admin) {
        let session = ensure_user(state, &api, account, &admin.token, &mut summary).await?;
        seed_account(&api, account, &session.token, clock, &mut summary).await?;
        sessions.push(session);
    }
    seed_account(&api, &ACCOUNTS[0], &admin.token, clock, &mut summary).await?;
    save_friends(&api, &sessions).await?;
    summary.active_invite_code = save_invites(state, &api, &admin).await?;

    for session in sessions.iter().chain([&admin]) {
        api.expect(
            Method::POST,
            "/api/v1/auth/logout",
            Some(&session.token),
            None,
            &[StatusCode::NO_CONTENT],
        )
        .await?;
    }
    Ok(summary)
}

async fn seed_account(
    api: &Api,
    account: &Account,
    token: &str,
    clock: Clock,
    summary: &mut Summary,
) -> Result<()> {
    let token = Some(token);
    let profile = (account.profile)();
    api.expect(Method::PATCH, "/api/v1/me", token, Some(&profile), OK)
        .await?;

    let custom_exercises = plan::custom_exercises(account.kind);
    if !custom_exercises.is_empty() {
        save_custom_exercises(api, token, custom_exercises).await?;
    }
    let exercises = exercise_ids(api, token).await?;
    let places = save_places(api, account, token).await?;
    let routines = save_routines(api, account, token, &exercises).await?;
    summary.routines += routines.len();

    let ids = Ids {
        exercises: &exercises,
        routines: &routines,
        places: &places,
    };
    for workout in plan::workouts(account.kind, &clock) {
        let body = workout_body(account, &workout, &ids)?;
        let id = fixture_id(&format!("{}/workout/{}", account.username, workout.key));
        let uri = format!("/api/v1/train/workouts/{id}");
        let (status, response) = api.call(Method::PUT, &uri, token, Some(&body)).await?;
        match status {
            StatusCode::OK | StatusCode::CREATED => summary.workouts += 1,
            // The app saved a newer revision, which wins like it would against any other device
            StatusCode::CONFLICT => summary
                .kept_workouts
                .push(format!("{}/{}", account.username, workout.key)),
            _ => bail!("PUT {uri} returned {status}: {response}"),
        }
    }

    for activity in plan::activities(account.kind, &clock) {
        let id = fixture_id(&format!("{}/activity/{}", account.username, activity.key));
        let uri = format!("/api/v1/train/activities/{id}");
        api.expect(Method::PUT, &uri, token, Some(&activity.body), SAVED)
            .await?;
        summary.activities += 1;
    }
    Ok(())
}

/// demo and casual are friends, demo sharing everything and casual only its sessions, and
/// newbie's request to demo waits for an answer. Requests are idempotent, and sending one back
/// accepts it, so reruns keep this state
async fn save_friends(api: &Api, sessions: &[Session]) -> Result<()> {
    let [demo, casual, newbie] = sessions else {
        bail!("expected the demo, casual and newbie sessions");
    };
    for (from, to) in [(casual, "demo"), (demo, "casual"), (newbie, "demo")] {
        let body = json!({ "username": to });
        api.expect(
            Method::POST,
            "/api/v1/friends/requests",
            Some(&from.token),
            Some(&body),
            SAVED,
        )
        .await?;
    }
    for (session, sharing) in [
        (
            demo,
            json!({ "workouts": true, "activities": true, "stats": true, "personal_records": true }),
        ),
        (
            casual,
            json!({ "workouts": true, "activities": true, "stats": false, "personal_records": false }),
        ),
    ] {
        api.expect(
            Method::PUT,
            "/api/v1/me/sharing",
            Some(&session.token),
            Some(&sharing),
            OK,
        )
        .await?;
    }
    Ok(())
}

/// Deletes every fixture account and everything it owns, leaving other accounts alone.
/// Returns how many accounts were removed.
pub async fn reset(state: &AppState) -> Result<usize> {
    let api = Api::new(state);
    let mut removed = 0;
    for account in &ACCOUNTS {
        let Some(user_id) = fixture_user_id(state, account).await? else {
            continue;
        };
        // Only fixtures use the fixture admin, so the invites it issued go with it
        sqlx::query("DELETE FROM invites WHERE created_by = $1")
            .bind(user_id)
            .execute(&state.db)
            .await?;
        match sign_in(state, &api, account).await? {
            // Deleting through the API also removes stored progress photos
            Some(session) => {
                api.expect(
                    Method::DELETE,
                    "/api/v1/me",
                    Some(&session.token),
                    Some(&json!({ "password": PASSWORD })),
                    &[StatusCode::NO_CONTENT],
                )
                .await?;
            }
            None => {
                eprintln!(
                    "warning: {} no longer accepts the fixture password; deleting its rows directly (progress photo files, if any, stay in storage)",
                    account.email()
                );
                users::repo::delete(&state.db, user_id).await?;
            }
        }
        removed += 1;
    }
    Ok(removed)
}

/// The fixture account's id, if it exists. Another account holding the fixture username or
/// email is an error, so fixtures never sign in as, change or delete someone else's account.
async fn fixture_user_id(state: &AppState, account: &Account) -> Result<Option<Uuid>> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, email FROM users WHERE lower(username) = lower($1) OR lower(email) = lower($2)",
    )
    .bind(account.username)
    .bind(account.email())
    .fetch_all(&state.db)
    .await?;
    match rows.as_slice() {
        [] => Ok(None),
        [(id, email)] if email.eq_ignore_ascii_case(&account.email()) => Ok(Some(*id)),
        _ => bail!(
            "the username `{}` or email {} belongs to an account that is not a fixture; use a dedicated fixture database",
            account.username,
            account.email()
        ),
    }
}

/// Signs in with the fixture password; `None` when the account is missing or its password
/// was changed.
async fn sign_in(state: &AppState, api: &Api, account: &Account) -> Result<Option<Session>> {
    if fixture_user_id(state, account).await?.is_none() {
        return Ok(None);
    }
    let body = json!({ "login": account.email(), "password": PASSWORD });
    let (status, response) = api
        .expect(
            Method::POST,
            "/api/v1/auth/login",
            None,
            Some(&body),
            &[StatusCode::OK, StatusCode::UNAUTHORIZED],
        )
        .await?;
    if status == StatusCode::UNAUTHORIZED {
        return Ok(None);
    }
    session(&response).map(Some)
}

fn session(auth: &Value) -> Result<Session> {
    Ok(Session {
        token: text(auth, "/session/token")?,
        user_id: text(auth, "/user/id")?.parse()?,
    })
}

/// Signs in, or explains why an existing fixture account cannot be used.
async fn existing(state: &AppState, api: &Api, account: &Account) -> Result<Option<Session>> {
    if let Some(session) = sign_in(state, api, account).await? {
        return Ok(Some(session));
    }
    if fixture_user_id(state, account).await?.is_some() {
        bail!(
            "fixture account {} no longer accepts the fixture password; run `make seed-reset` to recreate the fixture accounts",
            account.email()
        );
    }
    Ok(None)
}

/// The first admin is created the way `create-admin` does it, since nobody could invite it.
async fn ensure_admin(state: &AppState, api: &Api, summary: &mut Summary) -> Result<Session> {
    let account = &ACCOUNTS[0];
    if let Some(session) = existing(state, api, account).await? {
        return Ok(session);
    }
    let user = NewUser::validate(
        account.username,
        &account.email(),
        PASSWORD.to_owned(),
        None,
        None,
    )
    .map_err(|err| anyhow!("invalid fixture admin: {err:?}"))?;
    let mut tx = state.db.begin().await?;
    signup::create(&mut tx, user, Role::Admin, None)
        .await
        .map_err(|err| anyhow!("failed to create the fixture admin: {err:?}"))?;
    tx.commit().await?;
    summary.accounts_created += 1;
    sign_in(state, api, account)
        .await?
        .context("the new fixture admin cannot sign in")
}

/// Everyone else signs up like a real user: with an invite from the fixture admin.
async fn ensure_user(
    state: &AppState,
    api: &Api,
    account: &Account,
    admin_token: &str,
    summary: &mut Summary,
) -> Result<Session> {
    if let Some(session) = existing(state, api, account).await? {
        return Ok(session);
    }
    let invite = json!({
        "note": format!("Fixture: sign-up for {}", account.username),
        "expires_in_days": 1,
    });
    let (_, created) = api
        .expect(
            Method::POST,
            "/api/v1/admin/invites",
            Some(admin_token),
            Some(&invite),
            CREATED,
        )
        .await?;
    let registration = json!({
        "username": account.username,
        "email": account.email(),
        "password": PASSWORD,
        "invite_code": text(&created, "/code")?,
    });
    let (_, auth) = api
        .expect(
            Method::POST,
            "/api/v1/auth/register",
            None,
            Some(&registration),
            CREATED,
        )
        .await?;
    summary.accounts_created += 1;
    session(&auth)
}

async fn list(api: &Api, token: Option<&str>, uri: &str) -> Result<Vec<Value>> {
    let (_, list) = api.expect(Method::GET, uri, token, None, OK).await?;
    match list {
        Value::Array(items) => Ok(items),
        other => bail!("GET {uri} did not return a list: {other}"),
    }
}

/// Lowercased name to id, for every item of `items` that `keep` accepts.
fn by_name(items: &[Value], keep: impl Fn(&Value) -> bool) -> Result<HashMap<String, String>> {
    items
        .iter()
        .filter(|item| keep(item))
        .map(|item| Ok((text(item, "/name")?.to_lowercase(), text(item, "/id")?)))
        .collect()
}

/// The catalog and the account's own exercises. Everyone sees what other users created too,
/// and names are only unique per owner, so those are left out: a plan must never pick up
/// another account's exercise of the same name.
async fn exercise_ids(api: &Api, token: Option<&str>) -> Result<HashMap<String, String>> {
    by_name(&list(api, token, "/api/v1/train/exercises").await?, |e| {
        e["is_custom"] == Value::Bool(false) || e["is_own"] == Value::Bool(true)
    })
}

/// Custom exercise ids come from the server, so an existing one with the same name is updated.
async fn save_custom_exercises(api: &Api, token: Option<&str>, bodies: Vec<Value>) -> Result<()> {
    let existing = by_name(&list(api, token, "/api/v1/train/exercises").await?, |e| {
        e["is_own"] == Value::Bool(true)
    })?;
    for body in bodies {
        match existing.get(&text(&body, "/name")?.to_lowercase()) {
            Some(id) => {
                let uri = format!("/api/v1/train/exercises/{id}");
                api.expect(Method::PUT, &uri, token, Some(&body), OK)
                    .await?
            }
            None => {
                api.expect(
                    Method::POST,
                    "/api/v1/train/exercises",
                    token,
                    Some(&body),
                    CREATED,
                )
                .await?
            }
        };
    }
    Ok(())
}

/// Place key to the id of the place's current version.
async fn save_places(
    api: &Api,
    account: &Account,
    token: Option<&str>,
) -> Result<HashMap<&'static str, String>> {
    let mut versions = HashMap::new();
    for place in plan::places(account.kind) {
        let id = fixture_id(&format!("{}/place/{}", account.username, place.key));
        let body = json!({ "name": place.name, "kind": place.kind, "equipment": place.equipment });
        let uri = format!("/api/v1/train/places/{id}");
        let (status, saved) = api.call(Method::PUT, &uri, token, Some(&body)).await?;
        match status {
            StatusCode::OK | StatusCode::CREATED => {
                versions.insert(place.key, text(&saved, "/version_id")?);
            }
            StatusCode::NOT_FOUND => bail!(
                "the fixture place `{}` was archived in the app; run `make seed-reset` to recreate it",
                place.name
            ),
            _ => bail!("PUT {uri} returned {status}: {saved}"),
        }
    }
    Ok(versions)
}

/// Routine name to id. Routine ids come from the server, so routines are matched by name.
async fn save_routines(
    api: &Api,
    account: &Account,
    token: Option<&str>,
    exercises: &HashMap<String, String>,
) -> Result<HashMap<&'static str, String>> {
    let existing = by_name(&list(api, token, "/api/v1/train/routines").await?, |_| true)?;
    let mut ids = HashMap::new();
    for template in plan::routines(account.kind) {
        let routine_exercises = template
            .exercises()
            .iter()
            .map(|planned| {
                Ok(json!({
                    "exercise_id": exercise_id(exercises, planned.exercise)?,
                    "rest_seconds": planned.rest_seconds,
                    "sets": planned.sets.iter().map(plan::Set::target).collect::<Vec<_>>(),
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        let body = json!({
            "name": template.routine,
            "notes": template.notes,
            "exercises": routine_exercises,
        });
        let (_, saved) = match existing.get(&template.routine.to_lowercase()) {
            Some(id) => {
                let uri = format!("/api/v1/train/routines/{id}");
                api.expect(Method::PUT, &uri, token, Some(&body), OK)
                    .await?
            }
            None => {
                api.expect(
                    Method::POST,
                    "/api/v1/train/routines",
                    token,
                    Some(&body),
                    CREATED,
                )
                .await?
            }
        };
        ids.insert(template.routine, text(&saved, "/id")?);
    }
    Ok(ids)
}

fn exercise_id(exercises: &HashMap<String, String>, name: &str) -> Result<String> {
    exercises
        .get(&name.to_lowercase())
        .cloned()
        .with_context(|| format!("no exercise named `{name}`; did the catalog change?"))
}

struct Ids<'a> {
    exercises: &'a HashMap<String, String>,
    routines: &'a HashMap<&'static str, String>,
    places: &'a HashMap<&'static str, String>,
}

/// The full workout document the app would upload, with ids derived from the workout key.
fn workout_body(account: &Account, workout: &PlannedWorkout, ids: &Ids<'_>) -> Result<Value> {
    let key = format!("{}/workout/{}", account.username, workout.key);
    let exercises = workout
        .exercises
        .iter()
        .enumerate()
        .map(|(i, planned)| {
            let sets: Vec<Value> = planned
                .sets
                .iter()
                .enumerate()
                .map(|(j, set)| {
                    json!({
                        "id": fixture_id(&format!("{key}/{i}/{j}")),
                        "kind": set.kind,
                        "reps": set.reps,
                        "weight_kg": set.weight_kg,
                        "duration_seconds": set.duration_seconds,
                        "distance_m": set.distance_m,
                        "rpe": set.rpe,
                        "completed": set.completed,
                    })
                })
                .collect();
            Ok(json!({
                "id": fixture_id(&format!("{key}/{i}")),
                "exercise_id": exercise_id(ids.exercises, planned.exercise)?,
                "rest_seconds": planned.rest_seconds,
                "sets": sets,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let place_version_id = ids
        .places
        .get(workout.place)
        .with_context(|| format!("no fixture place `{}`", workout.place))?;

    Ok(json!({
        "title": workout.title,
        "notes": workout.notes,
        "routine_id": ids.routines.get(workout.routine),
        "place_version_id": place_version_id,
        "started_at": workout.started_at,
        "ended_at": workout.ended_at,
        // The first revision: anything the app saves later is newer and survives a rerun
        "revision": 1,
        "exercises": exercises,
    }))
}

/// Invites in the states the admin screen shows; the used ones come from fixture sign-ups.
/// Returns the active code when this run created it.
async fn save_invites(state: &AppState, api: &Api, admin: &Session) -> Result<Option<String>> {
    let token = Some(admin.token.as_str());
    let invites = list(api, token, "/api/v1/admin/invites").await?;
    let notes: Vec<&str> = invites
        .iter()
        .filter_map(|invite| invite["note"].as_str())
        .collect();

    let mut active_code = None;
    for (note, wanted) in INVITES {
        if notes.contains(&note) {
            continue;
        }
        let body = json!({ "note": note, "expires_in_days": 90, "max_uses": 5 });
        let (_, created) = api
            .expect(
                Method::POST,
                "/api/v1/admin/invites",
                token,
                Some(&body),
                CREATED,
            )
            .await?;
        let id = text(&created, "/invite/id")?;
        match wanted {
            InviteState::Active => active_code = Some(text(&created, "/code")?),
            InviteState::Revoked => {
                let uri = format!("/api/v1/admin/invites/{id}");
                api.expect(Method::DELETE, &uri, token, None, &[StatusCode::NO_CONTENT])
                    .await?;
            }
            InviteState::Expired => {
                // The API only takes expiry in days from now, so only fixtures back-date one
                sqlx::query(
                    "UPDATE invites SET expires_at = now() - interval '1 day'
                     WHERE id = $1::uuid AND created_by = $2",
                )
                .bind(&id)
                .bind(admin.user_id)
                .execute(&state.db)
                .await?;
            }
        }
    }
    Ok(active_code)
}
