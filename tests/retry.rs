//! Integration tests for the retry/backoff layer against a real (mock) HTTP server.
//! These verify what the unit tests cannot: that `ureq` actually returns `Err` on the
//! HTTP statuses IMGW nodes emit (404/422/5xx), so the retry loop re-issues the call.

use imgw_rs::client::{Client, Endpoints};
use imgw_rs::http::Backoff;
use mockito::Matcher;

fn test_client() -> Client {
    // Real agent, real endpoints (unused here), but zero backoff so tests are instant.
    Client::with(Endpoints::default(), Backoff::none())
}

#[test]
fn returns_body_on_success_with_a_single_request() {
    let mut server = mockito::Server::new();
    let m = server
        .mock("GET", "/ok")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body("hello")
        .expect(1)
        .create();

    let client = test_client();
    let body = client.get(&format!("{}/ok", server.url()), 3).unwrap();

    assert_eq!(body, "hello");
    m.assert(); // exactly one request
}

#[test]
fn retries_then_gives_up_after_max_attempts_on_persistent_5xx() {
    let mut server = mockito::Server::new();
    let m = server
        .mock("GET", "/down")
        .match_query(Matcher::Any)
        .with_status(503)
        .expect(3) // the retry loop must issue exactly `tries` requests
        .create();

    let client = test_client();
    let r = client.get(&format!("{}/down", server.url()), 3);

    assert!(r.is_err());
    m.assert();
}

#[test]
fn recovers_when_a_later_attempt_succeeds() {
    let mut server = mockito::Server::new();
    // First two attempts 500, then a 200 — mockito serves mocks in creation order,
    // consuming each once its expected count is reached.
    let fail = server
        .mock("GET", "/flaky")
        .match_query(Matcher::Any)
        .with_status(500)
        .expect(2)
        .create();
    let ok = server
        .mock("GET", "/flaky")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body("recovered")
        .expect(1)
        .create();

    let client = test_client();
    let body = client.get(&format!("{}/flaky", server.url()), 5).unwrap();

    assert_eq!(body, "recovered");
    fail.assert();
    ok.assert();
}

#[test]
fn retries_on_404_since_imgw_nodes_transiently_404() {
    let mut server = mockito::Server::new();
    let m = server
        .mock("GET", "/node")
        .match_query(Matcher::Any)
        .with_status(404)
        .expect(4)
        .create();

    let client = test_client();
    let r = client.get(&format!("{}/node", server.url()), 4);

    assert!(r.is_err());
    m.assert();
}
