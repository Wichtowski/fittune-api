//! Keyset pagination over `(started_at, id)`, shared by time-ordered collections.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub const DEFAULT_LIMIT: i64 = 20;
pub const MAX_LIMIT: i64 = 100;

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Pass back as `cursor` to fetch the next page; absent on the last page.
    pub next_cursor: Option<String>,
}

pub fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Decodes a cursor produced by [`encode_cursor`].
pub fn decode_cursor(cursor: Option<&str>) -> ApiResult<Option<(DateTime<Utc>, Uuid)>> {
    let Some(cursor) = cursor.filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let invalid = || ApiError::BadRequest("Invalid cursor".to_owned());
    let (micros, id) = cursor.split_once('_').ok_or_else(invalid)?;
    let micros: i64 = micros.parse().map_err(|_| invalid())?;
    let at = DateTime::from_timestamp_micros(micros).ok_or_else(invalid)?;
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok(Some((at, id)))
}

pub fn encode_cursor(at: DateTime<Utc>, id: Uuid) -> String {
    format!("{}_{id}", at.timestamp_micros())
}

/// Builds a page from `limit + 1` fetched rows: the extra row only signals that more exist.
pub fn paginate<T>(
    mut rows: Vec<T>,
    limit: i64,
    key: impl Fn(&T) -> (DateTime<Utc>, Uuid),
) -> Page<T> {
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = has_more
        .then(|| {
            rows.last().map(|row| {
                let (at, id) = key(row);
                encode_cursor(at, id)
            })
        })
        .flatten();
    Page {
        items: rows,
        next_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() -> ApiResult<()> {
        let at = DateTime::from_timestamp_micros(1_790_000_000_123_456).expect("valid timestamp");
        let id = Uuid::from_u128(42);
        assert_eq!(decode_cursor(Some(&encode_cursor(at, id)))?, Some((at, id)));
        assert_eq!(decode_cursor(None)?, None);
        assert!(decode_cursor(Some("garbage")).is_err());
        Ok(())
    }

    #[test]
    fn paginate_emits_cursor_only_when_more_rows_exist() {
        let at = DateTime::from_timestamp_micros(0).expect("valid timestamp");
        let rows: Vec<u128> = (0..3).collect();
        let key = |n: &u128| (at, Uuid::from_u128(*n));

        let page = paginate(rows.clone(), 2, key);
        assert_eq!(page.items, [0, 1]);
        assert_eq!(
            page.next_cursor,
            Some(encode_cursor(at, Uuid::from_u128(1)))
        );

        let page = paginate(rows, 3, key);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.next_cursor, None);
    }
}
