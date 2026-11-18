//! Enodo fork: end-to-end test of `--reconnect-interval` against a proxy server that dies
//! and comes back on the same port.
#![cfg(not(target_arch = "wasm32"))]
#![expect(clippy::unwrap_used)] // Fine for tests

use std::net::SocketAddr;
use std::time::Duration;

use re_log_encoding::ToTransport as _;
use re_log_encoding::rrd::Compression;
use re_log_types::{
    BlueprintActivationCommand, LogMsg, SetStoreInfo, StoreId, StoreInfo, StoreKind, StoreSource,
};
use re_protos::sdk_comms::v1alpha1::WriteMessagesRequest;
use re_protos::sdk_comms::v1alpha1::message_proxy_service_client::MessageProxyServiceClient;

/// A proxy server in its own runtime, so dropping it kills it the way a process exit would.
struct Server(Option<tokio::runtime::Runtime>);

impl Server {
    fn start(addr: SocketAddr) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.spawn(async move {
            re_grpc_server::serve(
                addr,
                re_grpc_server::ServerOptions::default(),
                re_grpc_server::shutdown::never(),
            )
            .await
            .unwrap();
        });
        Self(Some(runtime))
    }

    fn kill(mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

async fn write(addr: SocketAddr, messages: Vec<LogMsg>) {
    let mut client = loop {
        // The server may still be binding.
        let endpoint = tonic::transport::Endpoint::new(format!("http://{addr}")).unwrap();
        if let Ok(channel) = endpoint.connect().await {
            break MessageProxyServiceClient::new(channel);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    client
        .write_messages(tokio_stream::iter(messages.into_iter().map(|msg| {
            WriteMessagesRequest {
                log_msg: Some(msg.to_transport(Compression::Off).unwrap().into()),
            }
        })))
        .await
        .unwrap();
}

fn store_info(store_id: &StoreId) -> LogMsg {
    LogMsg::SetStoreInfo(SetStoreInfo {
        row_id: re_tuid::Tuid::new(),
        info: StoreInfo::new(store_id.clone(), StoreSource::Unknown),
    })
}

fn chunk(store_id: &StoreId, entity: &str) -> LogMsg {
    LogMsg::ArrowMsg(
        store_id.clone(),
        re_chunk::Chunk::builder(entity)
            .build()
            .unwrap()
            .to_arrow_msg()
            .unwrap(),
    )
}

/// Receives until a chunk for `last_entity` arrives; returns `(store, what)` for each message.
fn receive_until(rx: &re_log_channel::LogReceiver, last_entity: &str) -> Vec<(StoreId, String)> {
    let mut received = Vec::new();
    loop {
        let msg = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("the stream should come back");
        let Some(re_log_channel::DataSourceMessage::LogMsg(msg)) = msg.into_data() else {
            continue;
        };
        let what = match &msg {
            LogMsg::SetStoreInfo(_) => "info".to_owned(),
            LogMsg::BlueprintActivationCommand(_) => "activate".to_owned(),
            LogMsg::ArrowMsg(_, arrow) => re_chunk::Chunk::from_arrow_msg(arrow)
                .unwrap()
                .entity_path()
                .to_string(),
        };
        received.push((msg.store_id().clone(), what.clone()));
        if what == format!("/{last_entity}") {
            return received;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnects_and_keeps_the_layout() {
    re_grpc_client::set_reconnect_interval(Some(Duration::from_millis(100)));

    let addr = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let uri: re_uri::ProxyUri = format!("rerun+http://{addr}/proxy").parse().unwrap();

    // The viewer starts first, before anything is listening.
    let rx = re_grpc_client::stream(uri);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let recording_1 = StoreId::new(StoreKind::Recording, "app", "r1");
    let blueprint_1 = StoreId::new(StoreKind::Blueprint, "app", "b1");
    let replayed = chunk(&recording_1, "first");

    let server = Server::start(addr);
    write(
        addr,
        vec![
            store_info(&recording_1),
            replayed.clone(),
            store_info(&blueprint_1),
            LogMsg::BlueprintActivationCommand(BlueprintActivationCommand::make_active(
                blueprint_1.clone(),
            )),
            chunk(&recording_1, "end_1"),
        ],
    )
    .await;
    let first = tokio::task::block_in_place(|| receive_until(&rx, "end_1"));
    assert_eq!(first.len(), 5, "{first:?}");

    // The process dies and a new one comes up on the same port, with a new recording and
    // its default blueprint, and (as a proxy that stayed up would) the old data again.
    server.kill();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let server = Server::start(addr);

    let recording_2 = StoreId::new(StoreKind::Recording, "app", "r2");
    let blueprint_2 = StoreId::new(StoreKind::Blueprint, "app", "b2");
    write(
        addr,
        vec![
            replayed,
            store_info(&blueprint_2),
            LogMsg::BlueprintActivationCommand(BlueprintActivationCommand::make_active(
                blueprint_2.clone(),
            )),
            store_info(&recording_2),
            chunk(&recording_2, "end_2"),
        ],
    )
    .await;
    let second = tokio::task::block_in_place(|| receive_until(&rx, "end_2"));
    assert_eq!(
        second,
        vec![
            (recording_2.clone(), "info".to_owned()),
            (recording_2, "/end_2".to_owned()),
        ]
    );

    server.kill();
}
