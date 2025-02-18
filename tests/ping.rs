use ruknet::{app::RukMessageContext, Peer};

#[tokio::test]
async fn ping() {
    let ping_response = "ping response";

    let mut server = Peer::new("127.0.0.1:19132", ping_response).await.unwrap();
    let mut client = Peer::new("127.0.0.1:19134", "").await.unwrap();

    server.listen(10).await.unwrap();
    client.listen(1).await.unwrap();

    client.ping("127.0.0.1:19132").await.unwrap();

    let timeout_duration = tokio::time::Duration::from_secs(5);

    let result = tokio::time::timeout(timeout_duration, async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;

            if let Some(msg) = client.recv() {
                if let RukMessageContext::UnconnectedPong { ping_res, .. } = msg.context {
                    assert_eq!(ping_res, ping_response);
                    break;
                }
            }
        }
    });

    result.await.unwrap();
}
