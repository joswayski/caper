use std::{process::Command, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn valkey_tls_initializes_crypto_before_connecting() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut first_byte = [0];
            let received = stream.peek(&mut first_byte).await.unwrap_or_default();
            (received, first_byte[0])
        })
        .await
        .expect("the API must attempt a TLS connection")
    });
    // A fresh process catches provider initialization bugs that a test suite's
    // previously constructed HTTP/SQL clients can hide. No external service or
    // credential is used; the peer closes as soon as it sees the TLS handshake.
    let result = Command::new(env!("CARGO_BIN_EXE_caper-api"))
        .env_clear()
        .env(
            "VALKEY_URL",
            format!("rediss://test:test-password@{address}"),
        )
        .output()
        .unwrap();
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.status.code(), Some(1), "{output}");
    assert!(output.contains("VALKEY_URL is invalid or shared media state is unavailable"));
    assert!(!output.contains("test-password"));
    assert_eq!(peer.await.unwrap(), (1, 22), "expected TLS ClientHello");
}
