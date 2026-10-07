// SPDX-License-Identifier: AGPL-3.0-or-later
//! A JSON array written row by row as the database returns it, so a list the
//! size of the catalog never sits in memory (`docs/CAPACITY.md`).

use axum::body::Body;
use bytes::Bytes;
use serde::Serialize;
use tokio::sync::mpsc::Receiver;

/// `head`, then `[item,item,…]`, then `tail`. Items arrive on `rx` (a task
/// reading the database) and go through `to_json` on the way out. An error
/// after the first byte can only cut the body short: the status is sent.
pub fn array<T, J, F>(rx: Receiver<anyhow::Result<T>>, head: Vec<u8>, tail: &'static [u8], what: &'static str, to_json: F) -> Body
where
    T: Send + 'static,
    J: Serialize,
    F: Fn(T) -> J + Send + Sync + 'static,
{
    struct State<T, F> {
        rx: Receiver<anyhow::Result<T>>,
        head: Option<Vec<u8>>,
        tail: Vec<u8>,
        first: bool,
        done: bool,
        to_json: F,
    }
    let start = State { rx, head: Some(head), tail: tail.to_vec(), first: true, done: false, to_json };
    let body = futures_util::stream::unfold(start, move |mut s| async move {
        if s.done {
            return None;
        }
        let mut buf = s.head.take().unwrap_or_default();
        match s.rx.recv().await {
            Some(Ok(item)) => {
                buf.push(if s.first { b'[' } else { b',' });
                s.first = false;
                if let Err(e) = serde_json::to_writer(&mut buf, &(s.to_json)(item)) {
                    s.done = true;
                    return Some((Err(std::io::Error::other(e)), s));
                }
                Some((Ok(Bytes::from(buf)), s))
            }
            Some(Err(e)) => {
                tracing::warn!(error = %format!("{e:#}"), what, "streamed list failed");
                s.done = true;
                Some((Err(std::io::Error::other(e.to_string())), s))
            }
            None => {
                buf.extend_from_slice(if s.first { b"[]" } else { b"]" });
                buf.extend_from_slice(&s.tail);
                s.done = true;
                Some((Ok(Bytes::from(buf)), s))
            }
        }
    });
    // Fused: the compression layer polls once more after the end, and a bare
    // `unfold` panics on that (it cost every gzip client the whole list).
    Body::from_stream(futures_util::StreamExt::fuse(body))
}

/// The usual reader: every row of `query`'s stream into the channel.
pub async fn send_all<T, S>(mut rows: S, out: tokio::sync::mpsc::Sender<anyhow::Result<T>>, what: &'static str)
where
    S: futures_util::Stream<Item = Result<T, sqlx::Error>> + Unpin,
{
    use anyhow::Context;
    use futures_util::StreamExt;
    while let Some(row) = rows.next().await {
        let row = row.context(what);
        let failed = row.is_err();
        if out.send(row).await.is_err() || failed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect(body: Body) -> String {
        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn writes_head_items_and_tail() {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            for n in [1, 2, 3] {
                tx.send(Ok(n)).await.unwrap();
            }
        });
        let body = array(rx, br#"{"x":1,"items":"#.to_vec(), b"}", "test", |n: i32| n * 10);
        assert_eq!(collect(body).await, r#"{"x":1,"items":[10,20,30]}"#);
    }

    #[tokio::test]
    async fn an_empty_list_is_still_valid_json() {
        let (tx, rx) = tokio::sync::mpsc::channel::<anyhow::Result<i32>>(1);
        drop(tx);
        assert_eq!(collect(array(rx, Vec::new(), b"", "test", |n| n)).await, "[]");
    }
}
