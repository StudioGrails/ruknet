# RukNet

RukNet is the first perfect reimplementation of the RakNet protocol in Rust. It provides a robust and efficient networking library for Rust developers requiring RakNet compatibility.

## Overview

RukNet implements most features of the RakNet protocol while intentionally omitting certain features such as:

- `security`: Encryption and decryption of packets. This feature is planned for future implementation but is not a current priority.
- `receipt`: Ack of messages for application layer. This feature is not implemented as it is deemed unnecessary.

Other notable improvements include enhanced performance using optimized algorithms and a single-threaded design.

## Feature

- `RakNet Protocol Compatibility`: Supports core RakNet messaging and connection mechanisms.
- `Asynchronous Networking`: Built on top of Tokio for async operations.
- `Customizable Congestion Control`: Choose between UDT and Sliding Window congestion control algorithms.
- `Configurable Logging`: Enable debug logging for detailed information.

## Installation

To use RukNet in your project, add the following to your `Cargo.toml`:

```toml
[dependencies]
ruknet = "0.1.2"
```

## Usage

> [!NOTE]
> RukNet is still under active development. Please refer to the source code for more detailed insights.

### Server

Simple example of a server that listens for incoming connections, and sends a "ping response" in UnconnectedPong message when a client sends an UnconnectedPing message.

In Minecraft Bedrock Edition, this is used to determine the latency between the client and the server and to display the server's MOTD.

```rust
use ruknet::Peer;

#[tokio::main]
fn main() {
    let mut peer = Peer::new("127.0.0.1:19132", "ping response").await.unwrap();
    // listen with maximum number of connections set to 10
    peer.listen(10).await.unwrap();

    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        
        // receive up to 32 messages
        let msgs = peer.recv_many(32);

        for msg in msgs {
            println!("Received: {}", msg);
        }
    }
}
```

### Client

```rust
use ruknet::Peer;

#[tokio::main]
async fn main() {
    let mut peer = Peer::new("(your local address)", "ping response").await.unwrap();
    peer.listen(1).await.unwrap();

    peer.connect("(server address)").unwrap();

    loop {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // receive up to 32 messages
        let msgs = peer.recv_many(32);

        for msg in msgs {
            println!("Received: {}", msg);
        }
    }
}
```

## Configuration

- `debug`: Enables debug logging. Disabled by default.
- `udt`: Toggles the use of UDT congestion control instead of the default Sliding Window algorithm.

## Contributing

Contributions are welcome! Feel free to open an issue or submit a pull request.

## License

RukNet is licensed under the MIT license. See [LICENSE](LICENSE) for more details.