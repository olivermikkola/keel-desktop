use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub async fn server(
    responses: Vec<(String, String, Duration)>,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (content_type, response, delay) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end;
            loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = index + 4;
                    break;
                }
            }
            let header = String::from_utf8_lossy(&bytes[..header_end]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < header_end + length {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            requests.push(if length > 0 {
                serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
            } else {
                Value::Null
            });
            tokio::time::sleep(delay).await;
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(response.as_bytes()).await;
        }
        requests
    });
    (format!("http://{address}"), handle)
}
