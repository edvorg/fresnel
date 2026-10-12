//! The response body with a scripted source and paused time.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream;
use http_body_util::StreamBody;
use tokio::sync::Notify;
use tokio::time::{Instant, sleep};

use super::gzip::tests::gzip;
use super::*;
use crate::channel::streams::Streams;
use crate::error::ErrorClass;

const SECOND: Duration = Duration::from_secs(1);

/// A source that gives each piece after its wait in seconds, and then ends or stays open.
fn source(pieces: Vec<(u64, Vec<u8>)>, ends: bool) -> impl Body<Data = Bytes, Error = io::Error> + Send + 'static {
    let pieces = stream::iter(pieces).then(|(wait, piece)| async move {
        sleep(Duration::from_secs(wait)).await;
        Ok(Frame::data(Bytes::from(piece)))
    });
    let end = stream::once(async move {
        if !ends {
            std::future::pending::<()>().await;
        }
        None
    });
    StreamBody::new(pieces.map(Some).chain(end).filter_map(|item| async move { item }))
}

/// Reads the body to its end. Gives the data and the error, if one ended the body.
async fn read(body: &mut ResponseBody) -> (Vec<u8>, Option<NetError>) {
    let mut data = Vec::new();
    while let Some(frame) = body.frame().await {
        match frame {
            Ok(frame) => data.extend_from_slice(&frame.into_data().expect("data")),
            Err(error) => return (data, Some(error)),
        }
    }
    (data, None)
}

fn gzip_headers() -> HeaderMap {
    HeaderMap::from_iter([
        (CONTENT_ENCODING, HeaderValue::from_static("gzip")),
        (CONTENT_LENGTH, HeaderValue::from_static("123")),
    ])
}

#[tokio::test(start_paused = true)]
async fn a_gzip_body_arrives_decoded_and_a_plain_body_arrives_as_it_is() {
    let text = br#"{"kind":"PodList","items":[]}"#.repeat(500);
    let packed = gzip(&text);
    let pieces = packed.chunks(100).map(|piece| (0, piece.to_vec())).collect();
    let mut headers = gzip_headers();
    let mut body = ResponseBody::new(source(pieces, true)).decoded(&mut headers);
    assert!(headers.is_empty(), "the headers of the coded form are gone: {headers:?}");
    let (data, error) = read(&mut body).await;
    assert!(data == text && error.is_none(), "{error:?}");

    let mut headers = HeaderMap::from_iter([(CONTENT_LENGTH, HeaderValue::from_static("7"))]);
    let mut plain = ResponseBody::new(source(vec![(0, packed.clone())], true)).decoded(&mut headers);
    assert_eq!(headers.len(), 1);
    assert!(read(&mut plain).await.0 == packed, "a body with no gzip header is not decoded");
}

#[tokio::test(start_paused = true)]
async fn a_gzip_body_that_the_connection_cut_is_an_error() {
    let packed = gzip(&br#"{"kind":"PodList"}"#.repeat(500));
    let cut = packed[..packed.len() / 2].to_vec();
    let mut body = ResponseBody::new(source(vec![(0, cut)], true)).decoded(&mut gzip_headers());
    let (_, error) = read(&mut body).await;
    let error = error.expect("the data is not complete");
    assert!(error.to_string().starts_with("the gzip data of the answer ended early"), "{error}");
    assert!(body.frame().await.is_none(), "the body is at its end after an error");
}

#[tokio::test(start_paused = true)]
async fn a_body_with_no_data_ends_at_its_idle_limit() {
    let started = Instant::now();
    let pieces = vec![(0, b"page ".to_vec()), (10, b"one".to_vec())];
    let limits = BodyLimits::new(Some(15 * SECOND), Some(120 * SECOND), started);
    let mut body = ResponseBody::new(source(pieces, false)).with_limits(limits);
    let (data, error) = read(&mut body).await;
    let error = error.expect("a time limit");
    assert_eq!((data.as_slice(), started.elapsed()), (b"page one".as_slice(), 25 * SECOND));
    assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Timeout, "the answer had no data for 15s"));
}

#[tokio::test(start_paused = true)]
async fn a_body_that_never_ends_stops_at_its_total_limit() {
    let started = Instant::now() - 2 * SECOND;
    let pieces = (0..100).map(|_| (10, b"x".to_vec())).collect();
    let limits = BodyLimits::new(Some(15 * SECOND), Some(120 * SECOND), started);
    let mut body = ResponseBody::new(source(pieces, false)).with_limits(limits);
    let (data, error) = read(&mut body).await;
    assert_eq!(data.len(), 11, "one piece each 10 s");
    assert_eq!(started.elapsed(), 120 * SECOND, "the limit counts from the start of the request");
    assert_eq!(error.expect("a time limit").to_string(), "the answer was not complete in 120s");
    assert!(BodyLimits::new(None, None, started).is_none(), "a watch has no limit");
}

#[tokio::test(start_paused = true)]
async fn the_stream_is_free_at_the_end_of_the_body_and_each_piece_counts_as_a_frame() {
    let streams = Arc::new(Streams::new(1, Arc::new(Notify::new())));
    let permit = streams.try_open().expect("a stream");
    let mut body = ResponseBody::new(source(vec![(3, b"event".to_vec())], true)).on_stream(permit);
    assert_eq!((streams.open(), streams.last_frame_age()), (1, None));
    assert!(body.frame().await.expect("a frame").is_ok());
    sleep(2 * SECOND).await;
    assert_eq!((streams.open(), streams.last_frame_age()), (1, Some(2 * SECOND)));
    assert!(body.frame().await.is_none());
    assert_eq!(streams.open(), 0, "the body is at its end and still exists");
}
