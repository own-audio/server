// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cursor paging for every v2 list: `limit` and `cursor` in, `{items, next}`
//! out. A list sorts by its key with the id as the tie-breaker; the cursor is
//! the last row's key, opaque to clients (base64url JSON). Rejected: offsets,
//! whose pages shift under inserts and make the database count rows it
//! discards.

use crate::v2::problem::Problem;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

pub const DEFAULT_LIMIT: i64 = 100;
pub const MAX_LIMIT: i64 = 500;

#[derive(Deserialize, IntoParams, Default)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// How many items, 1 to 500; 100 when absent.
    pub limit: Option<i64>,
    /// `next` from the previous page; absent for the first.
    pub cursor: Option<String>,
}

impl PageQuery {
    pub fn limit(&self) -> Result<i64, Problem> {
        match self.limit {
            None => Ok(DEFAULT_LIMIT),
            Some(n) if (1..=MAX_LIMIT).contains(&n) => Ok(n),
            Some(_) => Err(Problem::bad_request("invalid_limit", format!("limit is 1 to {MAX_LIMIT}"))
                .field("limit", "out_of_range", format!("1 to {MAX_LIMIT}"))),
        }
    }

    /// The position after which this page starts, as the list's key type.
    pub fn after<K: DeserializeOwned>(&self) -> Result<Option<K>, Problem> {
        let Some(cursor) = self.cursor.as_deref().filter(|c| !c.is_empty()) else {
            return Ok(None);
        };
        URL_SAFE_NO_PAD
            .decode(cursor)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map(Some)
            .ok_or_else(|| {
                Problem::bad_request("invalid_cursor", "the cursor is not one this list handed out")
                    .field("cursor", "invalid", "pass `next` from the previous page unchanged")
            })
    }
}

/// One page of a list.
#[derive(Serialize, ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// The cursor for the next page; absent on the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// The records the items refer to, each once, when `include=` asked for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<crate::v2::music::Included>,
}

impl<T> Page<T> {
    /// From rows fetched with `LIMIT limit + 1`: the extra row only says that
    /// there is more, and the page ends at the row before it.
    pub fn from_rows<K: Serialize>(mut rows: Vec<T>, limit: i64, key: impl Fn(&T) -> K) -> Self {
        let more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let next = more.then(|| rows.last().map(|last| encode(&key(last)))).flatten();
        Page { items: rows, next, included: None }
    }
}

pub fn encode<K: Serialize>(key: &K) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(key).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_says_whether_there_is_more() {
        let page = Page::from_rows(vec![1, 2, 3], 2, |n| (format!("k{n}"), *n));
        assert_eq!(page.items, vec![1, 2]);
        let q = PageQuery { limit: Some(2), cursor: page.next };
        assert_eq!(q.after::<(String, i32)>().unwrap(), Some(("k2".to_string(), 2)));
        assert!(Page::from_rows(vec![1, 2], 2, |n| *n).next.is_none());
    }

    #[test]
    fn bad_input_is_a_400() {
        assert!(PageQuery { limit: Some(0), cursor: None }.limit().is_err());
        assert!(PageQuery { limit: Some(501), cursor: None }.limit().is_err());
        assert!(PageQuery { limit: None, cursor: Some("%%".into()) }.after::<i32>().is_err());
    }
}
