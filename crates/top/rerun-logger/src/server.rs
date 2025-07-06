//! gRPC server implementation for receiving Rerun log messages

use anyhow::{Context, Result};
use crossbeam::channel::Sender;
use re_log_types::LogMsg;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::signal;

#[cfg(feature = "server")]
use re_protos::sdk_comms::v1alpha1::{
    message_proxy_service_server::{MessageProxyService, MessageProxyServiceServer},
    WriteMessagesRequest, WriteMessagesResponse, ReadMessagesRequest, ReadMessagesResponse,
};
#[cfg(feature = "server")]
use tokio_stream::wrappers::ReceiverStream;

/// Run a gRPC server that accepts Rerun log messages
#[cfg(feature = "server")]
pub async fn run_grpc_server(
    port: u16,
    message_sender: Sender<LogMsg>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<()> {
    use std::net::SocketAddr;

    re_log::info!("Starting gRPC server on port {}", port);

    // Create the gRPC service
    let service = RerunLoggerService::new(message_sender);
    
    // Create the server
    let addr: SocketAddr = format!("0.0.0.0:{}", port)
        .parse()
        .with_context(|| format!("Failed to parse address for port {}", port))?;

    let server = tonic::transport::Server::builder()
        .add_service(MessageProxyServiceServer::new(service))
        .serve_with_shutdown(addr, async move {
            // Wait for shutdown signal
            while !shutdown_signal.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            re_log::info!("gRPC server shutdown signal received");
        });

    re_log::info!("gRPC server listening on {}", addr);

    if let Err(err) = server.await {
        re_log::error!("gRPC server error: {}", err);
        return Err(err.into());
    }

    re_log::info!("gRPC server stopped");
    Ok(())
}

/// gRPC service implementation for the Rerun Logger
#[cfg(feature = "server")]
struct RerunLoggerService {
    message_sender: Sender<LogMsg>,
    stats: ServiceStats,
}

#[cfg(feature = "server")]
#[derive(Default)]
struct ServiceStats {
    connections: std::sync::atomic::AtomicU64,
    messages_received: std::sync::atomic::AtomicU64,
    bytes_received: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "server")]
impl RerunLoggerService {
    fn new(message_sender: Sender<LogMsg>) -> Self {
        Self {
            message_sender,
            stats: ServiceStats::default(),
        }
    }
}

#[cfg(feature = "server")]
#[tonic::async_trait]
impl MessageProxyService for RerunLoggerService {
    async fn write_messages(
        &self,
        request: tonic::Request<tonic::Streaming<WriteMessagesRequest>>,
    ) -> Result<tonic::Response<WriteMessagesResponse>, tonic::Status> {
        let peer_addr = request
            .remote_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        re_log::info!("New gRPC connection from: {}", peer_addr);
        self.stats.connections.fetch_add(1, Ordering::Relaxed);

        let mut stream = request.into_inner();
        let message_sender = self.message_sender.clone();
        let stats = &self.stats;

        // Process incoming messages
        while let Some(request) = stream.message().await.transpose() {
            match request {
                Ok(write_request) => {
                    if let Some(log_msg) = write_request.log_msg {
                        match Self::process_log_msg(log_msg, &message_sender, stats).await {
                            Ok(_) => {
                                // Message processed successfully
                            }
                            Err(err) => {
                                re_log::warn!("Failed to process log message: {}", err);
                                return Err(tonic::Status::internal(format!("Failed to process message: {}", err)));
                            }
                        }
                    }
                }
                Err(err) => {
                    re_log::warn!("gRPC stream error: {}", err);
                    return Err(tonic::Status::internal(format!("Stream error: {}", err)));
                }
            }
        }
        
        re_log::info!("gRPC connection from {} closed", peer_addr);
        Ok(tonic::Response::new(WriteMessagesResponse {}))
    }

    type ReadMessagesStream = ReceiverStream<Result<ReadMessagesResponse, tonic::Status>>;

    async fn read_messages(
        &self,
        _request: tonic::Request<ReadMessagesRequest>,
    ) -> Result<tonic::Response<Self::ReadMessagesStream>, tonic::Status> {
        // For now, just return an empty stream since we're primarily focused on writing
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(tonic::Response::new(ReceiverStream::new(rx)))
    }

    type ReadTablesStream = ReceiverStream<Result<re_protos::sdk_comms::v1alpha1::ReadTablesResponse, tonic::Status>>;

    async fn write_table(
        &self,
        _request: tonic::Request<re_protos::sdk_comms::v1alpha1::WriteTableRequest>,
    ) -> Result<tonic::Response<re_protos::sdk_comms::v1alpha1::WriteTableResponse>, tonic::Status> {
        // Not implemented for this logger
        Err(tonic::Status::unimplemented("write_table not implemented"))
    }

    async fn read_tables(
        &self,
        _request: tonic::Request<re_protos::sdk_comms::v1alpha1::ReadTablesRequest>,
    ) -> Result<tonic::Response<Self::ReadTablesStream>, tonic::Status> {
        // Not implemented for this logger
        Err(tonic::Status::unimplemented("read_tables not implemented"))
    }
}

#[cfg(feature = "server")]
impl RerunLoggerService {
    async fn process_log_msg(
        log_msg_proto: re_protos::log_msg::v1alpha1::LogMsg,
        message_sender: &Sender<LogMsg>,
        stats: &ServiceStats,
    ) -> Result<()> {
        // Convert protobuf LogMsg to internal LogMsg
        let log_msg = re_log_encoding::protobuf_conversions::log_msg_from_proto(log_msg_proto)
            .context("Failed to convert protobuf message")?;

        // Send the message to the logger
        message_sender
            .send(log_msg)
            .context("Failed to send message to logger")?;

        stats.messages_received.fetch_add(1, Ordering::Relaxed);
        
        Ok(())
    }
}

/// Fallback implementation when server feature is not enabled
#[cfg(not(feature = "server"))]
pub async fn run_grpc_server(
    _port: u16,
    _message_sender: Sender<LogMsg>,
    _shutdown_signal: Arc<AtomicBool>,
) -> Result<()> {
    anyhow::bail!("gRPC server support not compiled in. Enable the 'server' feature to use this functionality.");
}

/// Wait for SIGINT or SIGTERM signals
pub async fn wait_for_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            re_log::info!("Received Ctrl+C signal");
        },
        _ = terminate => {
            re_log::info!("Received terminate signal");
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_signal_handling() {
        // This test just ensures the signal handling functions compile
        // In a real test environment, actually testing signals is complex
        // and usually done through integration tests
    }

    #[cfg(feature = "server")]
    #[test]
    fn test_service_creation() {
        let (sender, _receiver) = crossbeam::channel::unbounded();
        let service = RerunLoggerService::new(sender);
        
        // Check initial stats
        assert_eq!(service.stats.connections.load(Ordering::Relaxed), 0);
        assert_eq!(service.stats.messages_received.load(Ordering::Relaxed), 0);
    }
}