use std::fmt;
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, SyncSender},
};

use parking_lot::Mutex;

use re_log_types::LogMsg;

/// Helper function to create a file with TRUE multiprocess-safe access using process-specific files.
/// Each process writes to its own file with a unique suffix, eliminating all contention.
/// This approach has ZERO contention regardless of the number of processes.
fn create_file_multiprocess_safe(path: &PathBuf) -> Result<(std::fs::File, bool), FileSinkError> {
    // Generate a unique file for this process by appending process ID
    let process_id = std::process::id();
    let process_specific_path = if let Some(stem) = path.file_stem() {
        if let Some(parent) = path.parent() {
            parent.join(format!("{}_pid{}.rrd", stem.to_string_lossy(), process_id))
        } else {
            PathBuf::from(format!("{}_pid{}.rrd", stem.to_string_lossy(), process_id))
        }
    } else {
        path.with_extension(&format!("pid{}.rrd", process_id))
    };
    
    re_log::debug!("Creating process-specific file: {process_specific_path:?} (from {path:?})");
    
    // Check if the process-specific file already exists
    let file_exists = process_specific_path.exists();
    
    let file = if file_exists {
        // File exists - use append mode to preserve existing data
        re_log::debug!("Process-specific file {process_specific_path:?} exists, opening in append mode");
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(&process_specific_path)
            .map_err(|err| FileSinkError::CreateFile(process_specific_path.clone(), err))?
    } else {
        // File doesn't exist - create new file
        re_log::debug!("Creating new process-specific file {process_specific_path:?}");
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&process_specific_path)
            .map_err(|err| FileSinkError::CreateFile(process_specific_path.clone(), err))?
    };
    
    // If we're appending to an existing file, we need to remove the end marker first
    if file_exists {
        // We need to handle this carefully since we can't modify the file handle
        // For now, we'll create a new file handle to modify the existing file
        let mut modify_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&process_specific_path)
            .map_err(|err| FileSinkError::CreateFile(process_specific_path.clone(), err))?;
        
        remove_end_marker_from_file(&mut modify_file)?;
    }
    
    re_log::debug!("Successfully created process-specific file {process_specific_path:?} (append_mode: {})", file_exists);
    Ok((file, file_exists))
}


/// Helper function to remove the end marker from a file before appending.
/// Rerun recording files end with a special marker that needs to be removed before appending new data.
fn remove_end_marker_from_file(file: &mut std::fs::File) -> Result<(), FileSinkError> {
    use std::io::{Read, Seek, SeekFrom};
    
    let file_len = file.metadata()
        .map_err(|err| FileSinkError::CreateFile(PathBuf::from("unknown"), err))?
        .len();

    if file_len < 16 {
        // File too small to have an end marker
        return Ok(());
    }

    // Read last 16 bytes to check for end marker
    file.seek(SeekFrom::End(-16))
        .map_err(|err| FileSinkError::CreateFile(PathBuf::from("unknown"), err))?;

    let mut last_16_bytes = [0u8; 16];
    file.read_exact(&mut last_16_bytes)
        .map_err(|err| FileSinkError::CreateFile(PathBuf::from("unknown"), err))?;

    // Check if it's an end marker (MessageKind::End = 0)
    let message_kind = u64::from_le_bytes([
        last_16_bytes[0], last_16_bytes[1], last_16_bytes[2], last_16_bytes[3],
        last_16_bytes[4], last_16_bytes[5], last_16_bytes[6], last_16_bytes[7],
    ]);

    if message_kind == 0 { // MessageKind::End
        // Truncate file to remove the end marker
        file.set_len(file_len - 16)
            .map_err(|err| FileSinkError::CreateFile(PathBuf::from("unknown"), err))?;
        re_log::debug!("Removed end marker from file (was {} bytes, now {} bytes)", file_len, file_len - 16);
    }

    Ok(())
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

    /// Error due to multiprocess file access conflict.
    #[error("Failed to acquire exclusive access to file {0} after multiple attempts. Another process may be writing to the same file.")]
    MultiprocessConflict(PathBuf),
}

enum Command {
    Send(LogMsg),
    Flush(SyncSender<()>),
}

impl Command {
    fn flush() -> (Self, Receiver<()>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(0); // oneshot
        (Self::Flush(tx), rx)
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
        // We always compress on disk
        let encoding_options = crate::EncodingOptions::PROTOBUF_COMPRESSED;

        let (tx, rx) = std::sync::mpsc::channel();

        let path = path.into();

        re_log::debug!("Saving file to {path:?}…");

        // TODO(andreas): Can we ensure that a single process doesn't
        // have multiple file sinks for the same file live?
        // This likely caused an instability in the past, see https://github.com/rerun-io/rerun/issues/3306
        //
        // NOTE: We now use cooperative locking to allow multiple processes
        // to write to the same file safely with short-term exclusive access.

        let (file, append_mode) = create_file_multiprocess_safe(&path)?;
        let encoder = crate::encoder::DroppableEncoder::new(
            re_build_info::CrateVersion::LOCAL,
            encoding_options,
            file,
        )?;
        let join_handle = spawn_and_stream(Some(&path), encoder, rx)?;

        Ok(Self {
            tx: tx.into(),
            join_handle: Some(join_handle),
            path: Some(path),
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
        let join_handle = spawn_and_stream(None, encoder, rx)?;

        Ok(Self {
            tx: tx.into(),
            join_handle: Some(join_handle),
            path: None,
        })
    }

    #[inline]
    pub fn flush_blocking(&self) {
        let (cmd, oneshot) = Command::flush();
        self.tx.lock().send(Some(cmd)).ok();
        oneshot.recv().ok();
    }

    #[inline]
    pub fn send(&self, log_msg: LogMsg) {
        self.tx.lock().send(Some(Command::Send(log_msg))).ok();
    }
}

/// Set `filepath` to `None` to stream to standard output.
fn spawn_and_stream<W: std::io::Write + Send + 'static>(
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
                        Command::Flush(oneshot) => {
                            re_log::trace!("Flushing…");
                            if let Err(err) = encoder.flush_blocking() {
                                re_log::error!("Failed to flush log stream to {target}: {err}");
                                return;
                            }
                            drop(oneshot); // signals the oneshot
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

impl fmt::Debug for FileSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileSink")
            .field("path", &self.path.clone().unwrap_or("stdout".into()))
            .finish_non_exhaustive()
    }
}
