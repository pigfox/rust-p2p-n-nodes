# rust-p2p-n-nodes

A Rust P2P example with N nodes communicating over TCP.

## Install

```bash
cargo install cargo-tarpaulin
```

## Testing

```bash
# All tests + coverage report
cargo tarpaulin --out Stdout

# Or via the test script
./test.sh

# All tests without coverage
cargo test

# Individual test files
cargo test --test gossip    # tests/gossip.rs only
cargo test --test message   # tests/message.rs only
cargo test --test network   # tests/network.rs only
cargo test --test cli       # tests/cli.rs only

# Show tracing output while testing
cargo test -- --nocapture
```

## Running

Each node gets its own terminal. The arguments are `<node_id> <total_nodes>`.

### 3-node cluster

```bash
# Terminal 1
cargo run -- 0 3

# Terminal 2
cargo run -- 1 3

# Terminal 3
cargo run -- 2 3
```

Once all nodes print `mesh ready`, type a message in any terminal and it will appear in the others.

### N-node cluster

Any N works — just run one instance per node, all with the same total:

```bash
cargo run -- 0 10
cargo run -- 1 10
# ... up to
cargo run -- 9 10
```

## Practical limits

| Limit | Detail |
|---|---|
| **Ports** | Node `i` listens on `8000 + i`. A 1000-node cluster uses ports 8000–8999. |
| **Connections** | Full mesh: N nodes create N×(N-1)/2 persistent TCP connections. Scales to hundreds of nodes on a single machine. |
