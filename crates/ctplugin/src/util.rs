use crate::error::PluginError;
use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const DEFAULT_PLUGIN_TIMEOUT_MS: u64 = 30_000;
pub(crate) const DEFAULT_PLUGIN_MAX_FRAME_BYTES: usize = 1_048_576;

pub(crate) fn protocol_timeout_ms() -> u64 {
    std::env::var("SYSKITS_PLUGIN_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_PLUGIN_TIMEOUT_MS)
}

pub(crate) fn protocol_max_frame_bytes() -> usize {
    std::env::var("SYSKITS_PLUGIN_MAX_FRAME_BYTES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_PLUGIN_MAX_FRAME_BYTES)
}

pub(crate) fn spawn_timeout_killer(
    child_id: u32,
    timeout_ms: u64,
    timed_out: Arc<AtomicBool>,
    process_done: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
        if process_done.load(Ordering::Relaxed) {
            return;
        }
        timed_out.store(true, Ordering::Relaxed);
        #[cfg(unix)]
        unsafe {
            libc::kill(child_id as i32, libc::SIGTERM);
            std::thread::sleep(std::time::Duration::from_millis(50));
            libc::kill(child_id as i32, libc::SIGKILL);
        }
    });
}

/// Read one newline-delimited plugin frame with bounded memory.
///
/// This uses `BufRead::fill_buf` + `consume` to process chunks incrementally,
/// so oversized frames are detected early without allocating an unbounded buffer.
/// When a frame exceeds `max_frame_bytes`, the rest of that frame is drained up to
/// the next newline to keep subsequent reads aligned.
pub(crate) fn read_frame_line(
    reader: &mut impl BufRead,
    max_frame_bytes: usize,
) -> Result<Option<String>, PluginError> {
    let mut out = Vec::new();

    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            break;
        }

        let newline_at = chunk.iter().position(|&b| b == b'\n');
        let take_len = newline_at.map(|i| i + 1).unwrap_or(chunk.len());

        if out.len() + take_len > max_frame_bytes {
            reader.consume(take_len);
            if newline_at.is_none() {
                drain_until_newline(reader)?;
            }
            return Err(PluginError::Protocol(format!(
                "plugin frame too large: {} bytes > limit {} bytes",
                out.len() + take_len,
                max_frame_bytes
            )));
        }

        out.extend_from_slice(&chunk[..take_len]);
        reader.consume(take_len);

        if newline_at.is_some() {
            break;
        }
    }

    if out.is_empty() {
        return Ok(None);
    }

    let line = String::from_utf8(out)
        .map_err(|e| PluginError::Protocol(format!("plugin frame is not valid UTF-8: {e}")))?;
    Ok(Some(line))
}

fn drain_until_newline(reader: &mut impl BufRead) -> Result<(), PluginError> {
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(());
        }
        if let Some(i) = chunk.iter().position(|&b| b == b'\n') {
            reader.consume(i + 1);
            return Ok(());
        }
        let len = chunk.len();
        reader.consume(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn read_frame_line_ok() {
        let mut reader = Cursor::new(b"{\"k\":1}\n".to_vec());
        let line = read_frame_line(&mut reader, 1024).expect("must read");
        assert_eq!(line.as_deref(), Some("{\"k\":1}\n"));
    }

    #[test]
    fn read_frame_line_too_large_and_drains_line() {
        let mut reader = Cursor::new(b"abcdef\ngood\n".to_vec());
        let first = read_frame_line(&mut reader, 4);
        assert!(first.is_err());

        let second = read_frame_line(&mut reader, 16).expect("must read second line");
        assert_eq!(second.as_deref(), Some("good\n"));
    }
}
