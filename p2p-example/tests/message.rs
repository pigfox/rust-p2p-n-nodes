use p2p_example::{Message, Node};

#[test]
fn message_round_trips_through_json() {
    let msg = Message::new(3, "test payload");
    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: Message = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(msg.id, decoded.id);
    assert_eq!(decoded.origin, 3);
    assert_eq!(decoded.text, "test payload");
}

#[test]
fn message_ids_are_unique() {
    let a = Message::new(0, "ping");
    let b = Message::new(0, "ping");
    assert_ne!(a.id, b.id);
}

#[test]
fn message_preserves_origin() {
    for origin in [0, 1, 42, 255] {
        let msg = Message::new(origin, "x");
        assert_eq!(msg.origin, origin);
    }
}

#[tokio::test]
async fn duplicate_message_is_silently_dropped() {
    let (node, _rx) = Node::new(0);
    let msg = Message::new(1, "dup test");
    node.handle_message(msg.clone()).await;
    assert!(node.has_seen(&msg.id).await);
    assert_eq!(node.seen_count().await, 1);
    node.handle_message(msg.clone()).await;
    assert_eq!(node.seen_count().await, 1);
}

#[tokio::test]
async fn distinct_messages_are_both_accepted() {
    let (node, _rx) = Node::new(0);
    let a = Message::new(1, "first");
    let b = Message::new(1, "second");
    node.handle_message(a.clone()).await;
    node.handle_message(b.clone()).await;
    assert_eq!(node.seen_count().await, 2);
    assert!(node.has_seen(&a.id).await);
    assert!(node.has_seen(&b.id).await);
}
