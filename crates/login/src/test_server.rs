//! A minimal hand-rolled HTTP/1.1 server for testing the grants against
//! real bytes on a real socket, without a mocking dependency — the shape of
//! aldwin-llm's `test_server`, cut to what a token endpoint needs. Each
//! accepted connection pops the next canned response off a shared queue,
//! and every request is kept as `METHOD /path body`, so a test can read
//! back exactly which grant was asked for, where, and with what.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub enum Canned {
    /// An HTTP status and a JSON body.
    Status(u16, String),
    /// The same, after a pause — for a caller that goes away mid-request.
    Delayed(Duration, u16, String),
    /// Accepts the connection, reads the request, then closes without
    /// answering — a transport-level failure.
    HangUp,
    /// Accepts the connection, reads the request, and never answers.
    Stall,
}

pub struct FakeServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// The requests received so far, in order, each as `METHOD /path body`.
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
    mut socket: TcpStream,
    queue: Arc<Mutex<VecDeque<Canned>>>,
    requests: Arc<Mutex<Vec<String>>>,
) {
    let Some(request) = read_request(&mut socket).await else {
        return;
    };
    requests
        .lock()
        .expect("requests lock poisoned")
        .push(request);

    let Some(response) = queue.lock().expect("queue lock poisoned").pop_front() else {
        return;
    };
    match response {
        Canned::Status(code, body) => answer(&mut socket, code, &body).await,
        Canned::Delayed(pause, code, body) => {
            tokio::time::sleep(pause).await;
            answer(&mut socket, code, &body).await;
        }
        Canned::HangUp => {
            let _ = socket.shutdown().await;
        }
        Canned::Stall => std::future::pending().await,
    }
}

async fn answer(socket: &mut TcpStream, code: u16, body: &str) {
    let head = format!(
        "HTTP/1.1 {code} status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// Reads the request line, the headers, and exactly `Content-Length` body
/// bytes — just enough HTTP/1.1 to see what was asked without racing a
/// response against a client still mid-write.
async fn read_request(socket: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        if buf.len() > 1_000_000 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let content_length = content_length(&head);
    let body_start = header_end + 4;
    while buf.len() - body_start < content_length {
        let n = socket.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    // `POST /token HTTP/1.1` → `POST /token`.
    let mut request_line = head.lines().next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default();
    let path = request_line.next().unwrap_or_default();
    let body = String::from_utf8_lossy(&buf[body_start..]);
    Some(format!("{method} {path} {body}"))
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().to_string())
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
