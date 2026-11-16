use keel_core::models::download_gguf;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn checksummed_download_resumes_and_verifies_all_bytes() {
    let root = std::env::temp_dir().join(format!("keel_resume_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("model.gguf");
    std::fs::write(destination.with_extension("gguf.part"), b"GGU").unwrap();
    let all = b"GGUFverified model";
    let rest = &all[3..];
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let n = socket.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..n])
            .to_lowercase()
            .contains("range: bytes=3-"));
        socket.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 3-{}/{}\r\nConnection: close\r\n\r\n",rest.len(),all.len()-1,all.len()).as_bytes()).await.unwrap();
        socket.write_all(rest).await.unwrap();
    });
    let digest = hex::encode(Sha256::digest(all));
    let mut progress = vec![];
    download_gguf(
        &format!("http://{address}/model"),
        &destination,
        Some(&digest),
        "test",
        |p| progress.push(p),
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), all);
    assert_eq!(progress[0].downloaded_bytes, 3);
    assert!(progress.last().unwrap().done);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn checksum_failure_never_installs_an_invalid_file() {
    let root = std::env::temp_dir().join(format!("keel_bad_download_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let destination = root.join("model.gguf");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = socket.read(&mut request).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad")
            .await
            .unwrap();
    });
    assert!(download_gguf(
        &format!("http://{address}"),
        &destination,
        Some(&"0".repeat(64)),
        "test",
        |_| {}
    )
    .await
    .is_err());
    server.await.unwrap();
    assert!(!destination.exists());
    assert!(!destination.with_extension("gguf.part").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn runtime_archive_rejects_path_traversal() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("keel_archive_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let archive = root.join("runtime.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    zip.start_file("../escape", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"no").unwrap();
    zip.finish().unwrap();
    assert!(keel_core::setup::unpack_runtime(&archive, &root.join("unpack")).is_err());
    assert!(!root.join("escape").exists());
    std::fs::remove_dir_all(root).unwrap();
}
