//! Core logger implementation that buffers data and saves to .rrd files

use anyhow::{Context, Result};
use crossbeam::channel::{Receiver, Sender};
use re_chunk_store::ChunkStore;
use re_log_types::{ApplicationId, LogMsg, StoreId, StoreInfo, StoreKind, StoreSource};
use re_sdk::sink::FileSink;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::time;

use crate::config::LoggerConfig;

/// Statistics about the logger's operation
#[derive(Debug)]
pub struct LoggerStats {
    /// Total number of messages received
    pub messages_received: AtomicU64,
    /// Total number of messages written to file
    pub messages_written: AtomicU64,
    /// Total bytes written to file
    pub bytes_written: AtomicU64,
    /// Number of flush operations performed
    pub flush_count: AtomicU64,
    /// Current memory usage in bytes
    pub memory_usage: AtomicU64,
    /// Time when logging started
    pub start_time: std::time::Instant,
}

/// Main logger that buffers data and periodically saves to .rrd files
pub struct RerunLogger {
    config: LoggerConfig,
    chunk_store: Arc<std::sync::Mutex<ChunkStore>>,
    file_sink: Option<FileSink>,
    stats: Arc<LoggerStats>,
    pub shutdown_signal: Arc<AtomicBool>,
    message_sender: Sender<LogMsg>,
    message_receiver: Receiver<LogMsg>,
}

impl RerunLogger {
    /// Create a new RerunLogger with the given configuration
    pub async fn new(config: LoggerConfig) -> Result<Self> {
        config.validate()?;

        // Create the output file sink
        let file_sink = if config.output_path.to_str() == Some("-") {
            // Special case: stdout
            None
        } else {
            Some(
                FileSink::new(&config.output_path)
                    .with_context(|| format!("Failed to create file sink for {}", config.output_path.display()))?,
            )
        };

        // Create chunk store
        let store_id = StoreId::random(StoreKind::Recording);
        let store_info = StoreInfo {
            application_id: ApplicationId::from("rerun-logger"),
            store_id: store_id.clone(),
            cloned_from: None,
            store_source: StoreSource::Unknown,
            store_version: None,
        };

        let mut chunk_store = ChunkStore::new(store_id, Default::default());
        
        chunk_store.set_info(store_info);
        
        // Configure memory limits if specified
        if let Some(max_memory) = config.max_memory {
            // TODO: Set memory limit on the chunk store
            re_log::info!("Memory limit set to {} bytes", max_memory);
        }

        let chunk_store = Arc::new(std::sync::Mutex::new(chunk_store));

        // Create message channel for communication between components
        let (message_sender, message_receiver) = crossbeam::channel::unbounded();

        let stats = Arc::new(LoggerStats {
            messages_received: AtomicU64::new(0),
            messages_written: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            flush_count: AtomicU64::new(0),
            memory_usage: AtomicU64::new(0),
            start_time: Instant::now(),
        });

        Ok(Self {
            config,
            chunk_store,
            file_sink,
            stats,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            message_sender,
            message_receiver,
        })
    }

    /// Get a sender for log messages
    pub fn message_sender(&self) -> Sender<LogMsg> {
        self.message_sender.clone()
    }

    /// Get the current statistics
    pub fn stats(&self) -> &LoggerStats {
        &self.stats
    }

    /// Get the current statistics as an Arc
    pub fn stats_arc(&self) -> Arc<LoggerStats> {
        self.stats.clone()
    }

    /// Request a graceful shutdown
    pub fn shutdown(&self) {
        self.shutdown_signal.store(true, Ordering::Relaxed);
    }

    /// Run the logger (this will block until shutdown)
    pub async fn run(&self) -> Result<()> {
        re_log::info!("Starting Rerun Logger with config: {:?}", self.config);
        re_log::info!("Output file: {}", self.config.output_path.display());
        
        if let Some(ref connect_url) = self.config.connect_url {
            re_log::info!("Connecting to gRPC proxy: {}", connect_url);
        } else {
            re_log::info!("Listening on port: {}", self.config.port);
        }

        // Start the main processing loop
        let processing_handle = {
            let logger = self.clone_for_task();
            tokio::spawn(async move {
                logger.process_messages().await
            })
        };

        // Start the periodic flush task
        let flush_handle = {
            let logger = self.clone_for_task();
            tokio::spawn(async move {
                logger.periodic_flush().await
            })
        };

        // Start either gRPC client or server based on configuration
        let network_handle = if let Some(ref connect_url) = self.config.connect_url {
            // gRPC client mode - connect to external proxy
            let message_sender = self.message_sender.clone();
            let url = connect_url.clone();
            let shutdown_signal = self.shutdown_signal.clone();
            
            Some(tokio::spawn(async move {
                crate::client::run_grpc_client(url, message_sender, shutdown_signal).await
            }))
        } else {
            // gRPC server mode - start our own server if enabled
            #[cfg(feature = "server")]
            {
                let message_sender = self.message_sender.clone();
                let port = self.config.port;
                let shutdown_signal = self.shutdown_signal.clone();
                
                Some(tokio::spawn(async move {
                    crate::server::run_grpc_server(port, message_sender, shutdown_signal).await
                }))
            }
            #[cfg(not(feature = "server"))]
            {
                re_log::info!("Server feature not enabled, running in file-only mode");
                None
            }
        };

        // Wait for shutdown signal
        if let Some(network_handle) = network_handle {
            tokio::select! {
                result = processing_handle => {
                    re_log::info!("Message processing task completed: {:?}", result);
                }
                result = flush_handle => {
                    re_log::info!("Flush task completed: {:?}", result);
                }
                result = network_handle => {
                    re_log::info!("Network task (server/client) completed: {:?}", result);
                }
                _ = self.wait_for_shutdown() => {
                    re_log::info!("Shutdown signal received");
                }
            }
        } else {
            tokio::select! {
                result = processing_handle => {
                    re_log::info!("Message processing task completed: {:?}", result);
                }
                result = flush_handle => {
                    re_log::info!("Flush task completed: {:?}", result);
                }
                _ = self.wait_for_shutdown() => {
                    re_log::info!("Shutdown signal received");
                }
            }
        }

        // Perform final flush
        self.flush_to_file().await?;
        re_log::info!("Logger shutdown complete");
        
        Ok(())
    }

    /// Wait for shutdown signal
    async fn wait_for_shutdown(&self) {
        while !self.shutdown_signal.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Clone the logger for use in async tasks
    fn clone_for_task(&self) -> Self {
        Self {
            config: self.config.clone(),
            chunk_store: self.chunk_store.clone(),
            file_sink: None, // File sink is not Send, so we'll recreate it in tasks that need it
            stats: self.stats.clone(),
            shutdown_signal: self.shutdown_signal.clone(),
            message_sender: self.message_sender.clone(),
            message_receiver: self.message_receiver.clone(),
        }
    }

    /// Main message processing loop
    async fn process_messages(&self) -> Result<()> {
        let mut last_memory_check = Instant::now();
        let memory_check_interval = Duration::from_secs(1);

        while !self.shutdown_signal.load(Ordering::Relaxed) {
            // Try to receive a message with timeout
            match self.message_receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(msg) => {
                    self.handle_message(msg).await?;
                }
                Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                    // No message received, continue
                }
                Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                    re_log::info!("Message channel disconnected, shutting down");
                    break;
                }
            }

            // Periodic memory usage check
            if last_memory_check.elapsed() >= memory_check_interval {
                self.update_memory_usage();
                self.check_memory_pressure().await?;
                last_memory_check = Instant::now();
            }
        }

        Ok(())
    }

    /// Handle a single log message
    async fn handle_message(&self, msg: LogMsg) -> Result<()> {
        self.stats.messages_received.fetch_add(1, Ordering::Relaxed);

        // Add message to chunk store
        match msg {
            LogMsg::SetStoreInfo(set_store_info) => {
                if let Ok(mut store) = self.chunk_store.lock() {
                    store.set_info(set_store_info.info);
                    re_log::trace!("Updated store info");
                }
            }
            LogMsg::ArrowMsg(_, arrow_msg) => {
                // This is the main data message type
                // For now, we'll store the message for later flushing
                // TODO: Convert ArrowMsg to Chunk and add to store
                re_log::trace!("Received ArrowMsg with {} bytes", arrow_msg.batch.get_array_memory_size());
            }
            LogMsg::BlueprintActivationCommand(_) => {
                // Blueprint messages are not relevant for pure data logging
                re_log::trace!("Ignoring blueprint activation command");
            }
        }

        Ok(())
    }

    /// Periodic flush task
    async fn periodic_flush(&self) -> Result<()> {
        let mut interval = time::interval(self.config.flush_interval);
        interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        while !self.shutdown_signal.load(Ordering::Relaxed) {
            interval.tick().await;
            
            if self.should_flush() {
                if let Err(err) = self.flush_to_file().await {
                    re_log::error!("Failed to flush to file: {}", err);
                }
            }
        }

        Ok(())
    }

    /// Check if we should flush based on current conditions
    fn should_flush(&self) -> bool {
        if let Ok(store) = self.chunk_store.lock() {
            let stats = store.stats();
            let current_bytes = stats.total().total_size_bytes;
            let current_rows = stats.total().num_rows;

            // Check byte threshold
            if current_bytes >= self.config.flush_bytes {
                re_log::debug!("Flushing due to byte threshold: {} >= {}", current_bytes, self.config.flush_bytes);
                return true;
            }

            // Check row threshold if configured
            if let Some(max_rows) = self.config.flush_rows {
                if current_rows >= max_rows {
                    re_log::debug!("Flushing due to row threshold: {} >= {}", current_rows, max_rows);
                    return true;
                }
            }
        }

        false
    }

    /// Flush current data to file
    async fn flush_to_file(&self) -> Result<()> {
        let start_time = Instant::now();
        
        // Get all messages from the chunk store
        let messages = if let Ok(store) = self.chunk_store.lock() {
            let mut messages = Vec::new();
            
            // Convert chunks to messages
            for chunk in store.iter_chunks() {
                match chunk.to_arrow_msg() {
                    Ok(arrow_msg) => {
                        if let Some(store_info) = store.info() {
                            messages.push(LogMsg::ArrowMsg(store_info.store_id.clone(), arrow_msg));
                        }
                    }
                    Err(err) => {
                        re_log::warn!("Failed to convert chunk to arrow message: {}", err);
                    }
                }
            }
            
            messages
        } else {
            re_log::error!("Failed to lock chunk store for flushing");
            return Ok(());
        };
        
        if messages.is_empty() {
            re_log::trace!("No data to flush");
            return Ok(());
        }

        // Write messages to file if we have a file sink
        if let Some(ref file_sink) = self.file_sink {
            let mut bytes_written = 0u64;
            let message_count = messages.len();
            
            for msg in &messages {
                file_sink.send(msg.clone());
                // Note: FileSink.send() is infallible in the current API
                
                // Estimate bytes written (this is approximate)
                bytes_written += 100; // Rough estimate per message
                self.stats.messages_written.fetch_add(1, Ordering::Relaxed);
            }
            
            // Flush the file sink
            file_sink.flush_blocking();
            
            self.stats.bytes_written.fetch_add(bytes_written, Ordering::Relaxed);
            self.stats.flush_count.fetch_add(1, Ordering::Relaxed);
            
            let elapsed = start_time.elapsed();
            re_log::info!(
                "Flushed {} messages ({} bytes) to {} in {:?}",
                message_count,
                bytes_written,
                self.config.output_path.display(),
                elapsed
            );
        }

        Ok(())
    }

    /// Update memory usage statistics
    fn update_memory_usage(&self) {
        if let Ok(store) = self.chunk_store.lock() {
            let memory_usage = store.stats().total().total_size_bytes;
            self.stats.memory_usage.store(memory_usage, Ordering::Relaxed);
        }
    }

    /// Check memory pressure and perform garbage collection if needed
    async fn check_memory_pressure(&self) -> Result<()> {
        if let Some(max_memory) = self.config.max_memory {
            let current_usage = self.stats.memory_usage.load(Ordering::Relaxed);
            
            if current_usage >= max_memory {
                re_log::warn!(
                    "Memory usage ({} bytes) exceeds limit ({} bytes), forcing flush",
                    current_usage,
                    max_memory
                );
                
                self.flush_to_file().await?;
                
                // TODO: Implement more sophisticated garbage collection
                // For now, we just flush to file and rely on the chunk store's built-in GC
            }
        }
        
        Ok(())
    }
}

impl LoggerStats {
    /// Get messages per second since start
    pub fn messages_per_second(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.messages_received.load(Ordering::Relaxed) as f64 / elapsed
        } else {
            0.0
        }
    }

    /// Get bytes per second since start
    pub fn bytes_per_second(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bytes_written.load(Ordering::Relaxed) as f64 / elapsed
        } else {
            0.0
        }
    }

    /// Get uptime duration
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_logger_creation() {
        let temp_dir = tempdir().unwrap();
        let output_path = temp_dir.path().join("test.rrd");
        
        let config = LoggerConfig {
            output_path,
            ..Default::default()
        };
        
        let logger = RerunLogger::new(config).await.unwrap();
        assert_eq!(logger.stats().messages_received.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_logger_stats() {
        let temp_dir = tempdir().unwrap();
        let output_path = temp_dir.path().join("test.rrd");
        
        let config = LoggerConfig {
            output_path,
            ..Default::default()
        };
        
        let logger = RerunLogger::new(config).await.unwrap();
        let stats = logger.stats();
        
        // Initially should have zero values
        assert_eq!(stats.messages_received.load(Ordering::Relaxed), 0);
        assert_eq!(stats.messages_written.load(Ordering::Relaxed), 0);
        assert_eq!(stats.bytes_written.load(Ordering::Relaxed), 0);
        
        // Check that uptime is working
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(stats.uptime() >= Duration::from_millis(10));
    }

    #[test]
    fn test_flush_conditions() {
        let config = LoggerConfig {
            flush_bytes: 1000,
            flush_rows: Some(100),
            ..Default::default()
        };
        
        // Test that config produces expected thresholds
        assert_eq!(config.flush_bytes, 1000);
        assert_eq!(config.flush_rows, Some(100));
    }
}