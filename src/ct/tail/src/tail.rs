/*
 * Copyright(c) 2022-2025 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

pub mod args;
pub mod chunks;
mod follow;
mod parse;
mod paths;
mod platform;
pub mod text;

pub use args::ct_app;
use args::{tail_parse_args, TailFilterMode, TailOptions, TailSignum};
use chunks::TailReverseChunks;
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code, CTResult, CtSimpleError, FromIo};
use ctcore::{ct_show, ct_show_error};
use follow::Observer;
use paths::{TailFileExtTail, TailHeaderPrinter, TailInput, TailInputKind, TailMetadataExt};
use same_file::Handle;
use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, stdin, stdout, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write, StdoutLock};
use std::path::{Path, PathBuf};

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    tail_main(args)
}

pub fn tail_main(args: impl ctcore::Args) -> CTResult<()> {
    let options = tail_parse_args(args)?;

    options.check_warnings();

    match options.verify() {
        args::TailVerificationResult::CannotFollowStdinByName => {
            return Err(CtSimpleError::new(
                1,
                format!("cannot follow {} by name", text::TAIL_DASH.quote()),
            ))
        }
        // Exit early if we do not output anything. Note, that this may break a pipe
        // when tail is on the receiving side.
        args::TailVerificationResult::NoOutput => return Ok(()),
        args::TailVerificationResult::Ok => {}
    }

    tail_exec(&options)
}

fn tail_exec(options: &TailOptions) -> CTResult<()> {
    let mut printer = TailHeaderPrinter::new(options.verbose, true);
    let mut observer = Observer::from(options);

    observer.start(options)?;
    // Do an initial tail print of each path's content.
    // Add `path` and `reader` to `files` map if `--follow` is selected.
    for input in &options.inputs.clone() {
        match input.kind() {
            TailInputKind::File(path)
                if cfg!(not(unix)) || path != &PathBuf::from(text::TAIL_DEV_STDIN) =>
            {
                tail_file(options, &mut printer, input, path, &mut observer, 0)?;
            }
            // File points to /dev/stdin here
            TailInputKind::File(_) | TailInputKind::Stdin => {
                tail_stdin(options, &mut printer, input, &mut observer)?;
            }
        }
    }

    if options.follow.is_some() {
        /*
        POSIX specification regarding tail -f
        If the input file is a regular file or if the file operand specifies a FIFO, do not
        terminate after the last line of the input file has been copied, but read and copy
        further bytes from the input file when they become available. If no file operand is
        specified and standard input is a pipe or FIFO, the -f option shall be ignored. If
        the input file is not a FIFO, pipe, or regular file, it is unspecified whether or
        not the -f option shall be ignored.
        */
        if !options.has_only_stdin() {
            follow::follow(observer, options)?;
        }
    }

    if get_ct_exit_code() > 0 && paths::tail_stdin_is_bad_fd() {
        ct_show_error!("-: {}", text::TAIL_BAD_FD);
    }

    Ok(())
}

fn tail_file(
    options: &TailOptions,
    header_printer: &mut TailHeaderPrinter,
    input: &TailInput,
    path: &Path,
    observer: &mut Observer,
    offset: u64,
) -> CTResult<()> {
    // 检查文件是否存在
    if !path.exists() {
        return handle_file_not_found(input, path, observer);
    }

    // 检查路径是否为目录
    if path.is_dir() {
        return handle_directory(input, path, options, observer);
    }

    // 检查路径是否可追踪
    if input.is_tailable() {
        return handle_tailable_file(options, header_printer, input, path, observer, offset);
    }

    observer.add_bad_path(path, input.display_name.as_str(), false)?;
    Ok(())
}

fn handle_file_not_found(input: &TailInput, path: &Path, observer: &mut Observer) -> CTResult<()> {
    set_ct_exit_code(1);
    ct_show_error!(
        "cannot open '{}' for reading: {}",
        input.display_name,
        text::TAIL_NO_SUCH_FILE
    );
    observer.add_bad_path(path, input.display_name.as_str(), false)?;
    Ok(())
}

fn handle_directory(
    input: &TailInput,
    path: &Path,
    options: &TailOptions,
    observer: &mut Observer,
) -> CTResult<()> {
    set_ct_exit_code(1);
    ct_show_error!("error reading '{}': Is a directory", input.display_name);
    observer.add_bad_path(path, input.display_name.as_str(), false)?;

    if options.follow.is_some() {
        let msg = if options.retry { "" } else { "; giving up on this name" };
        ct_show_error!(
            "{}: cannot follow end of this type of file{}",
            input.display_name,
            msg
        );
    }

    if !observer.follow_name_retry() {
        return Ok(());
    }

    Ok(())
}

fn handle_tailable_file(
    options: &TailOptions,
    header_printer: &mut TailHeaderPrinter,
    input: &TailInput,
    path: &Path,
    observer: &mut Observer,
    offset: u64,
) -> CTResult<()> {
    let metadata = path.metadata().ok();
    match File::open(path) {
        Ok(mut file) => {
            header_printer.print_input(input);
            let reader = if !options.presume_input_pipe
                && file.is_seekable(if input.is_stdin() { offset } else { 0 })
                && metadata.as_ref().unwrap().get_block_size() > 0
            {
                tail_bounded(&mut file, options);
                BufReader::new(file)
            } else {
                let mut reader = BufReader::new(file);
                tail_unbounded(&mut reader, options)?;
                reader
            };

            observer.add_path(
                path,
                input.display_name.as_str(),
                Some(Box::new(reader)),
                true,
            )?;
        }
        Err(e) => {
            handle_file_open_error(e, input, path, observer)?;
        }
    }

    Ok(())
}

fn handle_file_open_error(e: std::io::Error, input: &TailInput, path: &Path, observer: &mut Observer) -> CTResult<()> {
    observer.add_bad_path(path, input.display_name.as_str(), false)?;
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        ct_show!(e.map_err_context(|| {
            format!("cannot open '{}' for reading", input.display_name)
        }));
    } else {
        return Err(e.map_err_context(|| {
            format!("cannot open '{}' for reading", input.display_name)
        }));
    }
    Ok(())
}

fn tail_stdin(
    options: &TailOptions,
    header_printer: &mut TailHeaderPrinter,
    input: &TailInput,
    observer: &mut Observer,
) -> CTResult<()> {
    match input.resolve() {
        Some(path) => handle_fifo_stdin(options, header_printer, input, observer, &path),
        None => handle_pipe_stdin(options, header_printer, input, observer),
    }
}

fn handle_fifo_stdin(
    options: &TailOptions,
    header_printer: &mut TailHeaderPrinter,
    input: &TailInput,
    observer: &mut Observer,
    path: &Path,
) -> CTResult<()> {
    let stdin_offset = get_stdin_offset();
    tail_file(options, header_printer, input, path, observer, stdin_offset)
}

fn handle_pipe_stdin(
    options: &TailOptions,
    header_printer: &mut TailHeaderPrinter,
    input: &TailInput,
    observer: &mut Observer,
) -> CTResult<()> {
    header_printer.print_input(input);
    
    if paths::tail_stdin_is_bad_fd() {
        handle_bad_stdin_fd(options, input)?;
        return Ok(());
    }

    let mut reader = BufReader::new(stdin());
    tail_unbounded(&mut reader, options)?;
    observer.add_stdin(input.display_name.as_str(), Some(Box::new(reader)), true)?;
    Ok(())
}

fn get_stdin_offset() -> u64 {
    if cfg!(unix) {
        if let Ok(mut stdin_handle) = Handle::stdin() {
            if let Ok(offset) = stdin_handle.as_file_mut().stream_position() {
                return offset;
            }
        }
    }
    0
}

fn handle_bad_stdin_fd(options: &TailOptions, input: &TailInput) -> CTResult<()> {
    set_ct_exit_code(1);
    ct_show_error!(
        "cannot fstat {}: {}",
        text::TAIL_STDIN_HEADER.quote(),
        text::TAIL_BAD_FD
    );
    if options.follow.is_some() {
        ct_show_error!(
            "error reading {}: {}",
            text::TAIL_STDIN_HEADER.quote(),
            text::TAIL_BAD_FD
        );
    }
    Ok(())
}

/// Find the index after the given number of instances of a given byte.
///
/// This function reads through a given reader until `num_delimiters`
/// instances of `delimiter` have been seen, returning the index of
/// the byte immediately following that delimiter. If there are fewer
/// than `num_delimiters` instances of `delimiter`, this returns the
/// total number of bytes read from the `reader` until EOF.
///
/// # Errors
///
/// This function returns an error if there is an error during reading
/// from `reader`.
///
/// # Examples
///
/// Basic usage:
///
/// ```rust,ignore
/// use std::io::Cursor;
///
/// let mut reader = Cursor::new("a\nb\nc\nd\ne\n");
/// let i = forwards_thru_file(&mut reader, 2, b'\n').unwrap();
/// assert_eq!(i, 4);
/// ```
///
/// If `num_delimiters` is zero, then this function always returns
/// zero:
///
/// ```rust,ignore
/// use std::io::Cursor;
///
/// let mut reader = Cursor::new("a\n");
/// let i = forwards_thru_file(&mut reader, 0, b'\n').unwrap();
/// assert_eq!(i, 0);
/// ```
///
/// If there are fewer than `num_delimiters` instances of `delimiter`
/// in the reader, then this function returns the total number of
/// bytes read:
///
/// ```rust,ignore
/// use std::io::Cursor;
///
/// let mut reader = Cursor::new("a\n");
/// let i = forwards_thru_file(&mut reader, 2, b'\n').unwrap();
/// assert_eq!(i, 2);
/// ```
fn tail_forwards_thru_file<R>(
    reader: &mut R,
    num_delimiters: u64,
    delimiter: u8,
) -> std::io::Result<usize>
where
    R: Read,
{
    if num_delimiters == 0 {
        return Ok(0);
    }

    let mut reader = BufReader::new(reader);

    let mut buf = vec![];
    let mut total = 0;

    for _ in 0..num_delimiters {
        match reader.read_until(delimiter, &mut buf) {
            Ok(0) => break,  // EOF reached
            Ok(n) => {
                total += n;
                buf.clear();
            }
            Err(e) => return Err(e),
        }
    }
    
    Ok(total)
}

/// Iterate over bytes in the file, in reverse, until we find the
/// `num_delimiters` instance of `delimiter`. The `file` is left seek'd to the
/// position just after that delimiter.
/// Iterate over bytes in the file, in reverse, until we find the
/// `num_delimiters` instance of `delimiter`. The `file` is left seek'd to the
/// position just after that delimiter.
/// Iterate over bytes in the file, in reverse, until we find the
/// `num_delimiters` instance of `delimiter`. The `file` is left seek'd to the
/// position just after that delimiter.
/// Iterate over bytes in the file, in reverse, until we find the
/// `num_delimiters` instance of `delimiter`. The `file` is left seek'd to the
/// position just after that delimiter.
fn tail_backwards_thru_file(file: &mut File, num_delimiters: u64, delimiter: u8) {
    // This variable counts the number of delimiters found in the file
    // so far (reading from the end of the file toward the beginning).
    let mut counter = 0;

    for (block_idx, slice) in TailReverseChunks::new(file).enumerate() {
        // Iterate over each byte in the slice in reverse order.
        let mut iter = slice.iter().enumerate().rev();

        // Ignore a trailing newline in the last block, if there is one.
        if block_idx == 0 {
            if let Some(c) = slice.last() {
                if *c == delimiter {
                    iter.next();
                }
            }
        }

        // For each byte, increment the count of the number of
        // delimiters found. If we have found more than the specified
        // number of delimiters, terminate the search and seek to the
        // appropriate location in the file.
        for (i, ch) in iter {
            if *ch == delimiter {
                counter += 1;
                if counter >= num_delimiters {
                    // After each iteration of the outer loop, the
                    // cursor in the file is at the *beginning* of the
                    // block, so seeking forward by `i + 1` bytes puts
                    // us right after the found delimiter.
                    file.seek(SeekFrom::Current((i + 1) as i64)).unwrap();
                    return;
                }
            }
        }
    }
}

/// When tail'ing a file, we do not need to read the whole file from start to
/// finish just to find the last n lines or bytes. Instead, we can seek to the
/// end of the file, and then read the file "backwards" in blocks of size
/// `BLOCK_SIZE` until we find the location of the first line/byte. This ends up
/// being a nice performance win for very large files.
fn tail_bounded(file: &mut File, options: &TailOptions) -> CTResult<()> {
    let stdout = stdout();
    let mut writer = BufWriter::new(stdout.lock());

    match &options.mode {
        TailFilterMode::Lines(signum, sep) => {
            handle_bounded_lines(file, signum, *sep, &mut writer)?;
        }
        TailFilterMode::Bytes(signum) => {
            handle_bounded_bytes(file, signum, &mut writer)?;
        }
    }

    Ok(())
}

fn handle_bounded_lines(
    file: &mut File,
    signum: &TailSignum,
    separator: u8,
    writer: &mut BufWriter<StdoutLock>,
) -> CTResult<()> {
    match signum {
        TailSignum::Negative(count) => {
            tail_backwards_thru_file(file, *count, separator);
            io::copy(file, writer)?;
        }
        TailSignum::Positive(count) => {
            let skip_bytes = tail_forwards_thru_file(file, count - 1, separator)?;
            file.seek(SeekFrom::Start(skip_bytes as u64))?;
            io::copy(file, writer)?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_bounded_bytes(
    file: &mut File,
    signum: &TailSignum,
    writer: &mut BufWriter<StdoutLock>,
) -> CTResult<()> {
    match signum {
        TailSignum::Negative(count) => {
            file.seek(SeekFrom::End(-(*count as i64)))?;
            io::copy(file, writer)?;
        }
        TailSignum::Positive(count) => {
            file.seek(SeekFrom::Start(*count - 1))?;
            io::copy(file, writer)?;
        }
        _ => {}
    }
    Ok(())
}

fn tail_unbounded<T: Read>(reader: &mut BufReader<T>, options: &TailOptions) -> CTResult<()> {
    let stdout = stdout();
    let mut writer = BufWriter::new(stdout.lock());
    match &options.mode {
        TailFilterMode::Lines(TailSignum::Negative(count), sep) => {
            let mut chunks = chunks::TailLinesChunkBuffer::new(*sep, *count);
            chunks.fill(reader)?;
            chunks.print(&mut writer)?;
        }
        TailFilterMode::Lines(TailSignum::PlusZero | TailSignum::Positive(1), _) => {
            io::copy(reader, &mut writer)?;
        }
        TailFilterMode::Lines(TailSignum::Positive(count), sep) => {
            let mut num_skip = *count - 1;
            let mut chunk = chunks::TailLinesChunk::new(*sep);
            while chunk.fill(reader)?.is_some() {
                let lines = chunk.get_lines() as u64;
                if lines < num_skip {
                    num_skip -= lines;
                } else {
                    break;
                }
            }
            if chunk.has_data() {
                chunk.print_lines(&mut writer, num_skip as usize)?;
                io::copy(reader, &mut writer)?;
            }
        }
        TailFilterMode::Bytes(TailSignum::Negative(count)) => {
            let mut chunks = chunks::TailBytesChunkBuffer::new(*count);
            chunks.fill(reader)?;
            chunks.print(&mut writer)?;
        }
        TailFilterMode::Bytes(TailSignum::PlusZero | TailSignum::Positive(1)) => {
            io::copy(reader, &mut writer)?;
        }
        TailFilterMode::Bytes(TailSignum::Positive(count)) => {
            let mut num_skip = *count - 1;
            let mut chunk = chunks::TailBytesChunk::new();
            loop {
                if let Some(bytes) = chunk.fill(reader)? {
                    let bytes: u64 = bytes as u64;
                    match bytes.cmp(&num_skip) {
                        Ordering::Less => num_skip -= bytes,
                        Ordering::Equal => {
                            break;
                        }
                        Ordering::Greater => {
                            writer.write_all(chunk.get_buffer_with(num_skip as usize))?;
                            break;
                        }
                    }
                } else {
                    return Ok(());
                }
            }

            io::copy(reader, &mut writer)?;
        }
        _ => {}
    }
    Ok(())
}



