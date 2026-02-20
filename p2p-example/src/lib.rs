use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{broadcast, Mutex, RwLock},
    time::sleep,
};
use tracing::{error, info, warn};
use uuid::Uuid;

// ── Message ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub origin: usize,
    pub text: String,
}

impl Message {
    pub fn new(origin: usize, text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            origin,
            text: text.into(),
        }
    }
}

// ── Node ──────────────────────────────────────────────────────────────────────

pub struct Node {
    pub id: usize,
    pub peers: Mutex<HashMap<SocketAddr, tokio::net::tcp::OwnedWriteHalf>>,
    seen: RwLock<HashSet<String>>,
    pub tx: broadcast::Sender<Message>,
}

impl Node {
    pub fn new(id: usize) -> (Arc<Self>, broadcast::Receiver<Message>) {
        let (tx, rx) = broadcast::channel(256);
        let node = Arc::new(Self {
            id,
            peers: Mutex::new(HashMap::new()),
            seen: RwLock::new(HashSet::new()),
            tx,
        });
        (node, rx)
    }

    pub async fn seen_count(&self) -> usize {
        self.seen.read().await.len()
    }

    pub async fn has_seen(&self, id: &str) -> bool {
        self.seen.read().await.contains(id)
    }

    pub async fn handle_message(self: &Arc<Self>, msg: Message) {
        {
            let mut seen = self.seen.write().await;
            if !seen.insert(msg.id.clone()) {
                return;
            }
        }
        info!(node = self.id, from = msg.origin, text = %msg.text, "received message");
        let _ = self.tx.send(msg.clone());
        self.gossip(&msg).await;
    }

    pub async fn gossip(self: &Arc<Self>, msg: &Message) {
        let line = serde_json::to_string(msg).expect("serialize") + "\n";
        let mut peers = self.peers.lock().await;
        let mut dead = Vec::new();
        for (addr, writer) in peers.iter_mut() {
            if let Err(e) = writer.write_all(line.as_bytes()).await {
                warn!(peer = %addr, err = %e, "peer write failed, dropping");
                dead.push(*addr);
            }
        }
        for addr in dead {
            peers.remove(&addr);
        }
    }

    pub async fn send(self: &Arc<Self>, text: impl Into<String>) {
        let msg = Message::new(self.id, text);
        self.seen.write().await.insert(msg.id.clone());
        info!(node = self.id, text = %msg.text, "sending message");
        self.gossip(&msg).await;
    }
}

// ── Networking ────────────────────────────────────────────────────────────────

pub const BASE_PORT: u16 = 8000;

pub fn addr_for(node_id: usize) -> SocketAddr {
    format!("127.0.0.1:{}", BASE_PORT + node_id as u16)
        .parse()
        .unwrap()
}

pub async fn run_listener(node: Arc<Node>) {
    let addr = addr_for(node.id);
    let listener = TcpListener::bind(addr).await.expect("bind listener");
    info!(node = node.id, %addr, "listening");
    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                info!(node = node.id, peer = %peer_addr, "accepted connection");
                let node = node.clone();
                tokio::spawn(handle_inbound(node, stream, peer_addr));
            }
            Err(e) => error!("accept error: {e}"),
        }
    }
}

pub async fn handle_inbound(node: Arc<Node>, stream: TcpStream, peer_addr: SocketAddr) {
    let (read, write) = stream.into_split();
    node.peers.lock().await.insert(peer_addr, write);
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match serde_json::from_str::<Message>(&line) {
            Ok(msg) => node.handle_message(msg).await,
            Err(e) => warn!(node = node.id, err = %e, "bad message"),
        }
    }
    info!(node = node.id, peer = %peer_addr, "peer disconnected");
    node.peers.lock().await.remove(&peer_addr);
}

pub async fn dial_peer(node: Arc<Node>, peer_id: usize) {
    let peer_addr = addr_for(peer_id);
    loop {
        match TcpStream::connect(peer_addr).await {
            Ok(stream) => {
                info!(node = node.id, peer = %peer_addr, "connected to peer");
                let (read, write) = stream.into_split();
                node.peers.lock().await.insert(peer_addr, write);
                let node2 = node.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(read).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if let Ok(msg) = serde_json::from_str::<Message>(&line) {
                            node2.handle_message(msg).await;
                        }
                    }
                    info!(node = node2.id, peer = %peer_addr, "outbound peer closed");
                    node2.peers.lock().await.remove(&peer_addr);
                });
                return;
            }
            Err(_) => {
                sleep(Duration::from_millis(500)).await;
            }
        }
    }
}

// ── Application logic ─────────────────────────────────────────────────────────

/// Core node loop with an injectable input reader. Production code passes
/// tokio::io::stdin(); tests pass in-memory byte slices.
pub async fn run_node_with_input<R>(node_id: usize, total: usize, input: R)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let (node, _rx) = Node::new(node_id);

    tokio::spawn(run_listener(node.clone()));
    sleep(Duration::from_millis(200)).await;

    for peer_id in (node_id + 1)..total {
        tokio::spawn(dial_peer(node.clone(), peer_id));
    }

    sleep(Duration::from_secs(2)).await;
    info!(node = node_id, "mesh ready - type messages and press Enter. Ctrl-C to quit.");

    let node_clone = node.clone();
    tokio::spawn(async move {
        let mut rx = node_clone.tx.subscribe();
        while let Ok(msg) = rx.recv().await {
            if msg.origin != node_clone.id {
                println!("\n[node{}->node{}] {}\n> ", msg.origin, node_clone.id, msg.text);
            }
        }
    });

    let mut lines = BufReader::new(input).lines();
    print!("> ");
    let _ = tokio::io::stdout().flush().await;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) if !line.trim().is_empty() => {
                node.send(line.trim().to_string()).await;
                print!("> ");
                let _ = tokio::io::stdout().flush().await;
            }
            Ok(Some(_)) => {
                print!("> ");
                let _ = tokio::io::stdout().flush().await;
            }
            Ok(None) | Err(_) => break,
        }
    }
}

/// Production entry point — reads from real stdin.
pub async fn run_node(node_id: usize, total: usize) {
    run_node_with_input(node_id, total, tokio::io::stdin()).await
}
