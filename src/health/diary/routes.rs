use axum::{
    Router,
    extract::State,
    http::StatusCode,
    routing::{get, put},
};
use chrono::NaiveDate;
use uuid::Uuid;

use super::{
    model::{Day, DayMeal, DayQuery, Entry, EntryRequest},
    repo::{self, SaveOutcome},
};
use crate::{
    app::AppState,
    auth::Auth,
    error::{ApiError, ApiResult},
    extract::{Json, Path, Query},
    health::{
        body, exercise, meals,
        nutrients::Totals,
        targets::{self, Body},
    },
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/days/{date}", get(day))
        .route("/entries/{id}", put(save).delete(delete))
}

async fn save(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(input): Json<EntryRequest>,
) -> ApiResult<(StatusCode, axum::Json<Entry>)> {
    let input = input.validate()?;
    match repo::save(&state.db, auth.user_id(), id, &input).await? {
        SaveOutcome::Saved { created, entry } => Ok((
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            axum::Json(entry),
        )),
        SaveOutcome::NotFound(what) => Err(ApiError::NotFound(what)),
    }
}

async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    if repo::delete(&state.db, auth.user_id(), id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound("entry"))
    }
}

async fn day(
    State(state): State<AppState>,
    auth: Auth,
    Path(date): Path<NaiveDate>,
    Query(query): Query<DayQuery>,
) -> ApiResult<axum::Json<Day>> {
    let db = &state.db;
    let user_id = auth.user_id();
    let tz = query.tz.trim();
    if tz.is_empty() || tz.len() > 64 {
        return Err(ApiError::validation("tz", "Unknown time zone"));
    }
    repo::check_time_zone(db, tz)
        .await
        .map_err(time_zone_error)?;

    // First visit creates the default meals
    meals::repo::list(db, user_id).await?;
    let mut entries = repo::day_entries(db, user_id, date).await?;
    let mut day_meals = Vec::new();
    let mut totals = Totals::default();
    for meal in repo::day_meals(db, user_id, date).await? {
        let (mine, rest): (Vec<Entry>, Vec<Entry>) =
            entries.into_iter().partition(|e| e.meal_id == meal.id);
        entries = rest;
        let meal_totals = mine.iter().fold(Totals::default(), |sum, e| {
            sum + Totals::of(&e.per_100g, e.grams)
        });
        totals = totals + meal_totals;
        day_meals.push(DayMeal {
            id: meal.id,
            name: meal.name,
            archived: meal.archived,
            entries: mine,
            totals: meal_totals,
        });
    }

    let profile = body::repo::profile(db, user_id).await?;
    let birthday = repo::birthday(db, user_id).await?;
    let weight_kg = body::repo::weight_on(db, user_id, date).await?;
    let activities = repo::activities(db, user_id, date, tz).await?;
    let workouts = repo::workout_seconds(db, user_id, date, tz).await?;
    let exercise = exercise::day(&activities, &workouts, weight_kg);

    let mut missing = Vec::new();
    if profile.as_ref().and_then(|p| p.sex).is_none() {
        missing.push("profile");
    }
    if birthday.is_none() {
        missing.push("birthday");
    }
    if weight_kg.is_none() {
        missing.push("weight");
    }
    let body = match (&profile, birthday, weight_kg) {
        (Some(p), Some(born), Some(weight_kg)) => {
            match (p.sex, p.height_cm, p.activity, p.goal, p.pace_kg_per_week) {
                (
                    Some(sex),
                    Some(height_cm),
                    Some(activity),
                    Some(goal),
                    Some(pace_kg_per_week),
                ) => Some(Body {
                    sex,
                    age_years: date.years_since(born).unwrap_or(0),
                    height_cm,
                    weight_kg,
                    activity,
                    goal,
                    pace_kg_per_week,
                }),
                _ => None,
            }
        }
        _ => None,
    };
    let overrides = profile.as_ref().map(|p| p.overrides()).unwrap_or_default();
    let targets = targets::daily(body.as_ref(), overrides, exercise.energy_kcal);

    Ok(axum::Json(Day {
        date,
        meals: day_meals,
        totals,
        targets,
        exercise,
        weight_kg,
        missing,
    }))
}

fn time_zone_error(err: sqlx::Error) -> ApiError {
    match &err {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("22023") => {
            ApiError::validation("tz", "Unknown time zone")
        }
        _ => err.into(),
    }
}
