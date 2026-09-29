//! The real Router and QUIC streams over 127.0.0.1: everything but the
//! WebRTC lane and the relay.

use std::path::Path;
use std::time::Duration;

use agent_inject::test_support::{ServeOpts, Session, finish, loopback_sender, serve_with, upload};
use agent_inject_proto::lookup::LookupOpts;
use agent_inject_proto::{RequestHeader, Status, UPLOAD_ALPN};
use fofoca::iroh::Endpoint;
use fofoca::iroh::endpoint::Connection;
use fofoca_iroh_webrtc_transport::IceConfig;
use sha2::{Digest, Sha256};

async fn session(dir: &Path) -> Session {
    serve_with(ServeOpts {
        dir: dir.to_owned(),
        lookups: LookupOpts::loopback(),
        ice: IceConfig::host_only(),
    })
    .await
    .expect("serve")
}

async fn connect(session: &Session) -> (Endpoint, Connection) {
    let endpoint = loopback_sender().await.expect("sender");
    let conn = endpoint
        .connect(session.ticket.addr.clone(), UPLOAD_ALPN)
        .await
        .expect("connect");
    (endpoint, conn)
}

fn header(session: &Session, name: &str, upload_id: u8, size: u64) -> RequestHeader {
    RequestHeader {
        secret: session.ticket.secret,
        upload_id: [upload_id; 16],
        name: name.to_owned(),
        size,
    }
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn large_file_arrives_intact() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path()).await;
    let (_endpoint, conn) = connect(&session).await;

    let body: Vec<u8> = (0..64u32 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let response = upload(
        &conn,
        &header(&session, "big.bin", 1, body.len() as u64),
        &body,
    )
    .await
    .unwrap();

    assert_eq!(response.status, Status::Ok);
    let saved = session.saved.recv().await.unwrap();
    assert_eq!(saved, dir.path().canonicalize().unwrap().join("big.bin"));
    let on_disk = std::fs::read(&saved).unwrap();
    assert_eq!(Sha256::digest(&on_disk), Sha256::digest(&body));
    session.shutdown().await;
}

#[tokio::test]
async fn parallel_uploads_collisions_and_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path()).await;
    let (_endpoint, conn) = connect(&session).await;

    let mut tasks = Vec::new();
    for index in 0..8u8 {
        let conn = conn.clone();
        let header = header(&session, "../../same.jpg", index, 3);
        tasks.push(tokio::spawn(async move {
            upload(&conn, &header, &[index; 3]).await.unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().status, Status::Ok);
    }
    let mut saved = Vec::new();
    for _ in 0..8 {
        saved.push(session.saved.recv().await.unwrap());
    }
    assert_eq!(entries(dir.path()).len(), 8);
    assert!(entries(dir.path()).contains(&"same.jpg".to_owned()));
    assert!(entries(dir.path()).contains(&"same-8.jpg".to_owned()));
    let root = dir.path().canonicalize().unwrap();
    assert!(
        saved
            .iter()
            .all(|path| path.parent() == Some(root.as_path()))
    );
    session.shutdown().await;
}

#[tokio::test]
async fn zero_byte_file_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path()).await;
    let (_endpoint, conn) = connect(&session).await;
    let response = upload(&conn, &header(&session, "empty.txt", 1, 0), &[])
        .await
        .unwrap();
    assert_eq!(response.status, Status::Ok);
    let saved = session.saved.recv().await.unwrap();
    assert!(std::fs::read(saved).unwrap().is_empty());
    session.shutdown().await;
}

#[tokio::test]
async fn wrong_secret_closes_only_that_connection() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path()).await;

    let (_bad_endpoint, bad) = connect(&session).await;
    let mut forged = header(&session, "a.txt", 1, 1);
    forged.secret = [0u8; 32];
    let refused = upload(&bad, &forged, b"x").await.unwrap();
    assert_eq!(refused.status, Status::Unauthorized);
    tokio::time::timeout(Duration::from_secs(5), bad.closed())
        .await
        .expect("the receiver closes a connection with a wrong secret");

    let (_endpoint, good) = connect(&session).await;
    let response = upload(&good, &header(&session, "a.txt", 2, 1), b"y")
        .await
        .unwrap();
    assert_eq!(response.status, Status::Ok);
    assert_eq!(
        std::fs::read(session.saved.recv().await.unwrap()).unwrap(),
        b"y"
    );
    assert_eq!(entries(dir.path()), ["a.txt"]);
    session.shutdown().await;
}

#[tokio::test]
async fn reset_mid_body_leaves_nothing_and_the_connection_lives() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path()).await;
    let (_endpoint, conn) = connect(&session).await;

    let (mut send, _recv) = conn.open_bi().await.unwrap();
    let declared = header(&session, "partial.bin", 1, 1024 * 1024);
    send.write_all(&declared.encode().unwrap()).await.unwrap();
    send.write_all(&[1u8; 1000]).await.unwrap();
    send.reset(0u32.into()).unwrap();

    // Give the receiver time to see the reset and drop its temp file.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !entries(dir.path()).is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(entries(dir.path()).is_empty(), "{:?}", entries(dir.path()));

    let response = upload(&conn, &header(&session, "next.txt", 2, 2), b"ok")
        .await
        .unwrap();
    assert_eq!(response.status, Status::Ok);
    assert!(session.saved.recv().await.unwrap().ends_with("next.txt"));
    session.shutdown().await;
}

#[tokio::test]
async fn done_signal_finishes_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session(dir.path()).await;
    let (_endpoint, conn) = connect(&session).await;
    for (index, name) in ["a.txt", "b.txt"].into_iter().enumerate() {
        let header = header(&session, name, u8::try_from(index).unwrap(), 1);
        assert_eq!(
            upload(&conn, &header, b"x").await.unwrap().status,
            Status::Ok
        );
    }
    let response = finish(&conn, &session.ticket.secret).await.unwrap();
    assert_eq!(response.status, Status::Ok);
    assert_eq!(response.message, "2");
    tokio::time::timeout(Duration::from_secs(5), session.finished())
        .await
        .expect("the done signal wakes the session");
    assert_eq!(
        session.saved.recv().await.unwrap().file_name().unwrap(),
        "a.txt"
    );
    assert_eq!(
        session.saved.recv().await.unwrap().file_name().unwrap(),
        "b.txt"
    );
    session.shutdown().await;
}
