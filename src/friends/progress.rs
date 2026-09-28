//! What a user can read about their friends. Visibility is always decided by [`access`]

use std::collections::HashMap;

use axum::extract::State;
use serde::Deserialize;
use uuid::Uuid;

use super::{
    access,
    model::{FeedEntry, FeedItem, Friend, Resource},
};
use crate::{
    activities,
    app::AppState,
    auth::Auth,
    error::ApiResult,
    extract::{Path, Query},
    pagination::{self, Page},
    stats::{
        self,
        model::{ExerciseRecord, Overview, PeriodQuery},
    },
    workouts::{self, repo::StatusFilter},
};

#[derive(Debug, Deserialize)]
pub struct FeedQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

/// Recent finished workouts and activities of every friend who shares them
pub async fn feed(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<FeedQuery>,
) -> ApiResult<axum::Json<Page<FeedEntry>>> {
    let friends = access::friends(&state.db, auth.user_id(), None).await?;
    Ok(axum::Json(load_feed(&state, friends, &query).await?))
}

/// The same feed for one friend, limited to what they share
pub async fn friend_feed(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
    Query(query): Query<FeedQuery>,
) -> ApiResult<axum::Json<Page<FeedEntry>>> {
    let friend = access::friend(&state.db, auth.user_id(), user_id).await?;
    Ok(axum::Json(load_feed(&state, vec![friend], &query).await?))
}

async fn load_feed(
    state: &AppState,
    friends: Vec<Friend>,
    query: &FeedQuery,
) -> ApiResult<Page<FeedEntry>> {
    let limit = pagination::clamp_limit(query.limit);
    let before = pagination::decode_cursor(query.cursor.as_deref())?;
    let sharing = |resource| {
        friends
            .iter()
            .filter(|f| f.sharing.allows(resource))
            .map(|f| f.user.id)
            .collect::<Vec<_>>()
    };
    let workout_owners = sharing(Resource::Workouts);
    let activity_owners = sharing(Resource::Activities);

    // Each source is fetched one past the page, so their merge holds every row the page needs
    let mut items: Vec<(Uuid, FeedItem)> = Vec::new();
    if !workout_owners.is_empty() {
        let params = workouts::repo::ListParams {
            status: Some(StatusFilter::Completed),
            before,
            limit: limit + 1,
        };
        let rows = workouts::repo::list(&state.db, &workout_owners, &params).await?;
        items.extend(
            rows.into_iter()
                .map(|w| (w.user_id, FeedItem::Workout(w.into()))),
        );
    }
    if !activity_owners.is_empty() {
        let rows =
            activities::repo::list(&state.db, &activity_owners, None, before, limit + 1).await?;
        items.extend(
            rows.into_iter()
                .map(|a| (a.user_id, FeedItem::Activity(a.into()))),
        );
    }
    items.sort_by_key(|(_, item)| std::cmp::Reverse(item.key()));

    let users: HashMap<Uuid, _> = friends.into_iter().map(|f| (f.user.id, f.user)).collect();
    let entries = items
        .into_iter()
        .filter_map(|(owner, item)| {
            users.get(&owner).map(|user| FeedEntry {
                user: user.clone(),
                item,
            })
        })
        .collect();
    Ok(pagination::paginate(entries, limit, |e: &FeedEntry| {
        e.item.key()
    }))
}

/// A friend and what they share, for the profile header
pub async fn profile(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<axum::Json<Friend>> {
    Ok(axum::Json(
        access::friend(&state.db, auth.user_id(), user_id).await?,
    ))
}

pub async fn overview(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
    Query(query): Query<PeriodQuery>,
) -> ApiResult<axum::Json<Overview>> {
    access::can_view(&state.db, auth.user_id(), user_id, Resource::Stats).await?;
    Ok(axum::Json(
        stats::overview_for(&state.db, user_id, &query).await?,
    ))
}

pub async fn records(
    State(state): State<AppState>,
    auth: Auth,
    Path(user_id): Path<Uuid>,
) -> ApiResult<axum::Json<Vec<ExerciseRecord>>> {
    access::can_view(
        &state.db,
        auth.user_id(),
        user_id,
        Resource::PersonalRecords,
    )
    .await?;
    Ok(axum::Json(stats::repo::records(&state.db, user_id).await?))
}
