use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::{sleep, timeout, Duration},
};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_p2p-example"))
}

#[tokio::test]
async fn exits_nonzero_with_no_args() {
    let out = bin().output().await.unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Usage"), "got: {stderr}");
}

#[tokio::test]
async fn exits_nonzero_when_node_id_equals_total() {
    let out = bin().args(["2", "2"]).output().await.unwrap();
    assert!(!out.status.success());
}

#[tokio::test]
async fn exits_nonzero_when_node_id_exceeds_total() {
    let out = bin().args(["5", "3"]).output().await.unwrap();
    assert!(!out.status.success());
}

#[tokio::test]
async fn two_nodes_exchange_message() {
    let mut node0 = bin()
        .args(["0", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut node1 = bin()
        .args(["1", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    sleep(Duration::from_secs(3)).await;

    node0
        .stdin.as_mut().unwrap()
        .write_all(b"hello from node0\n").await.unwrap();

    let stdout1 = node1.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout1).lines();

    let received = timeout(Duration::from_secs(5), async move {
        while let Ok(Some(line)) = lines.next_line().await {
            if line.contains("hello from node0") { return true; }
        }
        false
    })
    .await;

    node0.kill().await.ok();
    node1.kill().await.ok();

    assert!(received.unwrap_or(false), "message should appear on node 1 stdout");
}
