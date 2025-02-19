use ruknet::{app::RukMessageContext, Peer};

#[tokio::test]
async fn ping_res() {
    let timeout_duration = tokio::time::Duration::from_secs(5);

    let ping_response_1 = "ping response 1";
    let ping_response_2 = "ping response 2";

    let mut server = Peer::new("127.0.0.1:19132", ping_response_1).await.unwrap();
    let mut client = Peer::new("127.0.0.1:19134", "").await.unwrap();

    server.listen(10).await.unwrap();
    client.listen(1).await.unwrap();

    client.ping("127.0.0.1:19132").await.unwrap();

    let result = tokio::time::timeout(timeout_duration, async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;

            if let Some(msg) = client.recv() {
                if let RukMessageContext::UnconnectedPong { ping_res, .. } = msg.context {
                    assert_eq!(ping_res, ping_response_1);
                    break;
                }
            }
        }
    });

    result.await.unwrap();

    server.set_ping_res(ping_response_2).await;

    client.ping("127.0.0.1:19132").await.unwrap();

    let result = tokio::time::timeout(timeout_duration, async {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;

            if let Some(msg) = client.recv() {
                if let RukMessageContext::UnconnectedPong { ping_res, .. } = msg.context {
                    assert_eq!(ping_res, ping_response_2);
                    break;
                }
            }
        }
    });

    result.await.unwrap();
}
