use ruknet::{app::RukMessageContext, Peer};

#[tokio::test]
async fn peer() {
    let mut server = Peer::new("127.0.0.1:19132", "ping response").await.unwrap();
    let mut client = Peer::new("127.0.0.1:19134", "ping response").await.unwrap();

    server.listen(10).await.unwrap();
    client.listen(1).await.unwrap();

    client.connect("127.0.0.1:19132", 3, 2000, None).unwrap();

    let timeout_duration = tokio::time::Duration::from_secs(5);

    let result = tokio::time::timeout(timeout_duration, async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;

            if let Some(msg) = client.recv() {
                if let RukMessageContext::ConnectionRequestAccepted = msg.context {
                    // Connection request is accepted
                    break;
                }
            }
        }
    });

    result.await.unwrap();
}
