/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2.
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2.
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

//! Stream producer helper for bridging writer-style command cores to `CtByteStream`.
//!
//! The producer writes to a pipe on a background thread; downstream reads from
//! the pipe through `CtPipelineData::ByteStream`.

use crate::{CtByteStream, CtPipelineMetadata};
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// Cooperative cancel flag for long-running producers (for example `tail -f`).
pub type CancelFlag = Arc<AtomicBool>;

/// Create a new cancel flag with initial value `false`.
pub fn new_cancel_flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

/// Reader + producer-thread wrapper.
///
/// This type implements `Read` and can be passed to `CtByteStream::new`.
/// On drop, it sets cancel and joins the producer thread.
pub struct CtByteStreamWithProducer {
    reader: Option<os_pipe::PipeReader>,
    producer: Option<JoinHandle<io::Result<()>>>,
    cancel: CancelFlag,
}

impl CtByteStreamWithProducer {
    fn finish_producer_if_needed(&mut self) -> io::Result<()> {
        if let Some(handle) = self.producer.take() {
            match handle.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
                Ok(Err(err)) => Err(err),
                Err(_) => Err(io::Error::other("producer thread panicked")),
            }
        } else {
            Ok(())
        }
    }

    /// Spawn a producer thread with a fresh cancel flag and wrap it as `CtByteStream`.
    pub fn spawn<F>(metadata: CtPipelineMetadata, writer_fn: F) -> io::Result<CtByteStream>
    where
        F: FnOnce(os_pipe::PipeWriter, CancelFlag) -> io::Result<()> + Send + 'static,
    {
        Self::spawn_with_cancel(metadata, new_cancel_flag(), writer_fn)
    }

    /// Spawn a producer thread with an externally provided cancel flag.
    pub fn spawn_with_cancel<F>(
        metadata: CtPipelineMetadata,
        cancel: CancelFlag,
        writer_fn: F,
    ) -> io::Result<CtByteStream>
    where
        F: FnOnce(os_pipe::PipeWriter, CancelFlag) -> io::Result<()> + Send + 'static,
    {
        let (reader, writer) = os_pipe::pipe()?;
        let cancel_clone = cancel.clone();
        let producer = std::thread::spawn(move || writer_fn(writer, cancel_clone));

        let produced = CtByteStreamWithProducer {
            reader: Some(reader),
            producer: Some(producer),
            cancel,
        };
        Ok(CtByteStream::new(produced, metadata))
    }
}

impl Read for CtByteStreamWithProducer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = match self.reader.as_mut() {
            Some(reader) => reader.read(buf)?,
            None => 0,
        };

        if n == 0 {
            self.finish_producer_if_needed()?;
        }

        Ok(n)
    }
}

impl Drop for CtByteStreamWithProducer {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Close read end first so producer blocked in write() can get BrokenPipe.
        let _ = self.reader.take();

        if let Some(handle) = self.producer.take() {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) if err.kind() == io::ErrorKind::BrokenPipe => {}
                Ok(Err(_)) => {}
                Err(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn stream_producer_spawn_writes_bytes() {
        let meta = CtPipelineMetadata::default();
        let mut stream = CtByteStreamWithProducer::spawn(meta, |mut writer, _cancel| {
            writer.write_all(b"hello stream")?;
            writer.flush()?;
            Ok(())
        })
        .expect("spawn producer");

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).expect("read stream");
        assert_eq!(buf, b"hello stream");
    }

    #[test]
    fn stream_producer_drop_sets_cancel() {
        let meta = CtPipelineMetadata::default();
        let observed_cancel = Arc::new(AtomicBool::new(false));
        let observed_cancel_clone = observed_cancel.clone();
        let cancel = new_cancel_flag();

        let mut stream =
            CtByteStreamWithProducer::spawn_with_cancel(meta, cancel, move |mut w, c| {
                loop {
                    if c.load(Ordering::Relaxed) {
                        observed_cancel_clone.store(true, Ordering::Relaxed);
                        return Ok(());
                    }
                    if let Err(err) = w.write_all(b"x") {
                        if err.kind() == io::ErrorKind::BrokenPipe {
                            observed_cancel_clone.store(true, Ordering::Relaxed);
                            return Ok(());
                        }
                        return Err(err);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .expect("spawn producer");

        let mut one = [0_u8; 1];
        let _ = stream.read(&mut one).expect("read one byte");
        drop(stream);

        for _ in 0..100 {
            if observed_cancel.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(observed_cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn stream_producer_drop_unblocks_writer() {
        let meta = CtPipelineMetadata::default();
        let stream = CtByteStreamWithProducer::spawn(meta, |mut writer, _cancel| {
            loop {
                writer.write_all(&[0_u8; 4096])?;
            }
        })
        .expect("spawn producer");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(stream);
            let _ = tx.send(());
        });

        rx.recv_timeout(Duration::from_secs(1))
            .expect("drop should not block");
    }

    #[test]
    fn stream_producer_core_error_path_closes_stream() {
        let meta = CtPipelineMetadata::default();
        let mut stream = CtByteStreamWithProducer::spawn(meta, |mut writer, _cancel| {
            writer.write_all(b"partial")?;
            Err(io::Error::other("core failure"))
        })
        .expect("spawn producer");

        let mut buf = Vec::new();
        let err = stream
            .read_to_end(&mut buf)
            .expect_err("stream should surface producer error");
        assert_eq!(buf, b"partial");
        assert!(
            err.to_string().contains("core failure"),
            "error should include producer failure cause"
        );
    }
}
