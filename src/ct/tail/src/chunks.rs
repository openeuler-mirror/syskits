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

//! Iterating over a file by chunks, either starting at the end of the file with [`TailReverseChunks`]
//! or at the end of piped stdin with [`TailLinesChunk`] or [`TailBytesChunk`].
//!
//! Use [`TailReverseChunks::new`] to create a new iterator over chunks of bytes from the file.

use ctcore::ct_error::CTResult;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};

/// When reading files in reverse in `bounded_tail`, this is the size of each
/// block read at a time.
pub const TAIL_BLOCK_SIZE: u64 = 1 << 16;

/// The size of the backing buffer of a LinesChunk or BytesChunk in bytes. The value of BUFFER_SIZE
/// originates from the BUFSIZ constant in stdio.h and the libc crate to make stream IO efficient.
/// In the latter the value is constantly set to 8192 on all platforms, where the value in stdio.h
/// is determined on each platform differently. Since libc chose 8192 as a reasonable default the
/// value here is set to this value, too.
pub const TAIL_BUFFER_SIZE: usize = 8192;

/// An iterator over a file in non-overlapping chunks from the end of the file.
///
/// Each chunk is a [`Vec`]<[`u8`]> of size [`TAIL_BLOCK_SIZE`] (except
/// possibly the last chunk, which might be smaller). Each call to
/// [`TailReverseChunks::next`] will seek backwards through the given file.
pub struct TailReverseChunks<'a> {
    /// The file to iterate over, by blocks, from the end to the beginning.
    file: &'a File,

    /// The total number of bytes in the file.
    size: u64,

    /// The total number of blocks to read.
    max_blocks_to_read: usize,

    /// The index of the next block to read.
    block_idx: usize,
}

impl<'a> TailReverseChunks<'a> {
    pub fn new(file: &'a mut File) -> TailReverseChunks<'a> {
        let current = if cfg!(unix) {
            file.stream_position().unwrap()
        } else {
            0
        };
        let size = file.seek(SeekFrom::End(0)).unwrap() - current;
        let max_blocks_to_read = (size as f64 / TAIL_BLOCK_SIZE as f64).ceil() as usize;
        let block_idx = 0;
        TailReverseChunks {
            file,
            size,
            max_blocks_to_read,
            block_idx,
        }
    }
}

impl Iterator for TailReverseChunks<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        // If there are no more chunks to read, terminate the iterator.
        if self.block_idx >= self.max_blocks_to_read {
            return None;
        }

        // The chunk size is `BLOCK_SIZE` for all but the last chunk
        // (that is, the chunk closest to the beginning of the file),
        // which contains the remainder of the bytes.
        let block_size = if self.block_idx == self.max_blocks_to_read - 1 {
            self.size % TAIL_BLOCK_SIZE
        } else {
            TAIL_BLOCK_SIZE
        };

        // Seek backwards by the next chunk, read the full chunk into
        // `buf`, and then seek back to the start of the chunk again.
        let mut buf = vec![0; TAIL_BLOCK_SIZE as usize];
        let pos = self
            .file
            .seek(SeekFrom::Current(-(block_size as i64)))
            .unwrap();
        self.file
            .read_exact(&mut buf[0..(block_size as usize)])
            .unwrap();
        let pos2 = self
            .file
            .seek(SeekFrom::Current(-(block_size as i64)))
            .unwrap();
        assert_eq!(pos, pos2);

        self.block_idx += 1;

        Some(buf[0..(block_size as usize)].to_vec())
    }
}

/// The type of the backing buffer of [`TailBytesChunk`] and [`TailLinesChunk`] which can hold
/// [`TAIL_BUFFER_SIZE`] elements at max.
type TailChunkBuffer = [u8; TAIL_BUFFER_SIZE];

/// A [`TailBytesChunk`] storing a fixed size number of bytes in a buffer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TailBytesChunk {
    /// The [`TailChunkBuffer`], an array storing the bytes, for example filled by
    /// [`TailBytesChunk::fill`]
    buffer: TailChunkBuffer,

    /// Stores the number of bytes, this buffer holds. This is not equal to buffer.len(), since the
    /// [`TailBytesChunk`] may store less bytes than the internal buffer can hold. In addition
    /// [`TailBytesChunk`] may be reused, what makes it necessary to track the number of stored bytes.
    /// The choice of usize is sufficient here, since the number of bytes max value is
    /// [`TAIL_BUFFER_SIZE`], which is a usize.
    bytes: usize,
}

impl TailBytesChunk {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            buffer: [0; TAIL_BUFFER_SIZE],
            bytes: 0,
        }
    }

    /// Create a new chunk from an existing chunk. The new chunk's buffer will be copied from the
    /// old chunk's buffer, copying the slice `[offset..old_chunk.bytes]` into the new chunk's
    /// buffer but starting at 0 instead of offset. If the offset is larger or equal to
    /// `chunk.lines` then a new empty `BytesChunk` is returned.
    ///
    /// # Arguments
    ///
    /// * `chunk`: The chunk to create a new `BytesChunk` chunk from
    /// * `offset`: Start to copy the old chunk's buffer from this position. May not be larger
    ///   than `chunk.bytes`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = BytesChunk::new();
    /// chunk.buffer[1] = 1;
    /// chunk.bytes = 2;
    /// let new_chunk = BytesChunk::from_chunk(&chunk, 0);
    /// assert_eq!(2, new_chunk.get_buffer().len());
    /// assert_eq!(&[0, 1], new_chunk.get_buffer());
    ///
    /// let new_chunk = BytesChunk::from_chunk(&chunk, 1);
    /// assert_eq!(1, new_chunk.get_buffer().len());
    /// assert_eq!(&[1], new_chunk.get_buffer());
    /// ```
    fn from_chunk(chunk: &Self, offset: usize) -> Self {
        if offset >= chunk.bytes {
            return Self::new();
        }

        let mut buffer: TailChunkBuffer = [0; TAIL_BUFFER_SIZE];
        let slice = chunk.get_buffer_with(offset);
        buffer[..slice.len()].copy_from_slice(slice);
        Self {
            buffer,
            bytes: chunk.bytes - offset,
        }
    }

    /// Receive the internal buffer safely, so it returns a slice only containing as many bytes as
    /// large the `self.bytes` value is.
    ///
    /// returns: a slice containing the bytes of the internal buffer from `[0..self.bytes]`
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = BytesChunk::new();
    /// chunk.bytes = 1;
    /// assert_eq!(&[0], chunk.get_buffer());
    /// ```
    pub fn get_buffer(&self) -> &[u8] {
        &self.buffer[..self.bytes]
    }

    /// Like [`TailBytesChunk::get_buffer`], but returning a slice from `[offset.self.bytes]`.
    ///
    /// returns: a slice containing the bytes of the internal buffer from `[offset..self.bytes]`
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = BytesChunk::new();
    /// chunk.bytes = 2;
    /// assert_eq!(&[0], chunk.get_buffer_with(1));
    /// ```
    pub fn get_buffer_with(&self, offset: usize) -> &[u8] {
        &self.buffer[offset..self.bytes]
    }

    pub fn has_data(&self) -> bool {
        self.bytes > 0
    }

    /// Fills `self.buffer` with maximal [`TAIL_BUFFER_SIZE`] number of bytes, draining the reader by
    /// that number of bytes. If EOF is reached (so 0 bytes are read), then returns
    /// [`CTResult<None>`] or else the result with [`Some(bytes)`] where bytes is the number of bytes
    /// read from the source.
    pub fn fill(&mut self, filehandle: &mut impl BufRead) -> CTResult<Option<usize>> {
        let num_bytes = filehandle.read(&mut self.buffer)?;
        self.bytes = num_bytes;
        if num_bytes == 0 {
            return Ok(None);
        }

        Ok(Some(self.bytes))
    }
}

/// An abstraction layer on top of [`TailBytesChunk`] mainly to simplify filling only the needed amount
/// of chunks. See also [`Self::fill`].
pub struct TailBytesChunkBuffer {
    /// The number of bytes to print
    num_print: u64,
    /// The current number of bytes summed over all stored chunks in [`Self::chunks`]. Use u64 here
    /// to support files > 4GB on 32-bit systems. Note, this differs from `BytesChunk::bytes` which
    /// is a usize. The choice of u64 is based on `tail::FilterMode::Bytes`.
    bytes: u64,
    /// The buffer to store [`TailBytesChunk`] in
    chunks: VecDeque<Box<TailBytesChunk>>,
}

impl TailBytesChunkBuffer {
    /// Creates a new [`TailBytesChunkBuffer`].
    ///
    /// # Arguments
    ///
    /// * `num_print`: The number of bytes to print
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = BytesChunk::new();
    /// chunk.buffer[1] = 1;
    /// chunk.bytes = 2;
    /// let new_chunk = BytesChunk::from_chunk(&chunk, 0);
    /// assert_eq!(2, new_chunk.get_buffer().len());
    /// assert_eq!(&[0, 1], new_chunk.get_buffer());
    ///
    /// let new_chunk = BytesChunk::from_chunk(&chunk, 1);
    /// assert_eq!(1, new_chunk.get_buffer().len());
    /// assert_eq!(&[1], new_chunk.get_buffer());
    /// ```
    pub fn new(num_print: u64) -> Self {
        Self {
            bytes: 0,
            num_print,
            chunks: VecDeque::new(),
        }
    }

    /// Fills this buffer with chunks and consumes the reader completely. This method ensures that
    /// there are exactly as many chunks as needed to match `self.num_print` bytes, so there are
    /// in sum exactly `self.num_print` bytes stored in all chunks. The method returns an iterator
    /// over these chunks. If there are no chunks, for example because the piped stdin contained no
    /// bytes, or `num_print = 0` then `iterator.next` returns None.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use crate::chunks::BytesChunkBuffer;
    /// use std::io::{BufReader, Cursor};
    ///
    /// let mut reader = BufReader::new(Cursor::new(""));
    /// let num_print = 0;
    /// let mut chunks = BytesChunkBuffer::new(num_print);
    /// chunks.fill(&mut reader).unwrap();
    ///
    /// let mut reader = BufReader::new(Cursor::new("a"));
    /// let num_print = 1;
    /// let mut chunks = BytesChunkBuffer::new(num_print);
    /// chunks.fill(&mut reader).unwrap();
    /// ```
    pub fn fill(&mut self, reader: &mut impl BufRead) -> CTResult<()> {
        let mut chunk = Box::new(TailBytesChunk::new());

        // fill chunks with all bytes from reader and reuse already instantiated chunks if possible
        while (chunk.fill(reader)?).is_some() {
            self.bytes += chunk.bytes as u64;
            self.chunks.push_back(chunk);

            let first = &self.chunks[0];
            if self.bytes - first.bytes as u64 > self.num_print {
                chunk = self.chunks.pop_front().unwrap();
                self.bytes -= chunk.bytes as u64;
            } else {
                chunk = Box::new(TailBytesChunk::new());
            }
        }

        // quit early if there are no chunks for example in case the pipe was empty
        if self.chunks.is_empty() {
            return Ok(());
        }

        let chunk = self.chunks.pop_front().unwrap();

        // calculate the offset in the first chunk and put the calculated chunk as first element in
        // the self.chunks collection. The calculated offset must be in the range 0 to BUFFER_SIZE
        // and is therefore safely convertible to a usize without losses.
        let offset = self.bytes.saturating_sub(self.num_print) as usize;
        self.chunks
            .push_front(Box::new(TailBytesChunk::from_chunk(&chunk, offset)));

        Ok(())
    }

    pub fn print(&self, mut writer: impl Write) -> CTResult<()> {
        for chunk in &self.chunks {
            writer.write_all(chunk.get_buffer())?;
        }
        Ok(())
    }

    pub fn has_data(&self) -> bool {
        !self.chunks.is_empty()
    }
}

/// Works similar to a [`TailBytesChunk`] but also stores the number of lines encountered in the current
/// buffer. The size of the buffer is limited to a fixed size number of bytes.
#[derive(Debug)]
pub struct TailLinesChunk {
    /// Work on top of a [`TailBytesChunk`]
    chunk: TailBytesChunk,
    /// The number of lines delimited by `delimiter`. The choice of usize is sufficient here,
    /// because lines max value is the number of bytes contained in this chunk's buffer, and the
    /// number of bytes max value is [`TAIL_BUFFER_SIZE`], which is a usize.
    lines: usize,
    /// The delimiter to use, to count the lines
    delimiter: u8,
}

impl TailLinesChunk {
    pub fn new(delimiter: u8) -> Self {
        Self {
            chunk: TailBytesChunk::new(),
            lines: 0,
            delimiter,
        }
    }

    /// Count the number of lines delimited with [`Self::delimiter`] contained in the buffer.
    /// Currently [`memchr`] is used because performance is better than using an iterator or for
    /// loop.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = LinesChunk::new(b'\n');
    /// chunk.buffer[0..12].copy_from_slice("hello\nworld\n".as_bytes());
    /// chunk.bytes = 12;
    /// assert_eq!(2, chunk.count_lines());
    ///
    /// chunk.buffer[0..14].copy_from_slice("hello\r\nworld\r\n".as_bytes());
    /// chunk.bytes = 14;
    /// assert_eq!(2, chunk.count_lines());
    /// ```
    fn count_lines(&self) -> usize {
        memchr::memchr_iter(self.delimiter, self.get_buffer()).count()
    }

    /// Creates a new [`TailLinesChunk`] from an existing one with an offset in lines. The new chunk
    /// contains exactly `chunk.lines - offset` lines. The offset in bytes is calculated and applied
    /// to the new chunk, so the new chunk contains only the bytes encountered after the offset in
    /// number of lines and the `delimiter`. If the offset is larger than `chunk.lines` then a new
    /// empty `LinesChunk` is returned.
    ///
    /// # Arguments
    ///
    /// * `chunk`: The chunk to create the new chunk from
    /// * `offset`: The offset in number of lines (not bytes)
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = LinesChunk::new(b'\n');
    /// // manually filling the buffer and setting the correct values for bytes and lines
    /// chunk.buffer[0..12].copy_from_slice("hello\nworld\n".as_bytes());
    /// chunk.bytes = 12;
    /// chunk.lines = 2;
    ///
    /// let offset = 1; // offset in number of lines
    /// let new_chunk = LinesChunk::from(&chunk, offset);
    /// assert_eq!("world\n".as_bytes(), new_chunk.get_buffer());
    /// assert_eq!(6, new_chunk.bytes);
    /// assert_eq!(1, new_chunk.lines);
    /// ```
    fn from_chunk(chunk: &Self, offset: usize) -> Self {
        if offset > chunk.lines {
            return Self::new(chunk.delimiter);
        }

        let bytes_offset = chunk.calculate_bytes_offset_from(offset);
        let new_chunk = TailBytesChunk::from_chunk(&chunk.chunk, bytes_offset);

        Self {
            chunk: new_chunk,
            lines: chunk.lines - offset,
            delimiter: chunk.delimiter,
        }
    }

    /// Returns true if this buffer has stored any bytes.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = LinesChunk::new(b'\n');
    /// assert!(!chunk.has_data());
    ///
    /// chunk.buffer[0] = 1;
    /// assert!(!chunk.has_data());
    ///
    /// chunk.bytes = 1;
    /// assert!(chunk.has_data());
    /// ```
    pub fn has_data(&self) -> bool {
        self.chunk.has_data()
    }

    /// Returns this buffer safely. See [`TailBytesChunk::get_buffer`]
    ///
    /// returns: &[u8] with length `self.bytes`
    pub fn get_buffer(&self) -> &[u8] {
        self.chunk.get_buffer()
    }

    /// Returns this buffer safely with an offset applied. See [`TailBytesChunk::get_buffer_with`].
    ///
    /// returns: &[u8] with length `self.bytes - offset`
    pub fn get_buffer_with(&self, offset: usize) -> &[u8] {
        self.chunk.get_buffer_with(offset)
    }

    /// Return the number of lines the buffer contains. `self.lines` needs to be set before the call
    /// to this function returns the correct value. If the calculation of lines is needed then
    /// use `self.count_lines`.
    pub fn get_lines(&self) -> usize {
        self.lines
    }

    /// Fills `self.buffer` with maximal [`TAIL_BUFFER_SIZE`] number of bytes, draining the reader by
    /// that number of bytes. This function works like the [`TailBytesChunk::fill`] function besides
    /// that this function also counts and stores the number of lines encountered while reading from
    /// the `filehandle`.
    pub fn fill(&mut self, filehandle: &mut impl BufRead) -> CTResult<Option<usize>> {
        match self.chunk.fill(filehandle)? {
            None => {
                self.lines = 0;
                Ok(None)
            }
            Some(bytes) => {
                self.lines = self.count_lines();
                Ok(Some(bytes))
            }
        }
    }

    /// Calculates the offset in bytes within this buffer from the offset in number of lines. The
    /// resulting offset is 0-based and points to the byte after the delimiter.
    ///
    /// # Arguments
    ///
    /// * `offset`: the offset in number of lines. If offset is 0 then 0 is returned, if larger than
    ///   the contained lines then self.bytes is returned.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut chunk = LinesChunk::new(b'\n');
    /// chunk.buffer[0..12].copy_from_slice("hello\nworld\n".as_bytes());
    /// chunk.bytes = 12;
    /// chunk.lines = 2; // note that if not setting lines the result might not be what is expected
    /// let bytes_offset = chunk.calculate_bytes_offset_from(1);
    /// assert_eq!(6, bytes_offset);
    /// assert_eq!(
    ///     "world\n",
    ///     String::from_utf8_lossy(chunk.get_buffer_with(bytes_offset)));
    /// ```
    fn calculate_bytes_offset_from(&self, offset: usize) -> usize {
        let mut lines_offset = offset;
        let mut bytes_offset = 0;
        for byte in self.get_buffer() {
            if lines_offset == 0 {
                break;
            }
            if byte == &self.delimiter {
                lines_offset -= 1;
            }
            bytes_offset += 1;
        }
        bytes_offset
    }

    /// Print the bytes contained in this buffer calculated with the given offset in number of
    /// lines.
    ///
    /// # Arguments
    ///
    /// * `writer`: must implement [`Write`]
    /// * `offset`: An offset in number of lines.
    pub fn print_lines(&self, writer: &mut impl Write, offset: usize) -> CTResult<()> {
        self.print_bytes(writer, self.calculate_bytes_offset_from(offset))
    }

    /// Print the bytes contained in this buffer beginning from the given offset in number of bytes.
    ///
    /// # Arguments
    ///
    /// * `writer`: must implement [`Write`]
    /// * `offset`: An offset in number of bytes.
    pub fn print_bytes(&self, writer: &mut impl Write, offset: usize) -> CTResult<()> {
        writer.write_all(self.get_buffer_with(offset))?;
        Ok(())
    }
}

/// An abstraction layer on top of [`TailLinesChunk`] mainly to simplify filling only the needed amount
/// of chunks. See also [`Self::fill`]. Works similar like [`TailBytesChunkBuffer`], but works on top
/// of lines delimited by `self.delimiter` instead of bytes.
pub struct TailLinesChunkBuffer {
    /// The delimiter to recognize a line. Any [`u8`] is allowed.
    delimiter: u8,
    /// The amount of lines occurring in all currently stored [`TailLinesChunk`]s. Use u64 here to
    /// support files > 4GB on 32-bit systems. Note, this differs from [`TailLinesChunk::lines`] which
    /// is a usize. The choice of u64 is based on `tail::FilterMode::Lines`.
    lines: u64,
    /// The amount of lines to print.
    num_print: u64,
    /// Stores the [`TailLinesChunk`]
    chunks: VecDeque<Box<TailLinesChunk>>,
}

impl TailLinesChunkBuffer {
    /// Create a new [`TailLinesChunkBuffer`]
    pub fn new(delimiter: u8, num_print: u64) -> Self {
        Self {
            delimiter,
            num_print,
            lines: 0,
            chunks: VecDeque::new(),
        }
    }

    /// Fills this buffer with chunks and consumes the reader completely. This method ensures that
    /// there are exactly as many chunks as needed to match `self.num_print` lines, so there are
    /// in sum exactly `self.num_print` lines stored in all chunks. The method returns an iterator
    /// over these chunks. If there are no chunks, for example because the piped stdin contained no
    /// lines, or `num_print = 0` then `iterator.next` will return None.
    pub fn fill(&mut self, reader: &mut impl BufRead) -> CTResult<()> {
        let mut chunk = Box::new(TailLinesChunk::new(self.delimiter));

        while (chunk.fill(reader)?).is_some() {
            self.lines += chunk.lines as u64;
            self.chunks.push_back(chunk);

            let first = &self.chunks[0];
            if self.lines - first.lines as u64 > self.num_print {
                chunk = self.chunks.pop_front().unwrap();

                self.lines -= chunk.lines as u64;
            } else {
                chunk = Box::new(TailLinesChunk::new(self.delimiter));
            }
        }

        if self.chunks.is_empty() {
            // chunks is empty when a file is empty so quitting early here
            return Ok(());
        } else {
            let length = &self.chunks.len();
            let last = &mut self.chunks[length - 1];
            if !last.get_buffer().ends_with(&[self.delimiter]) {
                last.lines += 1;
                self.lines += 1;
            }
        }

        // skip unnecessary chunks and save the first chunk which may hold some lines we have to
        // print
        let chunk = loop {
            // it's safe to call unwrap here because there is at least one chunk and sorting out
            // more chunks than exist shouldn't be possible.
            let chunk = self.chunks.pop_front().unwrap();

            // skip is true as long there are enough lines left in the other stored chunks.
            let skip = self.lines - chunk.lines as u64 > self.num_print;
            if skip {
                self.lines -= chunk.lines as u64;
            } else {
                break chunk;
            }
        };

        // Calculate the number of lines to skip in the current chunk. The calculated value must be
        // in the range 0 to BUFFER_SIZE and is therefore safely convertible to a usize without
        // losses.
        let skip_lines = self.lines.saturating_sub(self.num_print) as usize;
        let chunk = TailLinesChunk::from_chunk(&chunk, skip_lines);
        self.chunks.push_front(Box::new(chunk));

        Ok(())
    }

    pub fn print(&self, mut writer: impl Write) -> CTResult<()> {
        for chunk in &self.chunks {
            chunk.print_bytes(&mut writer, 0)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::chunks::{TAIL_BUFFER_SIZE, TailBytesChunk};

    #[test]
    fn test_bytes_chunk_from_when_offset_is_zero() {
        let mut chunk = TailBytesChunk::new();
        chunk.bytes = TAIL_BUFFER_SIZE;
        chunk.buffer[1] = 1;
        let other = TailBytesChunk::from_chunk(&chunk, 0);
        assert_eq!(other, chunk);

        chunk.bytes = 2;
        let other = TailBytesChunk::from_chunk(&chunk, 0);
        assert_eq!(other, chunk);

        chunk.bytes = 1;
        let other = TailBytesChunk::from_chunk(&chunk, 0);
        assert_eq!(other.buffer, [0; TAIL_BUFFER_SIZE]);
        assert_eq!(other.bytes, chunk.bytes);

        chunk.bytes = TAIL_BUFFER_SIZE;
        let other = TailBytesChunk::from_chunk(&chunk, 2);
        assert_eq!(other.buffer, [0; TAIL_BUFFER_SIZE]);
        assert_eq!(other.bytes, TAIL_BUFFER_SIZE - 2);
    }

    #[test]
    fn test_bytes_chunk_from_when_offset_is_not_zero() {
        let mut chunk = TailBytesChunk::new();
        chunk.bytes = TAIL_BUFFER_SIZE;
        chunk.buffer[1] = 1;

        let other = TailBytesChunk::from_chunk(&chunk, 1);
        let mut expected_buffer = [0; TAIL_BUFFER_SIZE];
        expected_buffer[0] = 1;
        assert_eq!(other.buffer, expected_buffer);
        assert_eq!(other.bytes, TAIL_BUFFER_SIZE - 1);

        let other = TailBytesChunk::from_chunk(&chunk, 2);
        assert_eq!(other.buffer, [0; TAIL_BUFFER_SIZE]);
        assert_eq!(other.bytes, TAIL_BUFFER_SIZE - 2);
    }

    #[test]
    fn test_bytes_chunk_from_when_offset_is_larger_than_chunk_size_1() {
        let mut chunk = TailBytesChunk::new();
        chunk.bytes = TAIL_BUFFER_SIZE;
        let new_chunk = TailBytesChunk::from_chunk(&chunk, TAIL_BUFFER_SIZE + 1);
        assert_eq!(0, new_chunk.bytes);
    }

    #[test]
    fn test_bytes_chunk_from_when_offset_is_larger_than_chunk_size_2() {
        let mut chunk = TailBytesChunk::new();
        chunk.bytes = 0;
        let new_chunk = TailBytesChunk::from_chunk(&chunk, 1);
        assert_eq!(0, new_chunk.bytes);
    }

    #[test]
    fn test_bytes_chunk_from_when_offset_is_larger_than_chunk_size_3() {
        let mut chunk = TailBytesChunk::new();
        chunk.bytes = 1;
        let new_chunk = TailBytesChunk::from_chunk(&chunk, 2);
        assert_eq!(0, new_chunk.bytes);
    }

    #[test]
    fn test_bytes_chunk_from_when_offset_is_equal_to_chunk_size() {
        let mut chunk = TailBytesChunk::new();
        chunk.buffer[0] = 1;
        chunk.bytes = 1;
        let new_chunk = TailBytesChunk::from_chunk(&chunk, 1);
        assert_eq!(0, new_chunk.bytes);
    }
}
