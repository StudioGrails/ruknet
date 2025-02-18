use ruknet::{app::RukMessageContext, Peer, Priority, Reliability};

#[tokio::test]
async fn hello_world() {
    let mut server = Peer::new("127.0.0.1:19132", "ping response").await.unwrap();
    let mut client = Peer::new("127.0.0.1:19134", "ping response").await.unwrap();

    server.listen(10).await.unwrap();
    client.listen(1).await.unwrap();

    client.connect("127.0.0.1:19132", 3, 2000, None).unwrap();

    let server_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;

            if let Some(msg) = server.recv() {
                if let RukMessageContext::App { data } = msg.context {
                    server
                        .send(
                            msg.addr,
                            Priority::Immediate,
                            Reliability::Reliable,
                            0,
                            data,
                        )
                        .await;
                    // wait for the tick to send the message.
                    tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;
                    break;
                }
            }
        }
    });
    let client_task = tokio::spawn(async move {
        // first byte is the message id. Some of them are reserved for internal use.
        let mut send_data = vec![0xfe];
        send_data.extend_from_slice(b"Hello, world!");

        let timeout_duration = tokio::time::Duration::from_secs(5);

        let result = tokio::time::timeout(timeout_duration, async {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;

                if let Some(msg) = client.recv() {
                    match msg.context {
                        RukMessageContext::ConnectionRequestAccepted => {
                            client
                                .send(
                                    msg.addr,
                                    Priority::Immediate,
                                    Reliability::Reliable,
                                    0,
                                    send_data.clone(),
                                )
                                .await;
                        }
                        RukMessageContext::App { data } => {
                            if data == send_data {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        })
        .await;

        assert!(result.is_ok());
    });

    server_task.await.unwrap();
    client_task.await.unwrap();
}
