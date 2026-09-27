//! A minimal HTTP/1.1 server that answers each connection with the next
//! canned response. A retry opens a fresh connection, so a queued sequence
//! scripts a retry over real TCP. Each request's head is kept for assertions.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One response the server gives, to one connection, in queue order.
#[derive(Debug)]
pub enum Canned {
    /// A status with a JSON body.
    Status(u16, String),
    /// 200 with the raw SSE text as body.
    Sse(String),
    /// 200, writes the SSE prefix, then holds the connection open silently;
    /// for idle-timeout tests.
    SseThenStall(String),
    /// Closes without a response: a transport-level failure.
    HangUp,
}

/// A running fake server. It lives until the test's runtime shuts down.
#[derive(Debug)]
pub struct FakeServer {
    /// A free port on 127.0.0.1.
    pub addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    /// The server's URL with `path` appended, for a client's endpoint.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// The request line and headers of every request so far, in order.
    ///
    /// # Panics
    ///
    /// If a connection handler panicked holding the request log's lock.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("requests lock poisoned")
            .clone()
    }
}

/// Starts a server answering each connection with the next of `responses`;
/// once the queue is empty, connections are closed. Needs a tokio runtime.
///
/// # Panics
///
/// If no local port can be bound or tokio cannot adopt the listener.
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

/// Reads the head and `Content-Length` body bytes, so no response races a
/// client still writing; returns the head.
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
