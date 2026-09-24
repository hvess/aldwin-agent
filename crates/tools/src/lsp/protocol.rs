//! LSP's JSON-RPC framing: `Content-Length: N\r\n\r\n<N bytes of UTF-8 JSON>`.
//! No message shapes live here — just the byte-level envelope.

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).expect("serde_json::Value always serialises");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// `Ok(None)` on a clean EOF before any header line — the normal way a
/// language server's stdout ends when it exits.
pub async fn read_message<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    let mut saw_any_line = false;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return if saw_any_line {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF mid-headers",
                ))
            } else {
                Ok(None)
            };
        }
        saw_any_line = true;

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line ends the header block
        }
        if let Some(value) = trimmed.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().ok();
        }
    }

    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "message had no Content-Length header",
        )
    })?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn round_trips_a_message_through_an_in_memory_duplex_pipe() {
        let (mut client, server) = tokio::io::duplex(4096);
        let mut server_reader = BufReader::new(server);

        let sent = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"a":1}});
        write_message(&mut client, &sent).await.unwrap();

        let received = read_message(&mut server_reader).await.unwrap().unwrap();
        assert_eq!(received, sent);
    }

    #[tokio::test]
    async fn reads_two_back_to_back_messages_off_the_same_stream() {
        let (mut client, server) = tokio::io::duplex(4096);
        let mut server_reader = BufReader::new(server);

        write_message(&mut client, &json!({"a": 1})).await.unwrap();
        write_message(&mut client, &json!({"b": 2})).await.unwrap();

        assert_eq!(
            read_message(&mut server_reader).await.unwrap(),
            Some(json!({"a": 1}))
        );
        assert_eq!(
            read_message(&mut server_reader).await.unwrap(),
            Some(json!({"b": 2}))
        );
    }

    #[tokio::test]
    async fn clean_eof_before_any_bytes_is_none_not_an_error() {
        let (client, server) = tokio::io::duplex(4096);
        drop(client);
        let mut server_reader = BufReader::new(server);
        assert_eq!(read_message(&mut server_reader).await.unwrap(), None);
    }

    #[tokio::test]
    async fn missing_content_length_header_is_an_error() {
        let (mut client, server) = tokio::io::duplex(4096);
        let mut server_reader = BufReader::new(server);
        client.write_all(b"X-Custom: 1\r\n\r\n").await.unwrap();
        drop(client);
        let err = read_message(&mut server_reader).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
