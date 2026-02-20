use p2p_example::{addr_for, dial_peer, handle_inbound, run_listener, Message, Node, BASE_PORT};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};

fn addr(port: u16) -> SocketAddr {
    format!("127.0.0.1:{port}").parse().unwrap()
}

async fn bind(port: u16) -> TcpListener {
    TcpListener::bind(addr(port)).await.unwrap()
}

// ── addr_for ──────────────────────────────────────────────────────────────────

#[test]
fn addr_for_returns_correct_address() {
    assert_eq!(addr_for(0), format!("127.0.0.1:{BASE_PORT}").parse().unwrap());
    assert_eq!(addr_for(1), format!("127.0.0.1:{}", BASE_PORT + 1).parse().unwrap());
    assert_eq!(addr_for(5), format!("127.0.0.1:{}", BASE_PORT + 5).parse().unwrap());
}

// ── handle_inbound ────────────────────────────────────────────────────────────

#[tokio::test]
async fn handle_inbound_registers_peer_and_removes_on_disconnect() {
    let listener = bind(9200).await;
    let (node, _rx) = Node::new(0);
    let node_c = node.clone();
    tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.unwrap();
        handle_inbound(node_c, stream, peer_addr).await;
    });

    let stream = TcpStream::connect(addr(9200)).await.unwrap();
    let peer_addr = stream.local_addr().unwrap();
    sleep(Duration::from_millis(100)).await;
    assert!(node.peers.lock().await.contains_key(&peer_addr));

    drop(stream);
    sleep(Duration::from_millis(200)).await;
    assert!(!node.peers.lock().await.contains_key(&peer_addr));
}

#[tokio::test]
async fn handle_inbound_ignores_malformed_json() {
    let listener = bind(9202).await;
    let (node, _rx) = Node::new(0);
    let node_c = node.clone();
    tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.unwrap();
        handle_inbound(node_c, stream, peer_addr).await;
    });

    let mut stream = TcpStream::connect(addr(9202)).await.unwrap();
    sleep(Duration::from_millis(50)).await;
    stream.write_all(b"not json at all\n").await.unwrap();

    let valid = Message::new(1, "after bad json");
    let mut rx = node.tx.subscribe();
    let line = serde_json::to_string(&valid).unwrap() + "\n";
    stream.write_all(line.as_bytes()).await.unwrap();

    let result = timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap().text, "after bad json");
}

// ── gossip ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn gossip_drops_dead_peer() {
    let listener = bind(9204).await;
    let client = TcpStream::connect(addr(9204)).await.unwrap();
    let (server_stream, _) = listener.accept().await.unwrap();
    let dead_addr = client.local_addr().unwrap();

    let (node, _rx) = Node::new(0);
    let (_, write) = server_stream.into_split();
    node.peers.lock().await.insert(dead_addr, write);
    assert_eq!(node.peers.lock().await.len(), 1);

    drop(client);
    sleep(Duration::from_millis(100)).await;

    let msg = Message::new(0, "probe");
    let evicted = timeout(Duration::from_secs(3), async {
        loop {
            node.gossip(&msg).await;
            if node.peers.lock().await.is_empty() {
                return true;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(evicted.unwrap_or(false), "dead peer must be evicted");
}

#[tokio::test]
async fn gossip_with_no_peers_is_noop() {
    let (node, _rx) = Node::new(0);
    let msg = Message::new(0, "no peers");
    node.gossip(&msg).await;
}

// ── Node::send ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn node_send_delivers_to_connected_peer() {
    let listener = bind(9207).await;
    let (node, _rx) = Node::new(0);

    let client = TcpStream::connect(addr(9207)).await.unwrap();
    let (server_stream, _) = listener.accept().await.unwrap();
    let peer_addr = client.local_addr().unwrap();
    let (_, write) = server_stream.into_split();
    node.peers.lock().await.insert(peer_addr, write);

    let mut lines = BufReader::new(client).lines();
    node.send("hello peer").await;

    let line = timeout(Duration::from_secs(1), lines.next_line())
        .await.unwrap().unwrap().unwrap();
    let received: Message = serde_json::from_str(&line).unwrap();
    assert_eq!(received.text, "hello peer");
    assert_eq!(received.origin, 0);
    assert!(node.has_seen(&received.id).await);
}

#[tokio::test]
async fn send_marks_own_message_as_seen() {
    let (node, _rx) = Node::new(0);
    assert_eq!(node.seen_count().await, 0);
    node.send("mark test").await;
    assert_eq!(node.seen_count().await, 1);
}

// ── dial_peer ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dial_peer_connects_and_receives_messages() {
    const PORT: u16 = 9209;
    let (node1, _rx1) = Node::new(1);
    let node1_c = node1.clone();
    tokio::spawn(async move {
        let listener = TcpListener::bind(addr(PORT)).await.unwrap();
        loop {
            if let Ok((stream, peer_addr)) = listener.accept().await {
                tokio::spawn(handle_inbound(node1_c.clone(), stream, peer_addr));
            }
        }
    });
    sleep(Duration::from_millis(100)).await;

    let (node0, _rx0) = Node::new(0);
    let stream = TcpStream::connect(addr(PORT)).await.unwrap();
    let (read, write) = stream.into_split();
    node0.peers.lock().await.insert(addr(PORT), write);
    let node0_c = node0.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                node0_c.handle_message(msg).await;
            }
        }
    });
    sleep(Duration::from_millis(100)).await;

    let mut waiter = node1.tx.subscribe();
    node0.send("dial test message").await;

    let result = timeout(Duration::from_secs(2), waiter.recv()).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap().text, "dial test message");
    assert_eq!(node1.peers.lock().await.len(), 1);
}

#[tokio::test]
async fn dial_peer_removes_peer_on_remote_disconnect() {
    const PORT: u16 = 9211;
    let listener = TcpListener::bind(addr(PORT)).await.unwrap();
    let (node, _rx) = Node::new(0);

    let server_task = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        sleep(Duration::from_millis(500)).await;
    });

    let peer_id = (PORT - BASE_PORT) as usize;
    let node_c = node.clone();
    tokio::spawn(async move { dial_peer(node_c, peer_id).await; });

    sleep(Duration::from_millis(300)).await;
    assert_eq!(node.peers.lock().await.len(), 1, "peer should be registered");

    server_task.await.unwrap();
    sleep(Duration::from_millis(400)).await;
    assert_eq!(node.peers.lock().await.len(), 0, "peer must be removed after disconnect");
}

#[tokio::test]
async fn dial_peer_retries_until_listener_is_ready() {
    const PORT: u16 = 9213;
    let peer_id = (PORT - BASE_PORT) as usize;

    let (node, _rx) = Node::new(0);
    let node_c = node.clone();
    tokio::spawn(async move { dial_peer(node_c, peer_id).await; });

    sleep(Duration::from_millis(700)).await;

    let _listener = TcpListener::bind(addr(PORT)).await.unwrap();
    tokio::spawn(async move {
        let (_stream, _addr) = _listener.accept().await.unwrap();
        sleep(Duration::from_millis(2000)).await;
    });
    sleep(Duration::from_millis(50)).await;

    let registered = timeout(Duration::from_secs(4), async {
        loop {
            if node.peers.lock().await.len() == 1 { return true; }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(registered.unwrap_or(false), "dial_peer should connect after retrying");
}

#[tokio::test]
async fn dial_peer_reader_task_delivers_inbound_messages() {
    let peer_id = (9220_u16 - BASE_PORT) as usize;
    let listener = TcpListener::bind(addr(9220)).await.unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let msg = Message::new(99, "server push");
        let line = serde_json::to_string(&msg).unwrap() + "\n";
        stream.write_all(line.as_bytes()).await.unwrap();
        sleep(Duration::from_millis(500)).await;
    });

    let (node, _rx) = Node::new(0);
    let mut rx = node.tx.subscribe();
    let node_c = node.clone();
    tokio::spawn(async move { dial_peer(node_c, peer_id).await; });

    let result = timeout(Duration::from_secs(3), async move {
        loop {
            match rx.recv().await {
                Ok(msg) if msg.text == "server push" => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await;
    assert!(result.unwrap_or(false), "dial_peer reader task must deliver messages");
}

#[tokio::test]
async fn dial_peer_reader_task_cleans_up_on_eof() {
    let peer_id = (9221_u16 - BASE_PORT) as usize;
    let listener = TcpListener::bind(addr(9221)).await.unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        sleep(Duration::from_millis(200)).await;
    });

    let (node, _rx) = Node::new(0);
    let node_c = node.clone();
    tokio::spawn(async move { dial_peer(node_c, peer_id).await; });

    sleep(Duration::from_millis(100)).await;
    assert_eq!(node.peers.lock().await.len(), 1);
    sleep(Duration::from_millis(400)).await;
    assert_eq!(node.peers.lock().await.len(), 0, "reader task must clean up on EOF");
}

// ── run_listener ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_listener_accepts_and_delivers_message() {
    let node_id = (9215_u16 - BASE_PORT) as usize;
    let (node, _rx) = Node::new(node_id);
    tokio::spawn(run_listener(node.clone()));
    sleep(Duration::from_millis(150)).await;

    let mut stream = TcpStream::connect(addr(9215)).await.unwrap();
    sleep(Duration::from_millis(50)).await;

    let msg = Message::new(99, "via run_listener");
    let mut rx = node.tx.subscribe();
    let line = serde_json::to_string(&msg).unwrap() + "\n";
    stream.write_all(line.as_bytes()).await.unwrap();

    let result = timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap().text, "via run_listener");
}

// ── multi-hop ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn multi_hop_chain_gossip() {
    const PORTS: [u16; 3] = [9217, 9218, 9219];
    let mut nodes = Vec::new();
    for i in 0..3usize {
        let (node, _rx) = Node::new(i);
        nodes.push(node);
    }

    for i in 1..3usize {
        let node = nodes[i].clone();
        let port = PORTS[i];
        tokio::spawn(async move {
            let listener = TcpListener::bind(addr(port)).await.unwrap();
            loop {
                if let Ok((stream, peer_addr)) = listener.accept().await {
                    tokio::spawn(handle_inbound(node.clone(), stream, peer_addr));
                }
            }
        });
    }
    sleep(Duration::from_millis(150)).await;

    for (from, to) in [(0, 1), (1, 2)] {
        let stream = TcpStream::connect(addr(PORTS[to])).await.unwrap();
        let (read, write) = stream.into_split();
        nodes[from].peers.lock().await.insert(addr(PORTS[to]), write);
        let node_from = nodes[from].clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                    node_from.handle_message(msg).await;
                }
            }
        });
    }
    sleep(Duration::from_millis(200)).await;

    let mut rx2 = nodes[2].tx.subscribe();
    nodes[0].send("chain hop test").await;

    let result = timeout(Duration::from_secs(2), async move {
        loop {
            match rx2.recv().await {
                Ok(msg) if msg.text == "chain hop test" => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await;
    assert!(result.unwrap_or(false), "message must reach node 2 via node 1");
}

// ── run_node_with_input ───────────────────────────────────────────────────────

#[tokio::test]
async fn run_node_exits_cleanly_on_empty_input() {
    use p2p_example::run_node_with_input;
    let node_id = (9230_u16 - BASE_PORT) as usize;
    let total = node_id + 1;
    let empty: &[u8] = b"";
    let result = timeout(Duration::from_secs(8), run_node_with_input(node_id, total, empty)).await;
    assert!(result.is_ok(), "run_node should exit on EOF input");
}

#[tokio::test]
async fn run_node_sends_typed_message() {
    use p2p_example::run_node_with_input;
    let node_id = (9231_u16 - BASE_PORT) as usize;
    let total = node_id + 1;
    // non-empty line -> send path; blank line -> Ok(Some(_)) arm; EOF -> break
    let input: &[u8] = b"hello world\n\n";
    let result = timeout(Duration::from_secs(8), run_node_with_input(node_id, total, input)).await;
    assert!(result.is_ok(), "run_node should exit cleanly after sending");
}
