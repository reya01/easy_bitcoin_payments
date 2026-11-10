//! End-to-end webhook delivery test: a real loopback TCP listener plays the
//! store, the emitter posts to it, and the captured request is verified the
//! way a store would verify it — HMAC over the exact bytes received, the
//! timestamp window, the event and idempotency headers, payload integrity,
//! and rejection of a tampered payload. No network beyond loopback.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use easy_bitcoin_payments::webhook::{verify_signature, WebhookEmitter, WebhookError};

/// One HTTP request captured off the wire.
struct CapturedRequest {
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn header<'a>(request: &'a CapturedRequest, name: &str) -> &'a str {
    request
        .headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .unwrap_or("")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Read one full HTTP request (headers plus Content-Length body) from the
/// stream, then answer with `status` and hand the request back.
fn read_one_request(stream: &mut std::net::TcpStream, status: &str) -> CapturedRequest {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];

    let header_end = loop {
        if let Some(pos) = find(&buffer, b"\r\n\r\n") {
            break pos;
        }
        let read = stream.read(&mut chunk).expect("read request chunk");
        assert!(read > 0, "connection closed before the headers ended");
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut body = buffer[header_end + 4..].to_vec();

    let mut lines = head.lines();
    let method = lines
        .next()
        .expect("request line")
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }

    while body.len() < content_length {
        let read = stream.read(&mut chunk).expect("read request body");
        assert!(read > 0, "connection closed before the body ended");
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    let response = if let Some((code, content_type)) = status.split_once('|') {
        format!("HTTP/1.1 {code}\r\ncontent-type: {content_type}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
    } else {
        format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
    };
    stream
        .write_all(response.as_bytes())
        .expect("write response");
    stream.flush().expect("flush response");

    CapturedRequest {
        method,
        headers,
        body,
    }
}

/// Spawn a loopback listener that captures one request per connection and
/// forwards it over a channel. `status` is the HTTP status line it answers
/// with. Returns the URL to POST to.
fn spawn_store(status: &str) -> (String, mpsc::Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let addr = listener.local_addr().expect("local address");
    let (tx, rx) = mpsc::channel();

    let status = status.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let request = read_one_request(&mut stream, &status);
            if tx.send(request).is_err() {
                break;
            }
        }
    });

    (format!("http://{addr}/webhook"), rx)
}

fn timestamp_from(signature: &str) -> i64 {
    signature
        .split(',')
        .find_map(|part| part.trim().strip_prefix("t="))
        .and_then(|t| t.parse().ok())
        .expect("t= timestamp in the signature header")
}

#[tokio::test]
async fn emitter_delivers_a_verifiable_signed_webhook() {
    let (url, store) = spawn_store("200 OK");
    let emitter = WebhookEmitter::new(5);
    let secret = "whsec_e2e_test";
    let event = "invoice.paid";
    let idempotency_key = "inv_e2e:invoice.paid:42000";

    let payload = serde_json::json!({
        "event": "invoice.paid",
        "invoice_id": "inv_e2e",
        "store_id": "sto_e2e",
        "amount_sats": 42_000,
        "paid_sats": 42_000
    });

    emitter
        .emit(&url, secret, event, idempotency_key, &payload)
        .await
        .expect("webhook delivery should succeed");

    let request = store
        .recv_timeout(Duration::from_secs(5))
        .expect("the store captured one request");

    // The request is a POST carrying the expected headers.
    assert_eq!(request.method, "POST");
    assert_eq!(header(&request, "content-type"), "application/json");
    assert_eq!(header(&request, "x-ebp-event"), event);
    assert_eq!(header(&request, "x-ebp-idempotency-key"), idempotency_key);

    // Payload integrity: the bytes on the wire are exactly the payload.
    let received: serde_json::Value =
        serde_json::from_slice(&request.body).expect("body is the payload JSON");
    assert_eq!(received, payload);

    // The signature verifies against the bytes the store received, exactly
    // the way a store checks it (HMAC over "<t>.<body>" within the window).
    let signature = header(&request, "x-ebp-signature");
    assert!(signature.starts_with("t="));
    let timestamp = timestamp_from(signature);
    assert!(verify_signature(
        secret,
        signature,
        &request.body,
        timestamp,
        300
    ));

    // A tampered payload must fail the same verification.
    let mut tampered = request.body.clone();
    tampered[0] ^= 0x20;
    assert!(!verify_signature(
        secret, signature, &tampered, timestamp, 300
    ));
}

#[tokio::test]
async fn emitter_surfaces_http_failures() {
    let (url, _store) = spawn_store("500 Internal Server Error");
    let emitter = WebhookEmitter::new(5);
    let secret = "whsec_e2e_test";
    let payload = serde_json::json!({"event": "invoice.paid"});

    let err = emitter
        .emit(&url, secret, "invoice.paid", "key", &payload)
        .await
        .expect_err("a 500 response must surface as an error");

    match err {
        WebhookError::Status(code) => assert_eq!(code, 500),
        other => panic!("expected an HTTP status error, got: {other}"),
    }
}

#[tokio::test]
async fn a_web_page_answer_is_not_counted_as_delivered() {
    // A wrong address (say a shop's home page) answers 200 with HTML. That is
    // not a receiver of payment notices, so it must not read as "delivered".
    let (url, _store) = spawn_store("200 OK|text/html; charset=UTF-8");
    let emitter = WebhookEmitter::new(5);
    let payload = serde_json::json!({"event": "invoice.paid"});

    let err = emitter
        .emit(&url, "whsec_e2e_test", "invoice.paid", "key", &payload)
        .await
        .expect_err("an HTML 200 must surface as an error");
    assert!(matches!(err, WebhookError::WebPage), "got: {err}");

    // A JSON answer (what the WooCommerce plugin sends) is delivered.
    let (url, _store) = spawn_store("200 OK|application/json");
    emitter
        .emit(&url, "whsec_e2e_test", "invoice.paid", "key", &payload)
        .await
        .expect("a JSON 200 is delivered");
}
