use std::{
    fmt,
    path::PathBuf,
    sync::mpsc::{Receiver, RecvTimeoutError, SendError, Sender, SyncSender},
};

use parking_lot::Mutex;

use re_log_types::LogMsg;

/// An error that can occur when flushing.
#[derive(Debug, thiserror::Error)]
pub enum FileFlushError {
    #[error("Failed to flush file: {message}")]
    Failed { message: String },

    #[error("File flush timed out - not all messages were written.")]
    Timeout,
}

impl FileFlushError {
    fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
        }
    }
}

/// Errors that can occur when creating a [`FileSink`].
#[derive(thiserror::Error, Debug)]
pub enum FileSinkError {
    /// Error creating the file.
    #[error("Failed to create file {0}: {1}")]
    CreateFile(PathBuf, std::io::Error),

    /// Error spawning the file writer thread.
    #[error("Failed to spawn thread: {0}")]
    SpawnThread(std::io::Error),

    /// Error encoding a log message.
    #[error("Failed to encode LogMsg: {0}")]
    LogMsgEncode(#[from] crate::encoder::EncodeError),
}

enum Command {
    Send(LogMsg),
    Flush {
        on_done: SyncSender<Result<(), String>>,
    },
    #[allow(dead_code)] // Reserved for future manual rotation API
    RotateFile,
}

impl Command {
    fn flush() -> (Self, Receiver<Result<(), String>>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(0); // oneshot
        (Self::Flush { on_done: tx }, rx)
    }
}

/// Stream log messages to an `.rrd` file.
pub struct FileSink {
    // None = quit
    tx: Mutex<Sender<Option<Command>>>,
    join_handle: Option<std::thread::JoinHandle<()>>,

    /// Only used for diagnostics, not for access after `new()`.
    ///
    /// `None` indicates stdout.
    path: Option<PathBuf>,

    /// Maximum file size in bytes before creating a new file.
    /// `None` means no rotation.
    #[allow(dead_code)] // Stored for diagnostics/debugging
    max_file_size: Option<u64>,
}

impl Drop for FileSink {
    fn drop(&mut self) {
        self.tx.lock().send(None).ok();
        if let Some(join_handle) = self.join_handle.take() {
            join_handle.join().ok();
        }
    }
}

impl FileSink {
    /// Start writing log messages to a file at the given path.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Result<Self, FileSinkError> {
        Self::new_with_max_size(path, None)
    }

    /// Start writing log messages to a file at the given path with optional file rotation.
    ///
    /// If `max_file_size` is `Some(size)`, the sink will create new files when the current
    /// file exceeds `size` bytes. Static messages (SetStoreInfo, BlueprintActivationCommand)
    /// will be written to every file, while temporal messages (ArrowMsg) will be split across files.
    pub fn new_with_max_size(
        path: impl Into<std::path::PathBuf>,
        max_file_size: Option<u64>,
    ) -> Result<Self, FileSinkError> {
        // We always compress on disk
        let encoding_options = crate::EncodingOptions::PROTOBUF_COMPRESSED;

        let (tx, rx) = std::sync::mpsc::channel();

        let path = path.into();

        re_log::debug!("Saving file to {path:?}…");

        // TODO(andreas): Can we ensure that a single process doesn't
        // have multiple file sinks for the same file live?
        // This likely caused an instability in the past, see https://github.com/rerun-io/rerun/issues/3306

        let file = std::fs::File::create(&path)
            .map_err(|err| FileSinkError::CreateFile(path.clone(), err))?;
        let encoder = crate::encoder::DroppableEncoder::new(
            re_build_info::CrateVersion::LOCAL,
            encoding_options,
            file,
        )?;
        let join_handle = spawn_and_stream(Some(&path), encoder, rx, max_file_size)?;

        Ok(Self {
            tx: tx.into(),
            join_handle: Some(join_handle),
            path: Some(path),
            max_file_size,
        })
    }

    /// Start writing log messages to standard output.
    pub fn stdout() -> Result<Self, FileSinkError> {
        let encoding_options = crate::EncodingOptions::PROTOBUF_COMPRESSED;

        let (tx, rx) = std::sync::mpsc::channel();

        re_log::debug!("Writing to stdout…");

        let encoder = crate::encoder::DroppableEncoder::new(
            re_build_info::CrateVersion::LOCAL,
            encoding_options,
            std::io::stdout(),
        )?;
        let join_handle = spawn_and_stream(None, encoder, rx, None)?;

        Ok(Self {
            tx: tx.into(),
            join_handle: Some(join_handle),
            path: None,
            max_file_size: None,
        })
    }

    #[inline]
    pub fn flush_blocking(&self, timeout: std::time::Duration) -> Result<(), FileFlushError> {
        let (cmd, oneshot) = Command::flush();
        self.tx.lock().send(Some(cmd)).map_err(|_ignored| {
            FileFlushError::failed("File-writer thread shut down prematurely")
        })?;

        oneshot
            .recv_timeout(timeout)
            .map_err(|err| match err {
                RecvTimeoutError::Timeout => FileFlushError::Timeout,
                RecvTimeoutError::Disconnected => {
                    FileFlushError::failed("File-writer thread shut down prematurely")
                }
            })?
            .map_err(FileFlushError::failed)
    }

    #[inline]
    pub fn send(&self, log_msg: LogMsg) {
        self.tx.lock().send(Some(Command::Send(log_msg))).ok();
    }
}

/// Set `filepath` to `None` to stream to standard output.
fn spawn_and_stream<W: std::io::Write + Send + 'static>(
    filepath: Option<&std::path::Path>,
    encoder: crate::encoder::DroppableEncoder<W>,
    rx: Receiver<Option<Command>>,
    max_file_size: Option<u64>,
) -> Result<std::thread::JoinHandle<()>, FileSinkError> {
    // If we have a filepath and max_file_size, use the rotating version
    if let (Some(filepath), Some(max_size)) = (filepath, max_file_size) {
        spawn_rotating_file_stream(filepath, rx, max_size)
    } else {
        spawn_simple_stream(filepath, encoder, rx)
    }
}

/// Simple non-rotating stream handler
fn spawn_simple_stream<W: std::io::Write + Send + 'static>(
    filepath: Option<&std::path::Path>,
    mut encoder: crate::encoder::DroppableEncoder<W>,
    rx: Receiver<Option<Command>>,
) -> Result<std::thread::JoinHandle<()>, FileSinkError> {
    let (name, target) = if let Some(filepath) = filepath {
        ("file_writer", filepath.display().to_string())
    } else {
        ("stdout_writer", "stdout".to_owned())
    };
    std::thread::Builder::new()
        .name(name.into())
        .spawn({
            move || {
                while let Ok(Some(cmd)) = rx.recv() {
                    match cmd {
                        Command::Send(log_msg) => {
                            if let Err(err) = encoder.append(&log_msg) {
                                re_log::error!("Failed to write log stream to {target}: {err}");
                                return;
                            }
                        }
                        Command::Flush { on_done } => {
                            re_log::trace!("Flushing…");

                            let result = encoder.flush_blocking().map_err(|err| {
                                format!("Failed to flush log stream to {target}: {err}")
                            });

                            // Send back the result:
                            if let Err(SendError(result)) = on_done.send(result)
                                && let Err(err) = result
                            {
                                // There was an error, and nobody received it:
                                re_log::error!("{err}");
                            }
                        }
                        Command::RotateFile => {
                            // This command is only used in the file rotation version
                            re_log::warn!("Received RotateFile command on non-rotating sink");
                        }
                    }
                }
                if let Err(err) = encoder.finish() {
                    re_log::error!("Failed to end log stream for {target}: {err}");
                    return;
                }
                re_log::debug!("Log stream written to {target}");
            }
        })
        .map_err(FileSinkError::SpawnThread)
}

/// Rotating file stream handler - creates new files when size limit is exceeded
fn spawn_rotating_file_stream(
    base_path: &std::path::Path,
    rx: Receiver<Option<Command>>,
    max_file_size: u64,
) -> Result<std::thread::JoinHandle<()>, FileSinkError> {
    let base_path = base_path.to_owned();
    let encoding_options = crate::EncodingOptions::PROTOBUF_COMPRESSED;

    std::thread::Builder::new()
        .name("rotating_file_writer".into())
        .spawn({
            move || {
                let mut file_index = 0u32;
                let mut current_size = 0u64;
                let mut static_messages: Vec<LogMsg> = Vec::new();

                // Helper to create new file path
                let make_file_path = |index: u32| -> PathBuf {
                    if index == 0 {
                        base_path.clone()
                    } else {
                        let parent = base_path.parent().unwrap_or_else(|| std::path::Path::new(""));
                        let stem = base_path.file_stem().and_then(|s| s.to_str()).unwrap_or("recording");
                        let extension = base_path.extension().and_then(|s| s.to_str()).unwrap_or("rrd");
                        parent.join(format!("{}_{:03}.{}", stem, index, extension))
                    }
                };

                // Helper to create new encoder
                let create_encoder = |index: u32, static_msgs: &[LogMsg]| -> Result<crate::encoder::DroppableEncoder<std::fs::File>, FileSinkError> {
                    let file_path = make_file_path(index);
                    re_log::debug!("Creating new file: {file_path:?}");

                    let file = std::fs::File::create(&file_path)
                        .map_err(|err| FileSinkError::CreateFile(file_path.clone(), err))?;
                    let mut encoder = crate::encoder::DroppableEncoder::new(
                        re_build_info::CrateVersion::LOCAL,
                        encoding_options,
                        file,
                    )?;

                    // Write static messages to new file
                    for msg in static_msgs {
                        if let Err(err) = encoder.append(msg) {
                            re_log::error!("Failed to write static message to {file_path:?}: {err}");
                            return Err(FileSinkError::LogMsgEncode(err));
                        }
                    }

                    Ok(encoder)
                };

                let mut encoder = match create_encoder(file_index, &static_messages) {
                    Ok(e) => e,
                    Err(err) => {
                        re_log::error!("Failed to create initial encoder: {err}");
                        return;
                    }
                };
                file_index += 1;

                while let Ok(Some(cmd)) = rx.recv() {
                    match cmd {
                        Command::Send(log_msg) => {
                            // Track static messages
                            let is_static = matches!(
                                log_msg,
                                LogMsg::SetStoreInfo(_) | LogMsg::BlueprintActivationCommand(_)
                            );

                            if is_static {
                                static_messages.push(log_msg.clone());
                            }

                            // Check if we need to rotate before writing
                            if current_size > 0 && current_size >= max_file_size && !is_static {
                                // Finish current file
                                if let Err(err) = encoder.finish() {
                                    re_log::error!("Failed to finish file before rotation: {err}");
                                    return;
                                }

                                // Create new encoder
                                encoder = match create_encoder(file_index, &static_messages) {
                                    Ok(e) => e,
                                    Err(err) => {
                                        re_log::error!("Failed to create new encoder during rotation: {err}");
                                        return;
                                    }
                                };
                                file_index += 1;
                                current_size = 0;
                            }

                            // Write the message
                            match encoder.append(&log_msg) {
                                Ok(size) => {
                                    current_size += size;
                                }
                                Err(err) => {
                                    re_log::error!("Failed to write log stream: {err}");
                                    return;
                                }
                            }
                        }
                        Command::Flush { on_done } => {
                            re_log::trace!("Flushing…");

                            let result = encoder.flush_blocking().map_err(|err| {
                                format!("Failed to flush log stream: {err}")
                            });

                            // Send back the result:
                            if let Err(SendError(result)) = on_done.send(result)
                                && let Err(err) = result
                            {
                                // There was an error, and nobody received it:
                                re_log::error!("{err}");
                            }
                        }
                        Command::RotateFile => {
                            // Manual rotation request
                            if let Err(err) = encoder.finish() {
                                re_log::error!("Failed to finish file before manual rotation: {err}");
                                return;
                            }

                            encoder = match create_encoder(file_index, &static_messages) {
                                Ok(e) => e,
                                Err(err) => {
                                    re_log::error!("Failed to create new encoder during manual rotation: {err}");
                                    return;
                                }
                            };
                            file_index += 1;
                            current_size = 0;
                        }
                    }
                }

                // Finish the last file
                if let Err(err) = encoder.finish() {
                    re_log::error!("Failed to end log stream: {err}");
                    return;
                }
                re_log::debug!("Rotating file stream completed. {} files written.", file_index);
            }
        })
        .map_err(FileSinkError::SpawnThread)
}

impl fmt::Debug for FileSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSink")
            .field("path", &self.path.clone().unwrap_or("stdout".into()))
            .finish_non_exhaustive()
    }
}
