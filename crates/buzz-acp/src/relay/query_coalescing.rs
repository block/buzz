//! Coalesce identical in-flight HTTP bridge queries.
//!
//! This module deliberately coordinates only active requests. A completed
//! result is removed from the registry immediately, so query coalescing does
//! not become a response cache.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::watch;

use super::{RelayError, RestClient};

const REST_QUERY_FLIGHT_CAP: usize = 256;

#[derive(Clone, Debug)]
enum QueryFlightError {
    Http(String),
}

#[derive(Clone, Debug)]
enum QueryFlightResult {
    Complete(Result<Value, QueryFlightError>),
    LeaderCancelled,
}

struct QueryFlight {
    result_tx: watch::Sender<Option<QueryFlightResult>>,
}

/// A leader owns one in-flight identical `/query`. Dropping it removes the
/// flight and wakes followers so cancellation cannot strand them forever.
struct QueryFlightGuard {
    key: String,
    flight: Arc<QueryFlight>,
    completed: bool,
}

static REST_QUERY_FLIGHTS: OnceLock<Mutex<HashMap<String, Arc<QueryFlight>>>> = OnceLock::new();

fn rest_query_flights() -> &'static Mutex<HashMap<String, Arc<QueryFlight>>> {
    REST_QUERY_FLIGHTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_unpoisoned<T>(mutex: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn execute_query(client: &RestClient, body_bytes: &[u8]) -> Result<Value, RelayError> {
    let response = client.bridge_post("/query", body_bytes).await?;
    response
        .json()
        .await
        .map_err(|error| RelayError::Http(error.to_string()))
}

// `execute_query` currently returns only `RelayError::Http`: `bridge_post`
// maps transport/status failures to `Http`, and response JSON parsing does the
// same. Keep the fallback for future changes without changing the registry's
// stored error shape.
fn query_flight_error(error: &RelayError) -> QueryFlightError {
    match error {
        RelayError::Http(message) => QueryFlightError::Http(message.clone()),
        other => QueryFlightError::Http(other.to_string()),
    }
}

impl QueryFlightGuard {
    fn finish(mut self, result: &Result<Value, RelayError>) {
        self.completed = true;
        {
            let mut flights = lock_unpoisoned(rest_query_flights());
            if flights
                .get(&self.key)
                .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
            {
                flights.remove(&self.key);
            }
        }
        self.flight
            .result_tx
            .send_replace(Some(QueryFlightResult::Complete(
                result
                    .as_ref()
                    .map(Value::clone)
                    .map_err(query_flight_error),
            )));
    }
}

impl Drop for QueryFlightGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        {
            let mut flights = lock_unpoisoned(rest_query_flights());
            if flights
                .get(&self.key)
                .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
            {
                flights.remove(&self.key);
            }
        }
        self.flight
            .result_tx
            .send_replace(Some(QueryFlightResult::LeaderCancelled));
    }
}

impl RestClient {
    fn query_identity_key(&self) -> String {
        use sha2::{Digest, Sha256};

        let mut auth_tag_hasher = Sha256::new();
        match &self.auth_tag_json {
            None => auth_tag_hasher.update([0]),
            Some(auth_tag) => {
                auth_tag_hasher.update([1]);
                auth_tag_hasher.update(auth_tag.as_bytes());
            }
        }
        let auth_tag_hash = hex::encode(auth_tag_hasher.finalize());
        format!(
            "{}|{}|{auth_tag_hash}",
            self.base_url,
            self.keys.public_key().to_hex()
        )
    }

    fn query_flight_key(&self, body_bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};

        format!(
            "{}|{}",
            self.query_identity_key(),
            hex::encode(Sha256::digest(body_bytes))
        )
    }

    pub(super) async fn query_json(&self, body_bytes: Vec<u8>) -> Result<Value, RelayError> {
        let key = self.query_flight_key(&body_bytes);
        loop {
            let registration = {
                let mut flights = lock_unpoisoned(rest_query_flights());
                if let Some(flight) = flights.get(&key) {
                    Some((flight.clone(), false))
                } else if flights.len() >= REST_QUERY_FLIGHT_CAP {
                    None
                } else {
                    let (result_tx, _result_rx) = watch::channel(None);
                    let flight = Arc::new(QueryFlight { result_tx });
                    flights.insert(key.clone(), flight.clone());
                    Some((flight, true))
                }
            };
            let Some((flight, leader)) = registration else {
                return execute_query(self, &body_bytes).await;
            };

            if leader {
                let guard = QueryFlightGuard {
                    key: key.clone(),
                    flight,
                    completed: false,
                };
                let result = execute_query(self, &body_bytes).await;
                guard.finish(&result);
                return result;
            }

            let mut result_rx = flight.result_tx.subscribe();
            loop {
                if let Some(result) = result_rx.borrow().clone() {
                    match result {
                        QueryFlightResult::Complete(result) => {
                            return result.map_err(|error| match error {
                                QueryFlightError::Http(message) => RelayError::Http(message),
                            });
                        }
                        QueryFlightResult::LeaderCancelled => break,
                    }
                }
                if result_rx.changed().await.is_err() {
                    break;
                }
            }
            // The leader was cancelled. Re-enter the registry and become a
            // leader if no replacement flight has claimed this key yet.
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures_util::future::join_all;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn read_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        let body_start = loop {
            let read = socket.read(&mut chunk).await.expect("read query request");
            assert!(read > 0, "query request ended before headers");
            request.extend_from_slice(&chunk[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let content_length: usize = request[..body_start]
            .split(|byte| *byte == b'\n')
            .find_map(|line| {
                let line = line.strip_suffix(b"\r")?;
                let colon = line.iter().position(|byte| *byte == b':')?;
                let name = &line[..colon];
                let value = &line[colon + 1..];
                (name.eq_ignore_ascii_case(b"content-length"))
                    .then(|| std::str::from_utf8(value).ok()?.trim().parse().ok())
                    .flatten()
            })
            .expect("query request content length");
        while request.len() < body_start + content_length {
            let read = socket.read(&mut chunk).await.expect("read query body");
            assert!(read > 0, "query request ended before body");
            request.extend_from_slice(&chunk[..read]);
        }
        request[body_start..body_start + content_length].to_vec()
    }

    async fn test_client(
        response_body: &'static str,
        response_delay: Duration,
    ) -> (
        RestClient,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<Vec<u8>>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind query test server");
        let base_url = format!(
            "http://{}",
            listener.local_addr().expect("query test server address")
        );
        let requests = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let server_requests = requests.clone();
        let server_bodies = bodies.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let requests = server_requests.clone();
                let bodies = server_bodies.clone();
                tokio::spawn(async move {
                    let body = read_request(&mut socket).await;
                    requests.fetch_add(1, Ordering::SeqCst);
                    bodies.lock().expect("record query body").push(body);
                    tokio::time::sleep(response_delay).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        let client = RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        };
        (client, requests, bodies, server)
    }

    async fn wait_for_requests(requests: &AtomicUsize, count: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while requests.load(Ordering::SeqCst) < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("query reached test server");
    }

    #[tokio::test]
    async fn coalesces_overlapping_identical_reads_without_caching_completed_reads() {
        let (client, requests, _bodies, server) =
            test_client("[]", Duration::from_millis(50)).await;
        let queries = (0..30).map(|_| client.query(&[])).collect::<Vec<_>>();
        let results = tokio::time::timeout(Duration::from_secs(3), join_all(queries))
            .await
            .expect("coalesced queries should be bounded");
        for result in results {
            assert_eq!(result.expect("coalesced query"), json!([]));
        }
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        assert_eq!(client.query(&[]).await.expect("fresh query"), json!([]));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn isolates_flights_by_relay_signer_auth_and_filter() {
        let (client, requests, _bodies, server) =
            test_client("[]", Duration::from_millis(50)).await;
        let mut other_signer = client.clone();
        other_signer.keys = nostr::Keys::generate();
        let mut empty_auth = client.clone();
        empty_auth.auth_tag_json = Some(String::new());
        let mut other_auth = client.clone();
        other_auth.auth_tag_json = Some("different-membership-delegation".into());
        let filters = [nostr::Filter::new().limit(1)];

        let (first, second, third, fourth, fifth) = tokio::join!(
            client.query(&[]),
            other_signer.query(&[]),
            empty_auth.query(&[]),
            client.query(&filters),
            other_auth.query(&[]),
        );
        for result in [first, second, third, fourth, fifth] {
            result.expect("isolated query");
        }
        assert_eq!(requests.load(Ordering::SeqCst), 5);
        server.abort();

        let (first_client, first_requests, _first_bodies, first_server) =
            test_client("[]", Duration::from_millis(50)).await;
        let (second_client, second_requests, _second_bodies, second_server) =
            test_client("[]", Duration::from_millis(50)).await;
        let mut same_identity = second_client;
        same_identity.keys = first_client.keys.clone();
        let (first, second) = tokio::join!(first_client.query(&[]), same_identity.query(&[]));
        first.expect("first relay query");
        second.expect("second relay query");
        assert_eq!(first_requests.load(Ordering::SeqCst), 1);
        assert_eq!(second_requests.load(Ordering::SeqCst), 1);
        first_server.abort();
        second_server.abort();
    }

    #[tokio::test]
    async fn query_and_query_raw_join_when_their_wire_bodies_match() {
        let (client, requests, bodies, server) = test_client("[]", Duration::from_millis(50)).await;
        let (query, raw_query) = tokio::join!(client.query(&[]), client.query_raw(&[]));
        query.expect("typed query");
        raw_query.expect("raw query");
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            bodies.lock().expect("read query bodies").as_slice(),
            &[b"[]".to_vec()]
        );
        server.abort();
    }

    #[tokio::test]
    async fn leader_cancellation_releases_followers_for_a_replacement_request() {
        let (client, requests, _bodies, server) = test_client("[]", Duration::from_secs(1)).await;
        let leader = tokio::spawn({
            let client = client.clone();
            async move { client.query(&[]).await }
        });
        wait_for_requests(&requests, 1).await;
        let follower = tokio::spawn({
            let client = client.clone();
            async move { client.query(&[]).await }
        });
        let key = client.query_flight_key(b"[]");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !lock_unpoisoned(rest_query_flights())
                .get(&key)
                .is_some_and(|flight| flight.result_tx.receiver_count() > 0)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("follower registered");
        leader.abort();
        let result = tokio::time::timeout(Duration::from_secs(3), follower)
            .await
            .expect("follower must wake after leader cancellation")
            .expect("follower task must finish")
            .expect("replacement leader query must succeed");
        assert_eq!(result, json!([]));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn followers_receive_the_same_http_error_as_the_leader() {
        let (client, requests, _bodies, server) =
            test_client("not-json", Duration::from_millis(50)).await;
        let (leader, follower) = tokio::join!(client.query(&[]), client.query(&[]));
        let leader_message = match leader {
            Err(RelayError::Http(message)) => message,
            other => panic!("expected leader HTTP error, got {other:?}"),
        };
        let follower_message = match follower {
            Err(RelayError::Http(message)) => message,
            other => panic!("expected follower HTTP error, got {other:?}"),
        };
        assert_eq!(leader_message, follower_message);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
