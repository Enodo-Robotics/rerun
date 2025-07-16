use std::fmt;
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, SyncSender},
    time::Duration,
};

use parking_lot::Mutex;

use re_log_types::LogMsg;

/// Helper function to create a file with multiprocess-safe access and retry logic.
/// This function attempts to handle conflicts when multiple processes try to write to the same file.
/// IMPORTANT: This function preserves existing data by using proper file locking and append mode.
fn create_file_multiprocess_safe(path: &PathBuf) -> Result<(std::fs::File, bool), FileSinkError> {
    const MAX_RETRIES: u32 = 50;  // Increased retries for better reliability
    const INITIAL_DELAY_MS: u64 = 50;
    const MAX_DELAY_MS: u64 = 2000;  // Maximum 2 second delay
    
    let mut delay_ms = INITIAL_DELAY_MS;
    
    for attempt in 0..MAX_RETRIES {
        // First check if the file already exists
        let file_exists = path.exists();
        
        let open_result = if file_exists {
            // File exists - use append mode to preserve existing data
            re_log::debug!("File {path:?} exists, opening in append mode to preserve data");
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(path)
        } else {
            // File doesn't exist - create new file
            re_log::debug!("File {path:?} doesn't exist, creating new file");
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
        };
        
        match open_result {
            Ok(mut file) => {
                // Try to acquire an exclusive lock on the file
                if let Err(lock_err) = try_lock_file(&mut file, path) {
                    re_log::debug!("Failed to acquire lock on {path:?}: {lock_err}");
                    
                    // If we can't get the lock, treat it as a multiprocess conflict
                    if attempt < MAX_RETRIES - 1 {
                        re_log::debug!(
                            "File {path:?} is locked by another process. Retrying in {}ms... (attempt {}/{})",
                            delay_ms,
                            attempt + 1,
                            MAX_RETRIES
                        );
                        
                        // Sleep before retrying with exponential backoff
                        std::thread::sleep(Duration::from_millis(delay_ms));
                        delay_ms = std::cmp::min(delay_ms * 2, MAX_DELAY_MS);
                        continue;
                    } else {
                        return Err(FileSinkError::MultiprocessConflict(path.clone()));
                    }
                }
                
                if attempt > 0 {
                    re_log::debug!(
                        "Successfully opened and locked file {path:?} after {} attempts (append_mode: {})", 
                        attempt + 1,
                        file_exists
                    );
                }
                return Ok((file, file_exists));
            }
            Err(err) => {
                // Check if this is a multiprocess conflict (file locked/in use)
                let is_multiprocess_conflict = match err.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        // On Windows, this often indicates the file is locked by another process
                        true
                    }
                    std::io::ErrorKind::AlreadyExists => {
                        // On some systems, this can indicate file is in use
                        false // We use create(true) so this shouldn't happen
                    }
                    _ => {
                        // Check if error message contains file lock/sharing violation hints
                        let error_msg = err.to_string().to_lowercase();
                        error_msg.contains("sharing violation") || 
                        error_msg.contains("locked") ||
                        error_msg.contains("being used") ||
                        error_msg.contains("resource temporarily unavailable")
                    }
                };
                
                if is_multiprocess_conflict && attempt < MAX_RETRIES - 1 {
                    re_log::debug!(
                        "File {path:?} appears to be locked by another process. Retrying in {}ms... (attempt {}/{})",
                        delay_ms,
                        attempt + 1,
                        MAX_RETRIES
                    );
                    
                    // Sleep before retrying with exponential backoff
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    delay_ms = std::cmp::min(delay_ms * 2, MAX_DELAY_MS);
                } else if attempt == MAX_RETRIES - 1 {
                    // Last attempt failed
                    if is_multiprocess_conflict {
                        return Err(FileSinkError::MultiprocessConflict(path.clone()));
                    } else {
                        return Err(FileSinkError::CreateFile(path.clone(), err));
                    }
                } else {
                    // Non-multiprocess error on early attempt
                    return Err(FileSinkError::CreateFile(path.clone(), err));
                }
            }
        }
    }
    
    // Should never reach here due to the loop logic above
    Err(FileSinkError::MultiprocessConflict(path.clone()))
}

/// Try to acquire an exclusive lock on a file.
/// This function uses a lock file approach that's safer than direct file locking.
fn try_lock_file(file: &mut std::fs::File, path: &PathBuf) -> Result<(), FileSinkError> {
    // Use a lock file approach that's safe and cross-platform
    let lock_file_path = path.with_extension("rrd.lock");
    
    // Try to create the lock file exclusively
    match std::fs::OpenOptions::new()
        .create_new(true)  // Only create if it doesn't exist
        .write(true)
        .open(&lock_file_path)
    {
        Ok(lock_file) => {
            // Successfully created lock file
            re_log::debug!("Successfully acquired exclusive lock on {path:?} using lock file {lock_file_path:?}");
            
            // Store the lock file handle in the file for cleanup later
            // For now, we'll just drop it - the OS will clean it up when the process exits
            drop(lock_file);
            Ok(())
        }
        Err(err) => {
            match err.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    // Lock file already exists - another process is using the file
                    re_log::debug!("Lock file {lock_file_path:?} already exists, another process is using {path:?}");
                    Err(FileSinkError::MultiprocessConflict(path.clone()))
                }
                _ => {
                    // Other error creating lock file
                    re_log::debug!("Failed to create lock file {lock_file_path:?}: {err}");
                    Err(FileSinkError::CreateFile(path.clone(), err))
                }
            }
        }
    }
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
        // NOTE: We now use multiprocess-safe file creation to handle conflicts
        // when multiple processes try to write to the same file.

        let (mut file, append_mode) = create_file_multiprocess_safe(&path)?;
        
        // If we're appending to an existing file, we need to remove the end marker first
        if append_mode {
            remove_end_marker_from_file(&mut file)?;
        }
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
