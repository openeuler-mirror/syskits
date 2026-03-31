/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

// spell-checker:ignore fname, ftype, tname, fpath, specfile, testfile, unspec, ifile, ofile, outfile, fullblock, urand, fileio, atoe, atoibm, behaviour, bmax, bremain, cflags, creat, ctable, ctty, datastructures, doesnt, etoa, fileout, fname, gnudd, iconvflags, iseek, nocache, noctty, noerror, nofollow, nolinks, nonblock, oconvflags, oseek, outfile, parseargs, rlen, rmax, rremain, rsofar, rstat, sigusr, wlen, wstat seekable oconv canonicalized fadvise Fadvise FADV DONTNEED ESPIPE bufferedoutput, SETFL

mod blocks;
mod bufferedoutput;
mod conversion_tables;
mod datastructures;
mod numbers;
mod parseargs;
mod progress;

use crate::bufferedoutput::DDBufferedOutput;
use blocks::conv_block_unblock_helper;
use ctcore::ct_io::CtOwnedFileDescriptorOrHandle;
use datastructures::*;
#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::fcntl::FcntlArg::F_SETFL;
#[cfg(any(target_os = "linux", target_os = "android"))]
use nix::fcntl::OFlag;
use parseargs::Parser;
use progress::{gen_prog_updater, ProgUpdate, ReadStat, StatusLevel, WriteStat};

use std::cmp;
use std::env;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Stdout, Write};
#[cfg(any(target_os = "linux", target_os = "android"))]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::{
    fs::FileTypeExt,
    io::{AsRawFd, FromRawFd},
};
#[cfg(windows)]
use std::os::windows::{fs::MetadataExt, io::AsHandle};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering::Relaxed},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use clap::{crate_version, Arg, Command};
use ctcore::ct_display::Quotable;
#[cfg(unix)]
use ctcore::ct_error::set_ct_exit_code;
use ctcore::ct_error::{CTResult, FromIo};
#[cfg(target_os = "linux")]
use ctcore::ct_show_if_err;
use ctcore::{ct_format_usage, ct_help_about, ct_help_section, ct_help_usage, ct_show_error};
use gcd::Gcd;
#[cfg(target_os = "linux")]
use nix::{
    errno::Errno,
    fcntl::{posix_fadvise, PosixFadviseAdvice},
};

const DD_ABOUT: &str = ct_help_about!("dd.md");
const DD_AFTER_HELP: &str = ct_help_section!("after help", "dd.md");
const DD_USAGE: &str = ct_help_usage!("dd.md");
const BUF_INIT_BYTE: u8 = 0xDD;

/// Final settings after parsing
#[derive(Default)]
struct DdOptions {
    infile: Option<String>,
    outfile: Option<String>,
    ibs: usize,
    obs: usize,
    skip: u64,
    seek: u64,
    count: Option<Num>,
    iconv: IConvFlags,
    iflags: IFlags,
    oconv: OConvFlags,
    oflags: OFlags,
    status: Option<StatusLevel>,
    /// Whether the output writer should buffer partial blocks until complete.
    buffered: bool,
}

/// A timer which triggers on a given interval
///
/// After being constructed with [`Alarm::with_interval`], [`Alarm::is_triggered`]
/// will return true once per the given [`Duration`].
///
/// Can be cloned, but the trigger status is shared across all instances so only
/// the first caller each interval will yield true.
///
/// When all instances are dropped the background thread will exit on the next interval.
#[derive(Debug, Clone)]
pub struct Alarm {
    interval: Duration,
    trigger: Arc<AtomicBool>,
}

impl Alarm {
    pub fn with_interval(interval: Duration) -> Self {
        let trigger = Arc::new(AtomicBool::default());

        let weak_trigger = Arc::downgrade(&trigger);
        thread::spawn(move || {
            while let Some(trigger) = weak_trigger.upgrade() {
                thread::sleep(interval);
                trigger.store(true, Relaxed);
            }
        });

        Self { interval, trigger }
    }

    pub fn is_triggered(&self) -> bool {
        self.trigger.swap(false, Relaxed)
    }

    pub fn get_interval(&self) -> Duration {
        self.interval
    }
}

/// A number in blocks or bytes
///
/// Some values (seek, skip, iseek, oseek) can have values either in blocks or in bytes.
/// We need to remember this because the size of the blocks (ibs) is only known after parsing
/// all the arguments.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Num {
    Blocks(u64),
    Bytes(u64),
}

impl Default for Num {
    fn default() -> Self {
        Self::Blocks(0)
    }
}

impl Num {
    fn force_bytes_if(self, force: bool) -> Self {
        match self {
            Self::Blocks(n) if force => Self::Bytes(n),
            count => count,
        }
    }

    fn to_bytes(self, block_size: u64) -> u64 {
        match self {
            Self::Blocks(n) => n * block_size,
            Self::Bytes(n) => n,
        }
    }
}

/// Data sources.
///
/// Use [`Source::stdin_as_file`] if available to enable more
/// fine-grained access to reading from stdin.
enum Source {
    /// Input from stdin.
    #[cfg(not(unix))]
    Stdin(io::Stdin),

    /// Input from a file.
    File(File),

    /// Input from stdin, opened from its file descriptor.
    #[cfg(unix)]
    StdinFile(File),

    /// Input from a named pipe, also known as a FIFO.
    #[cfg(unix)]
    Fifo(File),
}

impl Source {
    /// Create a source from stdin using its raw file descriptor.
    ///
    /// This returns an instance of the `Source::StdinFile` variant,
    /// using the raw file descriptor of [`std::io::Stdin`] to create
    /// the [`std::fs::File`] parameter. You can use this instead of
    /// `Source::Stdin` to allow reading from stdin without consuming
    /// the entire contents of stdin when this process terminates.
    #[cfg(unix)]
    fn stdin_as_file() -> Self {
        let fd = io::stdin().as_raw_fd();
        let f = unsafe { File::from_raw_fd(fd) };
        Self::StdinFile(f)
    }

    /// The length of the data source in number of bytes.
    ///
    /// If it cannot be determined, then this function returns 0.
    fn len(&self) -> std::io::Result<i64> {
        match self {
            Self::File(f) => Ok(f.metadata()?.len().try_into().unwrap_or(i64::MAX)),
            _ => Ok(0),
        }
    }

    fn skip(&mut self, n: u64) -> io::Result<u64> {
        match self {
            #[cfg(not(unix))]
            Self::Stdin(stdin) => match io::copy(&mut stdin.take(n), &mut io::sink()) {
                Ok(m) if m < n => {
                    ct_show_error!("'standard input': cannot skip to specified offset");
                    Ok(m)
                }
                Ok(m) => Ok(m),
                Err(e) => Err(e),
            },
            #[cfg(unix)]
            Self::StdinFile(f) => {
                if let Ok(Some(len)) = try_get_len_of_block_device(f) {
                    if len < n {
                        // GNU compatibility:
                        // this case prints the stats but sets the exit code to 1
                        ct_show_error!("'standard input': cannot skip: Invalid argument");
                        set_ct_exit_code(1);
                        return Ok(len);
                    }
                }
                match io::copy(&mut f.take(n), &mut io::sink()) {
                    Ok(m) if m < n => {
                        ct_show_error!("'standard input': cannot skip to specified offset");
                        Ok(m)
                    }
                    Ok(m) => Ok(m),
                    Err(e) => Err(e),
                }
            }
            Self::File(f) => f.seek(io::SeekFrom::Current(n.try_into().unwrap())),
            #[cfg(unix)]
            Self::Fifo(f) => io::copy(&mut f.take(n), &mut io::sink()),
        }
    }

    /// Discard the system file cache for the given portion of the data source.
    ///
    /// `offset` and `len` specify a contiguous portion of the data
    /// source. This function informs the kernel that the specified
    /// portion of the source is no longer needed. If not possible,
    /// then this function returns an error.
    #[cfg(target_os = "linux")]
    fn discard_cache(&self, offset: libc::off_t, len: libc::off_t) -> nix::Result<()> {
        match self {
            Self::File(f) => {
                let advice = PosixFadviseAdvice::POSIX_FADV_DONTNEED;
                posix_fadvise(f.as_raw_fd(), offset, len, advice)
            }
            _ => Err(Errno::ESPIPE), // "Illegal seek"
        }
    }
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(not(unix))]
            Self::Stdin(stdin) => stdin.read(buf),
            Self::File(f) => f.read(buf),
            #[cfg(unix)]
            Self::StdinFile(f) => f.read(buf),
            #[cfg(unix)]
            Self::Fifo(f) => f.read(buf),
        }
    }
}

/// The source of the data, configured with the given settings.
///
/// Use the [`Input::new_stdin`] or [`Input::new_file`] functions to
/// construct a new instance of this struct. Then pass the instance to
/// the [`dd_copy`] function to execute the main copy operation
/// for `dd`.
struct Input<'a> {
    /// The source from which bytes will be read.
    src: Source,

    /// Configuration settings for how to read the data.
    settings: &'a DdOptions,
}

impl<'a> Input<'a> {
    /// Instantiate this struct with stdin as a source.
    fn new_stdin(settings: &'a DdOptions) -> CTResult<Self> {
        #[cfg(unix)]
        let mut src = Source::stdin_as_file();
        #[cfg(unix)]
        if let Source::StdinFile(f) = &src {
            // GNU compatibility:
            // this will check whether stdin points to a folder or not
            if f.metadata()?.is_file() && settings.iflags.directory {
                ct_show_error!("standard input: not a directory");
                return Err(1.into());
            }
        };
        if settings.skip > 0 {
            src.skip(settings.skip)?;
        }
        Ok(Self { src, settings })
    }

    /// Instantiate this struct with the named file as a source.
    fn new_file(filename: &Path, settings: &'a DdOptions) -> CTResult<Self> {
        let src = {
            let mut opts = OpenOptions::new();
            opts.read(true);

            #[cfg(any(target_os = "linux", target_os = "android"))]
            if let Some(libc_flags) = make_linux_iflags(&settings.iflags) {
                opts.custom_flags(libc_flags);
            }

            opts.open(filename)
                .map_err_context(|| format!("failed to open {}", filename.quote()))?
        };

        let mut src = Source::File(src);
        if settings.skip > 0 {
            src.skip(settings.skip)?;
        }
        Ok(Self { src, settings })
    }

    /// Instantiate this struct with the named pipe as a source.
    #[cfg(unix)]
    fn new_fifo(filename: &Path, settings: &'a DdOptions) -> CTResult<Self> {
        let mut opts = OpenOptions::new();
        opts.read(true);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        opts.custom_flags(make_linux_iflags(&settings.iflags).unwrap_or(0));
        let mut src = Source::Fifo(opts.open(filename)?);
        if settings.skip > 0 {
            src.skip(settings.skip)?;
        }
        Ok(Self { src, settings })
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn make_linux_iflags(iflags: &IFlags) -> Option<libc::c_int> {
    let mut flag = 0;

    if iflags.direct {
        flag |= libc::O_DIRECT;
    }
    if iflags.directory {
        flag |= libc::O_DIRECTORY;
    }
    if iflags.dsync {
        flag |= libc::O_DSYNC;
    }
    if iflags.noatime {
        flag |= libc::O_NOATIME;
    }
    if iflags.noctty {
        flag |= libc::O_NOCTTY;
    }
    if iflags.nofollow {
        flag |= libc::O_NOFOLLOW;
    }
    if iflags.nonblock {
        flag |= libc::O_NONBLOCK;
    }
    if iflags.sync {
        flag |= libc::O_SYNC;
    }

    if flag == 0 {
        None
    } else {
        Some(flag)
    }
}

impl<'a> Read for Input<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut base_idx = 0;
        let target_len = buf.len();
        loop {
            match self.src.read(&mut buf[base_idx..]) {
                Ok(0) => return Ok(base_idx),
                Ok(rlen) if self.settings.iflags.fullblock => {
                    base_idx += rlen;

                    if base_idx >= target_len {
                        return Ok(target_len);
                    }
                }
                Ok(len) => return Ok(len),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) if self.settings.iconv.noerror => return Ok(base_idx),
                Err(e) => return Err(e),
            }
        }
    }
}

impl<'a> Input<'a> {
    /// Discard the system file cache for the given portion of the input.
    ///
    /// `offset` and `len` specify a contiguous portion of the input.
    /// This function informs the kernel that the specified portion of
    /// the input file is no longer needed. If not possible, then this
    /// function prints an error message to stderr and sets the exit
    /// status code to 1.
    #[allow(unused_variables)]
    fn discard_cache(&self, offset: libc::off_t, len: libc::off_t) {
        #[cfg(target_os = "linux")]
        {
            ct_show_if_err!(self
                .src
                .discard_cache(offset, len)
                .map_err_context(|| "failed to discard cache for: 'standard input'".to_string()));
        }
        #[cfg(not(target_os = "linux"))]
        {
            // TODO Is there a way to discard filesystem cache on
            // these other operating systems?
        }
    }

    /// Fills a given buffer.
    /// Reads in increments of 'self.ibs'.
    /// The start of each ibs-sized read follows the previous one.
    fn fill_consecutive(&mut self, buf: &mut Vec<u8>) -> std::io::Result<ReadStat> {
        let mut reads_complete = 0;
        let mut reads_partial = 0;
        let mut bytes_total = 0;

        for chunk in buf.chunks_mut(self.settings.ibs) {
            match self.read(chunk)? {
                rlen if rlen == self.settings.ibs => {
                    bytes_total += rlen;
                    reads_complete += 1;
                }
                rlen if rlen > 0 => {
                    bytes_total += rlen;
                    reads_partial += 1;
                }
                _ => break,
            }
        }
        buf.truncate(bytes_total);
        Ok(ReadStat {
            reads_complete,
            reads_partial,
            // Records are not truncated when filling.
            records_truncated: 0,
            bytes_total: bytes_total.try_into().unwrap(),
        })
    }

    /// Fills a given buffer.
    /// Reads in increments of 'self.ibs'.
    /// The start of each ibs-sized read is aligned to multiples of ibs; remaining space is filled with the 'pad' byte.
    fn fill_blocks(&mut self, buf: &mut Vec<u8>, pad: u8) -> std::io::Result<ReadStat> {
        let mut reads_complete = 0;
        let mut reads_partial = 0;
        let mut base_idx = 0;
        let mut bytes_total = 0;

        while base_idx < buf.len() {
            let next_blk = cmp::min(base_idx + self.settings.ibs, buf.len());
            let target_len = next_blk - base_idx;

            match self.read(&mut buf[base_idx..next_blk])? {
                0 => break,
                rlen if rlen < target_len => {
                    bytes_total += rlen;
                    reads_partial += 1;
                    let padding = vec![pad; target_len - rlen];
                    buf.splice(base_idx + rlen..next_blk, padding.into_iter());
                }
                rlen => {
                    bytes_total += rlen;
                    reads_complete += 1;
                }
            }

            base_idx += self.settings.ibs;
        }

        buf.truncate(base_idx);
        Ok(ReadStat {
            reads_complete,
            reads_partial,
            records_truncated: 0,
            bytes_total: bytes_total.try_into().unwrap(),
        })
    }
}

enum Density {
    Sparse,
    Dense,
}

/// Data destinations.
enum Dest {
    /// Output to stdout.
    Stdout(Stdout),

    /// Output to a file.
    ///
    /// The [`Density`] component indicates whether to attempt to
    /// write a sparse file when all-zero blocks are encountered.
    File(File, Density),

    /// Output to a named pipe, also known as a FIFO.
    #[cfg(unix)]
    Fifo(File),

    /// Output to nothing, dropping each byte written to the output.
    #[cfg(unix)]
    Sink,
}

impl Dest {
    fn fsync(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            Self::File(f, _) => {
                f.flush()?;
                f.sync_all()
            }
            #[cfg(unix)]
            Self::Fifo(f) => {
                f.flush()?;
                f.sync_all()
            }
            #[cfg(unix)]
            Self::Sink => Ok(()),
        }
    }

    fn fdatasync(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            Self::File(f, _) => {
                f.flush()?;
                f.sync_data()
            }
            #[cfg(unix)]
            Self::Fifo(f) => {
                f.flush()?;
                f.sync_data()
            }
            #[cfg(unix)]
            Self::Sink => Ok(()),
        }
    }

    fn seek(&mut self, n: u64) -> io::Result<u64> {
        match self {
            Self::Stdout(stdout) => io::copy(&mut io::repeat(0).take(n), stdout),
            Self::File(f, _) => {
                #[cfg(unix)]
                if let Ok(Some(len)) = try_get_len_of_block_device(f) {
                    if len < n {
                        // GNU compatibility:
                        // this case prints the stats but sets the exit code to 1
                        ct_show_error!("'standard output': cannot seek: Invalid argument");
                        set_ct_exit_code(1);
                        return Ok(len);
                    }
                }
                f.seek(io::SeekFrom::Current(n.try_into().unwrap()))
            }
            #[cfg(unix)]
            Self::Fifo(f) => {
                // Seeking in a named pipe means *reading* from the pipe.
                io::copy(&mut f.take(n), &mut io::sink())
            }
            #[cfg(unix)]
            Self::Sink => Ok(0),
        }
    }

    /// Truncate the underlying file to the current stream position, if possible.
    fn truncate(&mut self) -> io::Result<()> {
        match self {
            Self::File(f, _) => {
                let pos = f.stream_position()?;
                f.set_len(pos)
            }
            _ => Ok(()),
        }
    }

    /// Discard the system file cache for the given portion of the destination.
    ///
    /// `offset` and `len` specify a contiguous portion of the
    /// destination. This function informs the kernel that the
    /// specified portion of the destination is no longer needed. If
    /// not possible, then this function returns an error.
    #[cfg(target_os = "linux")]
    fn discard_cache(&self, offset: libc::off_t, len: libc::off_t) -> nix::Result<()> {
        match self {
            Self::File(f, _) => {
                let advice = PosixFadviseAdvice::POSIX_FADV_DONTNEED;
                posix_fadvise(f.as_raw_fd(), offset, len, advice)
            }
            _ => Err(Errno::ESPIPE), // "Illegal seek"
        }
    }

    /// The length of the data destination in number of bytes.
    ///
    /// If it cannot be determined, then this function returns 0.
    fn len(&self) -> std::io::Result<i64> {
        match self {
            Self::File(f, _) => Ok(f.metadata()?.len().try_into().unwrap_or(i64::MAX)),
            _ => Ok(0),
        }
    }
}

/// Decide whether the given buffer is all zeros.
fn is_sparse(buf: &[u8]) -> bool {
    buf.iter().all(|&e| e == 0u8)
}

impl Write for Dest {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::File(f, Density::Sparse) if is_sparse(buf) => {
                let seek_amt: i64 = buf
                    .len()
                    .try_into()
                    .expect("Internal dd Error: Seek amount greater than signed 64-bit integer");
                f.seek(io::SeekFrom::Current(seek_amt))?;
                Ok(buf.len())
            }
            Self::File(f, _) => f.write(buf),
            Self::Stdout(stdout) => stdout.write(buf),
            #[cfg(unix)]
            Self::Fifo(f) => f.write(buf),
            #[cfg(unix)]
            Self::Sink => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Stdout(stdout) => stdout.flush(),
            Self::File(f, _) => f.flush(),
            #[cfg(unix)]
            Self::Fifo(f) => f.flush(),
            #[cfg(unix)]
            Self::Sink => Ok(()),
        }
    }
}

/// The destination of the data, configured with the given settings.
///
/// Use the [`Output::new_stdout`] or [`Output::new_file`] functions
/// to construct a new instance of this struct. Then use the
/// [`dd_copy`] function to execute the main copy operation for
/// `dd`.
struct DdOutput<'a> {
    /// The destination to which bytes will be written.
    dst: Dest,

    /// Configuration settings for how to read and write the data.
    settings: &'a DdOptions,
}

impl<'a> DdOutput<'a> {
    /// Instantiate this struct with stdout as a destination.
    fn new_stdout(settings: &'a DdOptions) -> CTResult<Self> {
        let mut dst = Dest::Stdout(io::stdout());
        dst.seek(settings.seek)
            .map_err_context(|| "write error".to_string())?;
        Ok(Self { dst, settings })
    }

    /// Instantiate this struct with the named file as a destination.
    fn new_file(filename: &Path, settings: &'a DdOptions) -> CTResult<Self> {
        fn open_dst(path: &Path, cflags: &OConvFlags, oflags: &OFlags) -> Result<File, io::Error> {
            let mut opts = OpenOptions::new();
            opts.write(true)
                .create(!cflags.nocreat)
                .create_new(cflags.excl)
                .append(oflags.append);

            #[cfg(any(target_os = "linux", target_os = "android"))]
            if let Some(libc_flags) = make_linux_oflags(oflags) {
                opts.custom_flags(libc_flags);
            }

            opts.open(path)
        }

        let dst = open_dst(filename, &settings.oconv, &settings.oflags)
            .map_err_context(|| format!("failed to open {}", filename.quote()))?;

        // Seek to the index in the output file, truncating if requested.
        //
        // Calling `set_len()` may result in an error (for example,
        // when calling it on `/dev/null`), but we don't want to
        // terminate the process when that happens.  Instead, we
        // suppress the error by calling `Result::ok()`. This matches
        // the behavior of GNU `dd` when given the command-line
        // argument `of=/dev/null`.
        if !settings.oconv.notrunc {
            dst.set_len(settings.seek).ok();
        }

        Self::prepare_file(dst, settings)
    }

    fn prepare_file(dst: File, settings: &'a DdOptions) -> CTResult<Self> {
        let density = if settings.oconv.sparse {
            Density::Sparse
        } else {
            Density::Dense
        };
        let mut dst = Dest::File(dst, density);
        dst.seek(settings.seek)
            .map_err_context(|| "failed to seek in output file".to_string())?;
        Ok(Self { dst, settings })
    }

    /// Instantiate this struct with file descriptor as a destination.
    ///
    /// This is useful e.g. for the case when the file descriptor was
    /// already opened by the system (stdout) and has a state
    /// (current position) that shall be used.
    fn new_file_from_stdout(settings: &'a DdOptions) -> CTResult<Self> {
        let fx = CtOwnedFileDescriptorOrHandle::from(io::stdout())?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if let Some(libc_flags) = make_linux_oflags(&settings.oflags) {
            nix::fcntl::fcntl(
                fx.as_raw().as_raw_fd(),
                F_SETFL(OFlag::from_bits_retain(libc_flags)),
            )?;
        }

        Self::prepare_file(fx.into_file(), settings)
    }

    /// Instantiate this struct with the given named pipe as a destination.
    #[cfg(unix)]
    fn new_fifo(filename: &Path, settings: &'a DdOptions) -> CTResult<Self> {
        // We simulate seeking in a FIFO by *reading*, so we open the
        // file for reading. But then we need to close the file and
        // re-open it for writing.
        if settings.seek > 0 {
            Dest::Fifo(File::open(filename)?).seek(settings.seek)?;
        }
        // If `count=0`, then we don't bother opening the file for
        // writing because that would cause this process to block
        // indefinitely.
        if let Some(Num::Blocks(0) | Num::Bytes(0)) = settings.count {
            let dst = Dest::Sink;
            return Ok(Self { dst, settings });
        }
        // At this point, we know there is at least one block to write
        // to the output, so we open the file for writing.
        let mut opts = OpenOptions::new();
        opts.write(true)
            .create(!settings.oconv.nocreat)
            .create_new(settings.oconv.excl)
            .append(settings.oflags.append);
        #[cfg(any(target_os = "linux", target_os = "android"))]
        opts.custom_flags(make_linux_oflags(&settings.oflags).unwrap_or(0));
        let dst = Dest::Fifo(opts.open(filename)?);
        Ok(Self { dst, settings })
    }

    /// Discard the system file cache for the given portion of the output.
    ///
    /// `offset` and `len` specify a contiguous portion of the output.
    /// This function informs the kernel that the specified portion of
    /// the output file is no longer needed. If not possible, then
    /// this function prints an error message to stderr and sets the
    /// exit status code to 1.
    #[allow(unused_variables)]
    fn discard_cache(&self, offset: libc::off_t, len: libc::off_t) {
        #[cfg(target_os = "linux")]
        {
            ct_show_if_err!(self
                .dst
                .discard_cache(offset, len)
                .map_err_context(|| "failed to discard cache for: 'standard output'".to_string()));
        }
        #[cfg(target_os = "linux")]
        {
            // TODO Is there a way to discard filesystem cache on
            // these other operating systems?
        }
    }

    /// Write the given bytes one block at a time.
    ///
    /// This may write partial blocks (for example, if the underlying
    /// call to [`Write::write`] writes fewer than `buf.len()`
    /// bytes). The returned [`WriteStat`] object will include the
    /// number of partial and complete blocks written during execution
    /// of this function.
    fn write_blocks(&mut self, buf: &[u8]) -> io::Result<WriteStat> {
        let mut writes_complete = 0;
        let mut writes_partial = 0;
        let mut bytes_total = 0;

        for chunk in buf.chunks(self.settings.obs) {
            let wlen = self.dst.write(chunk)?;
            if wlen < self.settings.obs {
                writes_partial += 1;
            } else {
                writes_complete += 1;
            }
            bytes_total += wlen;
        }

        Ok(WriteStat {
            writes_complete,
            writes_partial,
            bytes_total: bytes_total.try_into().unwrap_or(0u128),
        })
    }

    /// Flush the output to disk, if configured to do so.
    fn sync(&mut self) -> std::io::Result<()> {
        if self.settings.oconv.fsync {
            self.dst.fsync()
        } else if self.settings.oconv.fdatasync {
            self.dst.fdatasync()
        } else {
            // Intentionally do nothing in this case.
            Ok(())
        }
    }

    /// Truncate the underlying file to the current stream position, if possible.
    fn truncate(&mut self) -> std::io::Result<()> {
        self.dst.truncate()
    }
}

/// The block writer either with or without partial block buffering.
enum BlockWriter<'a> {
    /// Block writer with partial block buffering.
    ///
    /// Partial blocks are buffered until completed.
    Buffered(DDBufferedOutput<'a>),

    /// Block writer without partial block buffering.
    ///
    /// Partial blocks are written immediately.
    Unbuffered(DdOutput<'a>),
}

impl<'a> BlockWriter<'a> {
    fn discard_cache(&self, offset: libc::off_t, len: libc::off_t) {
        match self {
            Self::Unbuffered(o) => o.discard_cache(offset, len),
            Self::Buffered(o) => o.discard_cache(offset, len),
        }
    }

    fn flush(&mut self) -> io::Result<WriteStat> {
        match self {
            Self::Unbuffered(_) => Ok(WriteStat::default()),
            Self::Buffered(o) => o.flush(),
        }
    }

    fn sync(&mut self) -> io::Result<()> {
        match self {
            Self::Unbuffered(o) => o.sync(),
            Self::Buffered(o) => o.sync(),
        }
    }

    /// Truncate the file to the final cursor location.
    fn truncate(&mut self) {
        // Calling `set_len()` may result in an error (for example,
        // when calling it on `/dev/null`), but we don't want to
        // terminate the process when that happens. Instead, we
        // suppress the error by calling `Result::ok()`. This matches
        // the behavior of GNU `dd` when given the command-line
        // argument `of=/dev/null`.
        match self {
            Self::Unbuffered(o) => o.truncate().ok(),
            Self::Buffered(o) => o.truncate().ok(),
        };
    }

    fn write_blocks(&mut self, buf: &[u8]) -> std::io::Result<WriteStat> {
        match self {
            Self::Unbuffered(o) => o.write_blocks(buf),
            Self::Buffered(o) => o.dd_write_blocks(buf),
        }
    }
}

/// Copy data from input to output with dd functionality
fn dd_copy(mut input: Input, output: DdOutput) -> std::io::Result<()> {
    // 初始化复制环境
    let (mut state, mut output) = initialize_copy_environment(input, output);

    // 执行主复制循环
    perform_copy_loop(&mut state, &mut output)?;

    // 完成复制操作
    finalize_copy::<BlockWriter>(output, state)
}

/// 复制操作的状态
struct CopyState<'a> {
    input: Input<'a>,
    buffer: Vec<u8>,
    read_stat: ReadStat,
    write_stat: WriteStat,
    start_time: Instant,
    duration: Duration,
    prog_tx: mpsc::Sender<ProgUpdate>,
    output_thread: thread::JoinHandle<()>,
    read_offset: u64,
    write_offset: u128,
    alarm: Alarm,
}

/// 初始化复制环境
fn initialize_copy_environment<'a>(input: Input<'a>, output: DdOutput<'a>) -> (CopyState<'a>, BlockWriter<'a>) {
    let bsize = calc_bsize(input.settings.ibs, output.settings.obs);
    let buffer = vec![BUF_INIT_BYTE; bsize];
    let read_stat = ReadStat::default();
    let write_stat = WriteStat::default();
    let start_time = Instant::now();

    // 设置进度报告
    let (prog_tx, rx) = mpsc::channel();
    let output_thread = thread::spawn(gen_prog_updater(rx, input.settings.status));
    let alarm = Alarm::with_interval(Duration::from_secs(1));

    // 创建输出写入器
    let output = if output.settings.buffered {
        BlockWriter::Buffered(DDBufferedOutput::new(output))
    } else {
        BlockWriter::Unbuffered(output)
    };

    let state = CopyState {
        input,
        buffer,
        read_stat,
        write_stat,
        start_time,
        duration: Duration::ZERO,
        prog_tx,
        output_thread,
        read_offset: 0,
        write_offset: 0,
        alarm,
    };

    (state, output)
}

/// 执行主复制循环
fn perform_copy_loop(state: &mut CopyState, output: &mut BlockWriter) -> std::io::Result<()> {
    while below_count_limit(&state.input.settings.count, &state.read_stat) {
        // 读取数据块
        if !read_and_process_block(state, output)? {
            break;
        }

        // 更新进度
        update_progress(state);
    }
    Ok(())
}

/// 读取并处理一个数据块
fn read_and_process_block(state: &mut CopyState, output: &mut BlockWriter) -> std::io::Result<bool> {
    // 计算本次读取的缓冲区大小
    let loop_bsize = calc_loop_bsize(
        &state.input.settings.count,
        &state.read_stat,
        &state.write_stat,
        state.input.settings.ibs,
        state.buffer.len(),
    );

    state.start_time = Instant::now();
    // 读取数据
    let rstat_update = read_helper(&mut state.input, &mut state.buffer, loop_bsize)?;
    if rstat_update.is_empty() {
        return Ok(false);
    }

    // 写入数据
    let wstat_update = output.write_blocks(&state.buffer)?;

    // 处理缓存
    handle_cache_updates(state, &rstat_update, &wstat_update, output)?;

    //累加用时
    state.duration = state.duration + state.start_time.elapsed();

    // 更新统计信息
    state.read_stat += rstat_update;
    state.write_stat += wstat_update;
    state.read_offset += rstat_update.bytes_total;
    state.write_offset += wstat_update.bytes_total;

    Ok(true)
}

/// 处理缓存更新
fn handle_cache_updates(
    state: &mut CopyState,
    rstat_update: &ReadStat,
    wstat_update: &WriteStat,
    output: &mut BlockWriter,
) -> std::io::Result<()> {
    // 处理输入缓存
    if state.input.settings.iflags.nocache {
        let offset = (state.read_offset as i64).try_into().unwrap();
        let len = (rstat_update.bytes_total as i64).try_into().unwrap();
        state.input.discard_cache(offset, len);
    }

    // 处理输出缓存
    if state.input.settings.oflags.nocache {
        let offset = (state.write_offset as i64).try_into().unwrap();
        let len = (wstat_update.bytes_total as i64).try_into().unwrap();
        output.discard_cache(offset, len);
    }

    Ok(())
}

/// 更新进度信息
fn update_progress(state: &mut CopyState) {
    if state.alarm.is_triggered() {
        let prog_update = ProgUpdate::new(
            state.read_stat,
            state.write_stat,
            state.duration,
            false,
        );
        state.prog_tx.send(prog_update).unwrap_or(());
    }
}

/// 完成复制操作
fn finalize_copy<T>(mut output: BlockWriter, state: CopyState) -> std::io::Result<()> {
    let mut dur = state.duration;
    let start_time = Instant::now();
    
    // 刷新输出缓冲
    let wstat_update = output.flush()?;
    output.sync()?;

    // 如果需要，截断文件
    if !state.input.settings.oconv.notrunc {
        output.truncate();
    }

    dur = dur + start_time.elapsed();
    // 发送最终统计信息
    let final_wstat = state.write_stat + wstat_update;
    let prog_update = ProgUpdate::new(state.read_stat, final_wstat, dur, true);
    state.prog_tx.send(prog_update).unwrap_or(());

    // 等待输出线程完成
    state.output_thread
        .join()
        .expect("Failed to join with the output thread.");

    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[allow(clippy::cognitive_complexity)]
fn make_linux_oflags(oflags: &OFlags) -> Option<libc::c_int> {
    let mut flag = 0;

    // oflag=FLAG
    if oflags.append {
        flag |= libc::O_APPEND;
    }
    if oflags.direct {
        flag |= libc::O_DIRECT;
    }
    if oflags.directory {
        flag |= libc::O_DIRECTORY;
    }
    if oflags.dsync {
        flag |= libc::O_DSYNC;
    }
    if oflags.noatime {
        flag |= libc::O_NOATIME;
    }
    if oflags.noctty {
        flag |= libc::O_NOCTTY;
    }
    if oflags.nofollow {
        flag |= libc::O_NOFOLLOW;
    }
    if oflags.nonblock {
        flag |= libc::O_NONBLOCK;
    }
    if oflags.sync {
        flag |= libc::O_SYNC;
    }

    if flag == 0 {
        None
    } else {
        Some(flag)
    }
}

/// Read from an input (that is, a source of bytes) into the given buffer.
///
/// This function also performs any conversions as specified by
/// `conv=swab` or `conv=block` command-line arguments. This function
/// mutates the `buf` argument in-place. The returned [`ReadStat`]
/// indicates how many blocks were read.
fn read_helper(i: &mut Input, buf: &mut Vec<u8>, bsize: usize) -> std::io::Result<ReadStat> {
    // Local Helper Fns -------------------------------------------------
    fn perform_swab(buf: &mut [u8]) {
        for base in (1..buf.len()).step_by(2) {
            buf.swap(base, base - 1);
        }
    }
    // ------------------------------------------------------------------
    // Read
    // Resize the buffer to the bsize. Any garbage data in the buffer is overwritten or truncated, so there is no need to fill with BUF_INIT_BYTE first.
    buf.resize(bsize, BUF_INIT_BYTE);

    let mut rstat = match i.settings.iconv.sync {
        Some(ch) => i.fill_blocks(buf, ch)?,
        _ => i.fill_consecutive(buf)?,
    };
    // Return early if no data
    if rstat.reads_complete == 0 && rstat.reads_partial == 0 {
        return Ok(rstat);
    }

    // Perform any conv=x[,x...] options
    if i.settings.iconv.swab {
        perform_swab(buf);
    }

    match i.settings.iconv.mode {
        Some(ref mode) => {
            *buf = conv_block_unblock_helper(buf.clone(), mode, &mut rstat);
            Ok(rstat)
        }
        None => Ok(rstat),
    }
}

// Calculate a 'good' internal buffer size.
// For performance of the read/write functions, the buffer should hold
// both an integral number of reads and an integral number of writes. For
// sane real-world memory use, it should not be too large. I believe
// the least common multiple is a good representation of these interests.
// https://en.wikipedia.org/wiki/Least_common_multiple#Using_the_greatest_common_divisor
fn calc_bsize(ibs: usize, obs: usize) -> usize {
    let gcd = Gcd::gcd(ibs, obs);
    // calculate the lcm from gcd
    (ibs / gcd) * obs
}

// Calculate the buffer size appropriate for this loop iteration, respecting
// a count=N if present.
fn calc_loop_bsize(
    count: &Option<Num>,
    rstat: &ReadStat,
    wstat: &WriteStat,
    ibs: usize,
    ideal_bsize: usize,
) -> usize {
    match count {
        Some(Num::Blocks(rmax)) => {
            let rsofar = rstat.reads_complete + rstat.reads_partial;
            let rremain = rmax - rsofar;
            cmp::min(ideal_bsize as u64, rremain * ibs as u64) as usize
        }
        Some(Num::Bytes(bmax)) => {
            let bmax: u128 = (*bmax).into();
            let bremain: u128 = bmax - wstat.bytes_total;
            cmp::min(ideal_bsize as u128, bremain) as usize
        }
        None => ideal_bsize,
    }
}

// Decide if the current progress is below a count=N limit or return
// true if no such limit is set.
fn below_count_limit(count: &Option<Num>, rstat: &ReadStat) -> bool {
    match count {
        Some(Num::Blocks(n)) => rstat.reads_complete + rstat.reads_partial < *n,
        Some(Num::Bytes(n)) => rstat.bytes_total < *n,
        None => true,
    }
}

/// Canonicalized file name of `/dev/stdout`.
///
/// For example, if this process were invoked from the command line as
/// `dd`, then this function returns the [`OsString`] form of
/// `"/dev/stdout"`. However, if this process were invoked as `dd >
/// outfile`, then this function returns the canonicalized path to
/// `outfile`, something like `"/path/to/outfile"`.
fn stdout_canonicalized() -> OsString {
    match Path::new("/dev/stdout").canonicalize() {
        Ok(p) => p.into_os_string(),
        Err(_) => OsString::from("/dev/stdout"),
    }
}

/// Decide whether stdout is being redirected to a seekable file.
///
/// For example, if this process were invoked from the command line as
///
/// ```sh
/// dd if=/dev/zero bs=1 count=10 seek=5 > /dev/sda1
/// ```
///
/// where `/dev/sda1` is a seekable block device then this function
/// would return true. If invoked as
///
/// ```sh
/// dd if=/dev/zero bs=1 count=10 seek=5
/// ```
///
/// then this function would return false.
fn is_stdout_redirected_to_seekable_file() -> bool {
    let s = stdout_canonicalized();
    let p = Path::new(&s);
    match File::open(p) {
        Ok(mut f) => {
            f.stream_position().is_ok() && f.seek(SeekFrom::End(0)).is_ok() && f.rewind().is_ok()
        }
        Err(_) => false,
    }
}

/// Try to get the len if it is a block device
#[cfg(unix)]
fn try_get_len_of_block_device(file: &mut File) -> io::Result<Option<u64>> {
    let ftype = file.metadata()?.file_type();
    if !ftype.is_block_device() {
        return Ok(None);
    }

    // FIXME: this can be replaced by file.stream_len() when stable.
    let len = file.seek(SeekFrom::End(0))?;
    file.rewind()?;
    Ok(Some(len))
}

/// Decide whether the named file is a named pipe, also known as a FIFO.
#[cfg(unix)]
fn is_fifo(filename: &str) -> bool {
    if let Ok(metadata) = std::fs::metadata(filename) {
        if metadata.file_type().is_fifo() {
            return true;
        }
    }
    false
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    dd_main(args)
}

fn dd_main(args: impl ctcore::Args) -> CTResult<()> {
    let matches = ct_app().try_get_matches_from(args)?;

    let options: DdOptions = Parser::new().parse(
        &matches
            .get_many::<String>(options::OPERANDS)
            .unwrap_or_default()
            .map(|s| s.as_ref())
            .collect::<Vec<_>>()[..],
    )?;

    let i = match options.infile {
        #[cfg(unix)]
        Some(ref infile) if is_fifo(infile) => Input::new_fifo(Path::new(&infile), &options)?,
        Some(ref infile) => Input::new_file(Path::new(&infile), &options)?,
        None => Input::new_stdin(&options)?,
    };
    let o = match options.outfile {
        #[cfg(unix)]
        Some(ref outfile) if is_fifo(outfile) => DdOutput::new_fifo(Path::new(&outfile), &options)?,
        Some(ref outfile) => DdOutput::new_file(Path::new(&outfile), &options)?,
        None if is_stdout_redirected_to_seekable_file() => DdOutput::new_file_from_stdout(&options)?,
        None => DdOutput::new_stdout(&options)?,
    };
    dd_copy(i, o).map_err_context(|| "IO error".to_string())
}

pub fn ct_app() -> Command {
    Command::new(ctcore::ct_util_name())
        .version(crate_version!())
        .about(DD_ABOUT)
        .override_usage(ct_format_usage(DD_USAGE))
        .after_help(DD_AFTER_HELP)
        .infer_long_args(true)
        .arg(Arg::new(options::OPERANDS).num_args(1..))
}
