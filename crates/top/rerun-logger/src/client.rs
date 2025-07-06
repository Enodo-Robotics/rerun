//! gRPC client for connecting to external Rerun proxy servers

use anyhow::{Context, Result};
use crossbeam::channel::Sender;
use re_log_types::LogMsg;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

/// Run gRPC client to connect to external proxy server
pub async fn run_grpc_client(
    url: String,
    message_sender: Sender<LogMsg>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<()> {
    re_log::info!("Starting gRPC client to connect to: {}", url);
    
    // Parse the URL to validate it's a proper proxy URL
    let parsed_uri: re_uri::RedapUri = url.parse()
        .with_context(|| format!("Failed to parse URL: {}", url))?;
    
    let proxy_uri = match parsed_uri {
        re_uri::RedapUri::Proxy(proxy_uri) => proxy_uri,
        _ => {
            return Err(anyhow::anyhow!("URL must be a proxy endpoint (ending with /proxy): {}", url));
        }
    };
    
    re_log::info!("Validated proxy URI: {}", proxy_uri);
    
    // Connect using the existing gRPC client
    let on_msg = Some(Box::new(|| {
        // Callback called when messages are received (optional)
    }) as Box<dyn Fn() + Send + Sync>);
    
    let rx = re_grpc_client::message_proxy::stream(proxy_uri, on_msg);
    
    re_log::info!("Successfully connected to proxy server");
    
    // Forward messages from the proxy to our internal message channel
    // We need to use spawn_blocking because the smart channel recv() is synchronous
    let (tx, mut rx_async) = tokio::sync::mpsc::unbounded_channel();
    
    // Spawn a blocking task to handle the synchronous receiver
    let rx_clone = rx;
    let tx_clone = tx;
    tokio::task::spawn_blocking(move || {
        while let Ok(smart_msg) = rx_clone.recv() {
            if tx_clone.send(smart_msg).is_err() {
                break; // Channel closed
            }
        }
    });
    
    // Now use the async receiver in the main loop
    while !shutdown_signal.load(Ordering::Relaxed) {
        tokio::select! {
            msg_opt = rx_async.recv() => {
                match msg_opt {
                    Some(smart_msg) => {
                        match smart_msg.payload {
                            re_smart_channel::SmartMessagePayload::Msg(log_msg) => {
                                if let Err(err) = message_sender.send(log_msg) {
                                    re_log::error!("Failed to forward message from proxy: {}", err);
                                    break;
                                }
                            }
                            re_smart_channel::SmartMessagePayload::Flush { on_flush_done } => {
                                // Handle flush completion
                                on_flush_done();
                            }
                            re_smart_channel::SmartMessagePayload::Quit(err) => {
                                if let Some(err) = err {
                                    re_log::error!("Proxy connection closed with error: {}", err);
                                } else {
                                    re_log::info!("Proxy connection closed normally");
                                }
                                break;
                            }
                        }
                    }
                    None => {
                        re_log::info!("Proxy message channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                // Regular check for shutdown signal
                continue;
            }
        }
    }
    
    re_log::info!("gRPC client shutting down");
    Ok(())
}

/// Wait for a shutdown signal (Ctrl+C, SIGTERM, etc.)
pub async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to setup SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to setup SIGINT handler");
        
        tokio::select! {
            _ = sigterm.recv() => {
                re_log::info!("Received SIGTERM, shutting down gracefully");
            }
            _ = sigint.recv() => {
                re_log::info!("Received SIGINT (Ctrl+C), shutting down gracefully");
            }
        }
    }
    
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("Failed to setup Ctrl+C handler");
        re_log::info!("Received Ctrl+C, shutting down gracefully");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_uri_parsing() {
        // Test valid proxy URIs
        let valid_uris = [
            "rerun+http://localhost:9876/proxy",
            "rerun+https://example.com:9876/proxy",
            "rerun://secure.example.com:9876/proxy",
        ];
        
        for uri in &valid_uris {
            let parsed: Result<re_uri::RedapUri, _> = uri.parse();
            assert!(parsed.is_ok(), "Failed to parse valid URI: {}", uri);
            
            if let Ok(re_uri::RedapUri::Proxy(_)) = parsed {
                // Expected proxy URI
            } else {
                panic!("URI should parse as proxy: {}", uri);
            }
        }
        
        // Test invalid URIs (non-proxy endpoints)
        let invalid_uris = [
            "rerun+http://localhost:9876/catalog",
            "rerun+http://localhost:9876/",
            "http://localhost:9876/proxy", // Wrong scheme
        ];
        
        for uri in &invalid_uris {
            let parsed: Result<re_uri::RedapUri, _> = uri.parse();
            if let Ok(re_uri::RedapUri::Proxy(_)) = parsed {
                panic!("URI should not parse as valid proxy: {}", uri);
            }
        }
    }
}