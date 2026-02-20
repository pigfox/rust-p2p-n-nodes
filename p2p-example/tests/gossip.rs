use p2p_example::{handle_inbound, Message, Node};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};

const TEST_BASE: u16 = 9100;

fn test_addr(id: usize) -> SocketAddr {
    format!("127.0.0.1:{}", TEST_BASE + id as u16).parse().unwrap()
}

#[tokio::test]
async fn gossip_reaches_all_nodes() {
    const N: usize = 4;
    tracing_subscriber::fmt().with_test_writer().try_init().ok();

    let mut nodes = Vec::new();
    for i in 0..N {
        let (node, _rx) = Node::new(i);
        nodes.push(node);
    }

    for i in 0..N {
        let node = nodes[i].clone();
        let addr = test_addr(i);
        tokio::spawn(async move {
            let listener = TcpListener::bind(addr).await.unwrap();
            loop {
                if let Ok((stream, peer_addr)) = listener.accept().await {
                    tokio::spawn(handle_inbound(node.clone(), stream, peer_addr));
                }
            }
        });
    }

    sleep(Duration::from_millis(200)).await;

    for i in 0..N {
        for j in (i + 1)..N {
            let peer_addr = test_addr(j);
            let stream = TcpStream::connect(peer_addr).await.unwrap();
            let (read, write) = stream.into_split();
            nodes[i].peers.lock().await.insert(peer_addr, write);
            let node_i = nodes[i].clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(read).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                        node_i.handle_message(msg).await;
                    }
                }
            });
        }
    }

    sleep(Duration::from_millis(300)).await;

    let waiters: Vec<_> = (1..N)
        .map(|i| {
            let mut rx = nodes[i].tx.subscribe();
            timeout(Duration::from_secs(2), async move {
                loop {
                    match rx.recv().await {
                        Ok(msg) if msg.text == "hello from node 0" => return true,
                        Ok(_) => continue,
                        Err(_) => return false,
                    }
                }
            })
        })
        .collect();

    nodes[0].send("hello from node 0").await;

    for (idx, waiter) in waiters.into_iter().enumerate() {
        let node_i = idx + 1;
        let received = waiter.await;
        assert!(
            received.unwrap_or(false),
            "node {node_i} did not receive the gossip message within the timeout"
        );
    }
}

#[tokio::test]
async fn gossip_does_not_loop_back_to_sender() {
    const BASE: u16 = 9120;

    fn addr(id: usize) -> SocketAddr {
        format!("127.0.0.1:{}", BASE + id as u16).parse().unwrap()
    }

    let (n0, mut rx0) = Node::new(0);
    let (n1, _rx1) = Node::new(1);

    for (node, id) in [(&n0, 0usize), (&n1, 1)] {
        let node = node.clone();
        let a = addr(id);
        tokio::spawn(async move {
            let listener = TcpListener::bind(a).await.unwrap();
            loop {
                if let Ok((stream, peer_addr)) = listener.accept().await {
                    tokio::spawn(handle_inbound(node.clone(), stream, peer_addr));
                }
            }
        });
    }

    sleep(Duration::from_millis(150)).await;

    let stream = TcpStream::connect(addr(1)).await.unwrap();
    let (read, write) = stream.into_split();
    n0.peers.lock().await.insert(addr(1), write);
    let n0c = n0.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                n0c.handle_message(msg).await;
            }
        }
    });

    sleep(Duration::from_millis(150)).await;
    n0.send("test loopback").await;

    let result = timeout(Duration::from_millis(300), rx0.recv()).await;
    assert!(result.is_err(), "node 0 own rx should not fire for its own message");
}
