//! A minimal hand-rolled HTTP/1.1 server for testing `AnthropicClient`
//! against real bytes on a real socket, without a mocking dependency. Each
//! accepted connection pops the next canned response off a shared queue —
//! since a retried request opens a fresh connection, queuing
//! `[Status(503, ..), Sse(success)]` tests "fails once, retries, succeeds"
//! for real over TCP. Every request's head — its request line and headers —
//! is kept, so a test can read back what was sent with it.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub enum Canned {
    /// Non-streaming HTTP status + JSON body.
    Status(u16, String),
    /// 200 OK, `Content-Type: text/event-stream`, body is the raw SSE text.
    Sse(String),
    /// 200 OK + SSE headers, writes `prefix` then holds the connection open
    /// forever without writing more — for idle-timeout tests under
    /// `tokio::time::pause()`.
    SseThenStall(String),
    /// Accepts the connection, reads the request, then closes without
    /// writing a response at all (simulates a transport-level failure).
    HangUp,
}

pub struct FakeServer {
    pub addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// The head of every request received so far, in order — the request
    /// line and the headers, as sent.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("requests lock poisoned")
            .clone()
    }
}

pub fn spawn(responses: Vec<Canned>) -> FakeServer {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a local test port");
    std_listener.set_nonblocking(true).expect("set nonblocking");
    let addr = std_listener.local_addr().expect("local addr");
    let listener = TcpListener::from_std(std_listener).expect("adopt into tokio");
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let requests = Arc::new(Mutex::new(Vec::new()));

    let seen = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(handle_connection(socket, queue.clone(), seen.clone()));
        }
    });

    FakeServer { addr, requests }
}

async fn handle_connection(
    mut socket: tokio::net::TcpStream,
    queue: Arc<Mutex<VecDeque<Canned>>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let Some(head) = drain_request(&mut socket).await else {
        return;
    };
    requests.lock().expect("requests lock poisoned").push(head);

    let Some(response) = queue.lock().expect("queue lock poisoned").pop_front() else {
        return;
    };
    match response {
        Canned::Status(code, body) => {
            let head = format!("HTTP/1.1 {code} status\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n");
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
        Canned::Sse(body) => {
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
        Canned::SseThenStall(prefix) => {
            let head =
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(prefix.as_bytes()).await;
            std::future::pending::<()>().await;
        }
        Canned::HangUp => {
            let _ = socket.shutdown().await;
        }
    }
}

/// Reads the request line, headers, and (if declared) exactly
/// `Content-Length` body bytes — just enough HTTP/1.1 to avoid racing a
/// response against a client still mid-write — and returns the head.
async fn drain_request(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find_header_end(&buf) {
            break end;
        }
        if buf.len() > 1_000_000 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let content_length = parse_content_length(&head);
    let mut have_body = buf.len() - (header_end + 4);
    while have_body < content_length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        have_body += n;
    }
    Some(head)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().to_string())
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
