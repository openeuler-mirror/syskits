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

//! pwd命令, 在Linux和其他类Unix系统中用于显示当前工作目录的绝对路径。

extern crate rust_i18n;
use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};

use clap::ArgAction;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{Arg, Command, crate_version};
use ctcore::Tool;
use ctcore::ct_display::ct_println_verbatim;
use ctcore::ct_error::{CTError, CTResult, CtSimpleError, FromIo, strip_errno};
use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;
use sys_locale::get_locale;

pub mod pwd_flags {
    pub const PWD_LOGICAL: &str = "logical";
    pub const PWD_PHYSICAL: &str = "physical";
    pub const PWD_ARG_OTHERS: &str = "others";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwdMode {
    Logical,
    Physical,
}

#[cfg(target_os = "linux")]
struct SigpipeGuard {
    previous: ctcore::libc::sighandler_t,
}

#[cfg(target_os = "linux")]
impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        if !ctcore::ct_sigpipe_was_default() {
            return None;
        }

        let previous =
            unsafe { ctcore::libc::signal(ctcore::libc::SIGPIPE, ctcore::libc::SIG_DFL) };
        (previous != ctcore::libc::SIG_ERR).then_some(Self { previous })
    }
}

#[cfg(target_os = "linux")]
impl Drop for SigpipeGuard {
    fn drop(&mut self) {
        unsafe {
            ctcore::libc::signal(ctcore::libc::SIGPIPE, self.previous);
        }
    }
}

#[cfg(not(target_os = "linux"))]
struct SigpipeGuard;

#[cfg(not(target_os = "linux"))]
impl SigpipeGuard {
    fn for_cli() -> Option<Self> {
        None
    }
}

pub fn pwd_physical_path() -> io::Result<PathBuf> {
    // std::env::current_dir() 是 libc::getcwd() 的一个包装。

    // 在 Unix 上，getcwd() 必须返回物理路径：
    // https://pubs.opengroup.org/onlinepubs/9699919799/functions/getcwd.html
    #[cfg(unix)]
    {
        env::current_dir()
    }

    // 在 Windows 上，我们必须解析它。
    // 在其他系统上，我们也解析它，以防万一。
    #[cfg(not(unix))]
    {
        env::current_dir().and_then(|path| path.canonicalize())
    }
}

pub fn pwd_logical_path() -> io::Result<PathBuf> {
    // 如果我们不在 Windows 上，我们按 Unix 方式处理。
    //
    // 典型的类 Unix 内核实际上并不跟踪逻辑工作目录。它们知道进程所在的精确目录，getcwd()
    // 系统调用从中重建路径。
    //
    // 逻辑工作目录由 shell 维护，在 $PWD 环境变量中。所以我们仔细检查该变量是否看起来合理，
    // 如果不合理，我们会回退到物理路径。
    //
    // POSIX: https://pubs.opengroup.org/onlinepubs/9699919799/utilities/pwd.html
    #[cfg(not(windows))]
    {
        use std::fs::metadata;
        use std::os::unix::fs::MetadataExt;
        use std::path::Path;

        fn looks_reasonable(path: &Path) -> bool {
            // 首先，检查它是否是绝对路径。
            if !path.has_root() {
                return false;
            }

            // 然后，确保没有 . 或 .. 组件。
            // Path::components() 在这里没用，它会将这些组件标准化。
            // to_string_lossy() 可能会分配，但这没关系，我们每次运行只调用一次。
            // 它也可能丢失信息，但不会丢失我们检查所需的任何信息。
            if path
                .to_string_lossy()
                .split(std::path::is_separator)
                .any(|piece| piece == "." || piece == "..")
            {
                return false;
            }

            // 最后，检查它是否与我们所在的目录匹配。
            match (metadata(path), metadata(".")) {
                (Ok(path_md), Ok(current_dir_md)) => {
                    path_md.dev() == current_dir_md.dev() && path_md.ino() == current_dir_md.ino()
                }
                _ => false,
            }
        }

        if let Some(value) = env::var_os("PWD").map(PathBuf::from) {
            if looks_reasonable(&value) {
                Ok(value)
            } else {
                env::current_dir()
            }
        } else {
            env::current_dir()
        }
    }

    // Windows 上的 getcwd() 似乎包含符号链接，所以这很简单。
    #[cfg(windows)]
    {
        env::current_dir()
    }
}

pub fn resolve_pwd_path(mode: PwdMode) -> io::Result<PathBuf> {
    match mode {
        PwdMode::Logical => pwd_logical_path(),
        PwdMode::Physical => pwd_physical_path(),
    }
}

#[cfg(target_os = "linux")]
struct PwdDirectory {
    directory: *mut ctcore::libc::DIR,
}

#[cfg(target_os = "linux")]
struct PwdDirectoryEntry {
    name: Vec<u8>,
    inode: ctcore::libc::ino_t,
}

#[cfg(target_os = "linux")]
impl PwdDirectory {
    #[cfg(test)]
    fn from_file(file: &std::fs::File) -> io::Result<Self> {
        use std::os::fd::AsRawFd;

        let duplicated_fd = unsafe { ctcore::libc::dup(file.as_raw_fd()) };
        if duplicated_fd < 0 {
            return Err(io::Error::last_os_error());
        }

        Self::from_fd(duplicated_fd)
    }

    fn from_fd(fd: ctcore::libc::c_int) -> io::Result<Self> {
        let directory = unsafe { ctcore::libc::fdopendir(fd) };
        if directory.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                ctcore::libc::close(fd);
            }
            return Err(error);
        }

        Ok(Self { directory })
    }

    fn open_parent() -> io::Result<Self> {
        let fd = unsafe {
            ctcore::libc::openat(
                ctcore::libc::AT_FDCWD,
                c"..".as_ptr(),
                ctcore::libc::O_RDONLY
                    | ctcore::libc::O_NONBLOCK
                    | ctcore::libc::O_CLOEXEC
                    | ctcore::libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Self::from_fd(fd)
        }
    }

    fn as_raw_fd(&self) -> ctcore::libc::c_int {
        unsafe { ctcore::libc::dirfd(self.directory) }
    }

    fn next_entry(&mut self) -> io::Result<Option<PwdDirectoryEntry>> {
        use std::ffi::CStr;

        unsafe {
            *ctcore::libc::__errno_location() = 0;
            let entry = ctcore::libc::readdir(self.directory);
            if entry.is_null() {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(0) {
                    Ok(None)
                } else {
                    Err(error)
                };
            }

            let entry = &*entry;
            let name = CStr::from_ptr(entry.d_name.as_ptr()).to_bytes().to_vec();
            Ok(Some(PwdDirectoryEntry {
                name,
                inode: entry.d_ino,
            }))
        }
    }

    fn close(mut self) -> io::Result<()> {
        let result = unsafe { ctcore::libc::closedir(self.directory) };
        self.directory = std::ptr::null_mut();
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for PwdDirectory {
    fn drop(&mut self) {
        if !self.directory.is_null() {
            unsafe {
                ctcore::libc::closedir(self.directory);
            }
        }
    }
}

#[cfg(all(target_os = "linux", test))]
fn pwd_open_parent_directory(parent_fd: ctcore::libc::c_int) -> io::Result<std::fs::File> {
    use std::os::fd::FromRawFd;

    let fd = unsafe {
        ctcore::libc::openat(
            parent_fd,
            c"..".as_ptr(),
            ctcore::libc::O_RDONLY
                | ctcore::libc::O_NONBLOCK
                | ctcore::libc::O_CLOEXEC
                | ctcore::libc::O_DIRECTORY,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }
}

#[cfg(target_os = "linux")]
fn pwd_entry_matches_current(
    parent_fd: ctcore::libc::c_int,
    entry: &PwdDirectoryEntry,
    parent_device: u64,
    current_device: u64,
    current_inode: u64,
) -> bool {
    use std::ffi::CString;

    let crosses_device_boundary = parent_device != current_device;
    if entry.inode != 0 && !crosses_device_boundary {
        return entry.inode == current_inode;
    }

    let name = CString::new(entry.name.as_slice()).expect("directory entries do not contain NUL");
    let mut metadata = unsafe { std::mem::zeroed::<ctcore::libc::stat>() };
    if unsafe {
        ctcore::libc::fstatat(
            parent_fd,
            name.as_ptr(),
            &mut metadata,
            ctcore::libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return false;
    }

    metadata.st_ino == current_inode
        && (!crosses_device_boundary || metadata.st_dev == current_device)
}

#[cfg(target_os = "linux")]
fn pwd_robust_getcwd() -> CTResult<PathBuf> {
    use std::ffi::OsString;
    use std::fs::metadata;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::MetadataExt;

    let root = metadata("/").map_err_context(|| "failed to get attributes of '/'".to_owned())?;
    let mut current = metadata(".").map_err_context(|| "failed to stat '.'".to_owned())?;
    let mut parent_height = 1;
    let mut components = Vec::new();

    while current.dev() != root.dev() || current.ino() != root.ino() {
        let parent_reference = pwd_parent_reference(parent_height);
        let mut directory = PwdDirectory::open_parent()
            .map_err_context(|| format!("cannot open directory '{parent_reference}'"))?;
        let parent_fd = directory.as_raw_fd();
        if unsafe { ctcore::libc::fchdir(parent_fd) } != 0 {
            return Err(io::Error::last_os_error())
                .map_err_context(|| format!("failed to chdir to '{parent_reference}'"));
        }
        let parent_metadata =
            metadata(".").map_err_context(|| format!("failed to stat '{parent_reference}'"))?;
        let mut matching_name = None;
        let mut directory_error = None;

        loop {
            let entry = match directory.next_entry() {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    directory_error = Some(error);
                    break;
                }
            };
            if entry.name == b"." || entry.name == b".." {
                continue;
            }
            if pwd_entry_matches_current(
                parent_fd,
                &entry,
                parent_metadata.dev(),
                current.dev(),
                current.ino(),
            ) {
                matching_name = Some(OsString::from_vec(entry.name));
                break;
            }
        }

        let close_error = directory.close().err();
        if let Some(error) = directory_error.or(close_error) {
            return Err(error)
                .map_err_context(|| format!("reading directory '{parent_reference}'"));
        }

        let Some(name) = matching_name else {
            return Err(CtSimpleError::new(
                1,
                format!(
                    "couldn't find directory entry in '{parent_reference}' with matching i-node"
                ),
            ));
        };

        components.push(name);
        current = parent_metadata;
        parent_height += 1;
    }

    let mut path = PathBuf::from("/");
    for component in components.iter().rev() {
        path.push(component);
    }
    Ok(path)
}

#[cfg(all(target_os = "linux", test))]
fn pwd_robust_getcwd_from_directory(mut current_fd: std::fs::File) -> CTResult<PathBuf> {
    use std::ffi::OsString;
    use std::fs::metadata;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::MetadataExt;

    let root = metadata("/").map_err_context(|| "failed to get attributes of '/'".to_owned())?;
    let mut current = current_fd
        .metadata()
        .map_err_context(|| "failed to stat '.'".to_owned())?;
    let mut parent_height = 1;
    let mut components = Vec::new();

    while current.dev() != root.dev() || current.ino() != root.ino() {
        let parent_reference = pwd_parent_reference(parent_height);
        let parent = pwd_open_parent_directory(current_fd.as_raw_fd())
            .map_err_context(|| format!("cannot open directory '{parent_reference}'"))?;
        let parent_metadata = parent
            .metadata()
            .map_err_context(|| format!("failed to stat '{parent_reference}'"))?;
        let mut directory = PwdDirectory::from_file(&parent)
            .map_err_context(|| format!("cannot open directory '{parent_reference}'"))?;
        let mut matching_name = None;
        let mut directory_error = None;

        loop {
            let entry = match directory.next_entry() {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) => {
                    directory_error = Some(error);
                    break;
                }
            };
            if entry.name == b"." || entry.name == b".." {
                continue;
            }
            if pwd_entry_matches_current(
                parent.as_raw_fd(),
                &entry,
                parent_metadata.dev(),
                current.dev(),
                current.ino(),
            ) {
                matching_name = Some(OsString::from_vec(entry.name));
                break;
            }
        }

        let close_error = directory.close().err();
        if let Some(error) = directory_error.or(close_error) {
            return Err(error)
                .map_err_context(|| format!("reading directory '{parent_reference}'"));
        }

        let Some(name) = matching_name else {
            return Err(CtSimpleError::new(
                1,
                format!(
                    "couldn't find directory entry in '{parent_reference}' with matching i-node"
                ),
            ));
        };

        components.push(name);
        current = parent_metadata;
        current_fd = parent;
        parent_height += 1;
    }

    let mut path = PathBuf::from("/");
    for component in components.iter().rev() {
        path.push(component);
    }
    Ok(path)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn pwd_robust_getcwd() -> CTResult<PathBuf> {
    use std::fs::{metadata, read_dir, symlink_metadata};
    use std::os::unix::fs::MetadataExt;

    let root = metadata("/").map_err_context(|| "failed to get attributes of '/'".to_owned())?;
    let mut current = metadata(".").map_err_context(|| "failed to stat '.'".to_owned())?;
    let mut parent_path = PathBuf::from("..");
    let mut parent_height = 1;
    let mut components = Vec::new();

    while current.dev() != root.dev() || current.ino() != root.ino() {
        let parent_reference = pwd_parent_reference(parent_height);
        let parent = metadata(&parent_path)
            .map_err_context(|| format!("failed to stat '{parent_reference}'"))?;
        let entries = read_dir(&parent_path)
            .map_err_context(|| format!("cannot open directory '{parent_reference}'"))?;
        let mut matching_name = None;

        for entry in entries {
            let entry =
                entry.map_err_context(|| format!("reading directory '{parent_reference}'"))?;
            let name = entry.file_name();
            if name.as_encoded_bytes() == b"." || name.as_encoded_bytes() == b".." {
                continue;
            }
            let Ok(entry_metadata) = symlink_metadata(entry.path()) else {
                continue;
            };
            if entry_metadata.ino() == current.ino()
                && (parent.dev() == current.dev() || entry_metadata.dev() == current.dev())
            {
                matching_name = Some(name);
                break;
            }
        }

        let Some(name) = matching_name else {
            return Err(CtSimpleError::new(
                1,
                format!(
                    "couldn't find directory entry in '{parent_reference}' with matching i-node"
                ),
            ));
        };

        components.push(name);
        current = parent;
        parent_path.push("..");
        parent_height += 1;
    }

    let mut path = PathBuf::from("/");
    for component in components.iter().rev() {
        path.push(component);
    }
    Ok(path)
}

#[cfg(unix)]
fn pwd_parent_reference(height: usize) -> String {
    std::iter::repeat_n("..", height)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(not(unix))]
fn pwd_robust_getcwd() -> CTResult<PathBuf> {
    env::current_dir().map_err_context(|| "failed to get current directory".to_owned())
}

fn default_pwd_mode(posixly_correct: Option<&OsStr>) -> PwdMode {
    if posixly_correct.is_some() {
        PwdMode::Logical
    } else {
        PwdMode::Physical
    }
}

fn resolve_pwd_mode(matches: &clap::ArgMatches, posixly_correct: Option<&OsStr>) -> PwdMode {
    if matches.get_flag(pwd_flags::PWD_PHYSICAL) {
        PwdMode::Physical
    } else if matches.get_flag(pwd_flags::PWD_LOGICAL) {
        PwdMode::Logical
    } else {
        default_pwd_mode(posixly_correct)
    }
}

const PWD_LONG_OPTIONS: &[&str] = &["logical", "physical", "help", "version"];
const PWD_SHORT_EXTENSIONS: &[u8] = b"hV";

#[derive(Debug, PartialEq, Eq)]
enum PwdLongOptionMatch {
    Recognized(&'static str),
    Ambiguous(Vec<&'static str>),
    None,
}

#[derive(Debug)]
struct PwdUsageError {
    message: Vec<u8>,
}

impl PwdUsageError {
    fn boxed(message: Vec<u8>) -> Box<dyn CTError> {
        Box::new(Self { message })
    }
}

impl Display for PwdUsageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.message).fmt(formatter)
    }
}

impl Error for PwdUsageError {}

impl CTError for PwdUsageError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.message)
    }

    fn usage(&self) -> bool {
        true
    }
}

fn pwd_prepare_args(args: impl ctcore::Args, posixly_correct: bool) -> CTResult<Vec<OsString>> {
    let args = args.collect::<Vec<_>>();
    let mut parse_options = true;

    for argument in args.iter().skip(1) {
        if !parse_options {
            continue;
        }

        let bytes = argument.as_encoded_bytes();
        if bytes == b"--" {
            parse_options = false;
            continue;
        }
        if bytes.len() <= 1 || bytes[0] != b'-' {
            if posixly_correct {
                parse_options = false;
            }
            continue;
        }

        if let Some(long) = bytes.strip_prefix(b"--") {
            let separator = long.iter().position(|byte| *byte == b'=');
            let name = &long[..separator.unwrap_or(long.len())];
            match pwd_match_long_option(name) {
                PwdLongOptionMatch::Recognized(canonical) if separator.is_some() => {
                    return Err(PwdUsageError::boxed(
                        format!("option '--{canonical}' doesn't allow an argument").into_bytes(),
                    ));
                }
                PwdLongOptionMatch::Ambiguous(candidates) => {
                    let mut message = b"option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.extend_from_slice(b"' is ambiguous; possibilities:");
                    for candidate in candidates {
                        message.extend_from_slice(b" '--");
                        message.extend_from_slice(candidate.as_bytes());
                        message.push(b'\'');
                    }
                    return Err(PwdUsageError::boxed(message));
                }
                PwdLongOptionMatch::None => {
                    let mut message = b"unrecognized option '".to_vec();
                    message.extend_from_slice(bytes);
                    message.push(b'\'');
                    return Err(PwdUsageError::boxed(message));
                }
                PwdLongOptionMatch::Recognized(_) => continue,
            }
        }

        for option in &bytes[1..] {
            if matches!(option, b'L' | b'P') || PWD_SHORT_EXTENSIONS.contains(option) {
                continue;
            }

            let mut message = b"invalid option -- '".to_vec();
            message.push(*option);
            message.push(b'\'');
            return Err(PwdUsageError::boxed(message));
        }
    }

    Ok(args)
}

fn pwd_match_long_option(name: &[u8]) -> PwdLongOptionMatch {
    if let Some(option) = PWD_LONG_OPTIONS
        .iter()
        .copied()
        .find(|option| option.as_bytes() == name)
    {
        return PwdLongOptionMatch::Recognized(option);
    }

    let candidates = PWD_LONG_OPTIONS
        .iter()
        .copied()
        .filter(|option| option.as_bytes().starts_with(name))
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [] => PwdLongOptionMatch::None,
        [candidate] => PwdLongOptionMatch::Recognized(candidate),
        _ => PwdLongOptionMatch::Ambiguous(candidates),
    }
}

pub fn pwd_main(args: impl ctcore::Args) -> CTResult<()> {
    let _sigpipe_guard = SigpipeGuard::for_cli();
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let posixly_correct = env::var_os("POSIXLY_CORRECT");
    let matches = ct_app_with_posixly_correct(posixly_correct.is_some())
        .try_get_matches_from(pwd_prepare_args(args, posixly_correct.is_some())?)?;
    if matches
        .get_many::<String>(pwd_flags::PWD_ARG_OTHERS)
        .is_some()
    {
        ctcore::ct_show_error!("ignoring non-option arguments");
    }
    // 如果设置了 POSIXLY_CORRECT，我们希望进行逻辑解析。
    // 这在执行 mkdir -p a/b && ln -s a/b c && cd c && pwd 时会产生不同的输出
    // 在这种情况下，我们应该在路径末尾得到 c 而不是 a/b
    let cwd = match resolve_pwd_path(resolve_pwd_mode(&matches, posixly_correct.as_deref())) {
        Ok(path) => path,
        Err(_) => pwd_robust_getcwd()?,
    };

    // \\?\ 是 Windows 在某些情况下给路径加的前缀，包括对它们进行规范化时。
    // 有了正确的扩展特性，我们可以无损地删除它，但我们无损地打印它，所以没有理由麻烦。
    #[cfg(windows)]
    let cwd = cwd
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .map(Into::into)
        .unwrap_or(cwd);

    ct_println_verbatim(cwd).map_err(pwd_write_error)?;

    Ok(())
}

fn pwd_write_error(error: io::Error) -> Box<dyn ctcore::ct_error::CTError> {
    pwd_redirect_stdout_to_dev_null();
    let error = pwd_normalize_stdout_write_error(error, ctcore::ct_stdout_was_closed());
    CtSimpleError::new(1, format!("write error: {}", strip_errno(&error)))
}

fn pwd_normalize_stdout_write_error(error: io::Error, stdout_was_closed: bool) -> io::Error {
    #[cfg(unix)]
    if stdout_was_closed {
        return io::Error::from_raw_os_error(ctcore::libc::EBADF);
    }

    error
}

#[cfg(unix)]
fn pwd_redirect_stdout_to_dev_null() {
    const DEV_NULL: &[u8] = b"/dev/null\0";

    unsafe {
        let fd = ctcore::libc::open(DEV_NULL.as_ptr().cast(), ctcore::libc::O_WRONLY);
        if fd >= 0 {
            ctcore::libc::dup2(fd, ctcore::libc::STDOUT_FILENO);
            ctcore::libc::close(fd);
        }
    }
}

#[cfg(not(unix))]
fn pwd_redirect_stdout_to_dev_null() {}

pub fn ct_app() -> Command {
    ct_app_with_posixly_correct(false)
}

fn ct_app_with_posixly_correct(posixly_correct: bool) -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("pwd.about");
    let usage_description = t!("pwd.usage");
    let others = Arg::new(pwd_flags::PWD_ARG_OTHERS)
        .action(ArgAction::Append)
        .value_hint(clap::ValueHint::AnyPath);
    let others = if posixly_correct {
        others.trailing_var_arg(true).allow_hyphen_values(true)
    } else {
        others
    };
    let args = vec![
        Arg::new(pwd_flags::PWD_LOGICAL)
            .short('L')
            .long(pwd_flags::PWD_LOGICAL)
            .help(t!("pwd.clap.pwd_logical"))
            .action(ArgAction::SetTrue),
        Arg::new(pwd_flags::PWD_PHYSICAL)
            .short('P')
            .long(pwd_flags::PWD_PHYSICAL)
            .overrides_with(pwd_flags::PWD_LOGICAL)
            .help(t!("pwd.clap.pwd_physical"))
            .action(ArgAction::SetTrue),
        others,
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

#[derive(Default)]
pub struct Pwd;
impl Tool for Pwd {
    fn name(&self) -> &'static str {
        "pwd"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        pwd_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn test_tool_implementation() {
        let tool = Pwd;

        // 测试 name 方法
        assert_eq!(tool.name(), "pwd");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("pwd"));

        // 测试 execute 方法 - pwd 应该忽略非选项参数
        let args = vec![OsString::from("pwd")];
        if let Err(err) = tool.execute(&args) {
            // In parallel test execution, cwd can be invalidated by other tests.
            // pwd then reports an error with exit code 1.
            assert_eq!(err.code(), 1);
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_posixly_correct_selects_logical_default() {
        use std::os::unix::ffi::OsStringExt;

        let value = OsString::from_vec(vec![0xff]);

        assert_eq!(default_pwd_mode(Some(value.as_os_str())), PwdMode::Logical,);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn robust_getcwd_reconstructs_paths_beyond_path_max() {
        use std::fs::File;
        use std::os::fd::{AsRawFd, FromRawFd};

        const DEPTH: usize = 1400;
        const COMPONENT: &str = "dir";

        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let mut expected = temp_dir.path().to_path_buf();
        let mut current = File::open(temp_dir.path()).expect("open temporary directory");

        for _ in 0..DEPTH {
            assert_eq!(
                unsafe { ctcore::libc::mkdirat(current.as_raw_fd(), c"dir".as_ptr(), 0o700) },
                0
            );
            let child_fd = unsafe {
                ctcore::libc::openat(
                    current.as_raw_fd(),
                    c"dir".as_ptr(),
                    ctcore::libc::O_PATH | ctcore::libc::O_DIRECTORY | ctcore::libc::O_CLOEXEC,
                )
            };
            assert!(child_fd >= 0, "open nested directory");
            current = unsafe { File::from_raw_fd(child_fd) };
            expected.push(COMPONENT);
        }

        let result = pwd_robust_getcwd_from_directory(
            current.try_clone().expect("duplicate nested directory"),
        );

        for _ in 0..DEPTH {
            let parent = pwd_open_parent_directory(current.as_raw_fd())
                .expect("open parent directory for cleanup");
            assert_eq!(
                unsafe {
                    ctcore::libc::unlinkat(
                        parent.as_raw_fd(),
                        c"dir".as_ptr(),
                        ctcore::libc::AT_REMOVEDIR,
                    )
                },
                0
            );
            current = parent;
        }
        drop(current);
        temp_dir.close().expect("remove temporary directory");

        assert_eq!(
            result.expect("reconstruct deep working directory"),
            expected
        );
    }

    #[cfg(test)]
    mod ct_main_tests {
        use super::*;
        use std::ffi::OsString;
        #[test]
        fn test_pwd_main_execution_version() {
            let args_vec = [ctcore::ct_util_name(), "--version"];
            let args = args_vec.iter().map(OsString::from);
            let result = pwd_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_other_version() {
            let args_vec = [ctcore::ct_util_name(), "-V"];

            let args = args_vec.iter().map(OsString::from);
            let result = pwd_main(args);

            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_help() {
            let args_vec = [ctcore::ct_util_name(), "--help"];
            let args = args_vec.iter().map(OsString::from);
            let result = pwd_main(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_help_short() {
            let args_vec = [ctcore::ct_util_name(), "-h"];
            let args = args_vec.iter().map(OsString::from);
            let result = pwd_main(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_execution_unsupport_help() {
            let args_vec = [ctcore::ct_util_name(), "-H"];
            let args = args_vec.iter().map(OsString::from);
            let result = pwd_main(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_pwd_main_invalid_argument() {
            let args_vec = [ctcore::ct_util_name(), "--invalid-argument"];
            let args = args_vec.iter().map(OsString::from);
            let result = pwd_main(args);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod env_data_tests {
        use super::*;
        use std::env;
        use std::ffi::OsString;
        use std::fs;
        use tempfile::TempDir;

        #[test]
        fn test_env_data() {
            // test_pwd_main_support_missing_argument()
            {
                let args_vec = [ctcore::ct_util_name()];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_long()
            {
                let args_vec = [ctcore::ct_util_name(), "--logical"];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_short()
            {
                let args_vec = [ctcore::ct_util_name(), "-L"];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            //  test_pwd_main_physical_long()
            {
                let args_vec = [ctcore::ct_util_name(), "--physical"];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_physical_short()
            {
                let args_vec = [ctcore::ct_util_name(), "-P"];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_long_with_file()
            {
                let file_name = "test_pwd_main_logical_long";

                let args_vec = [ctcore::ct_util_name(), "--logical", file_name];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_logical_short_with_file()
            {
                let file_name = "test_pwd_main_logical_short";

                let args_vec = [ctcore::ct_util_name(), "-L", file_name];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_physical_long_with_file()
            {
                let file_name = "test_pwd_main_physical_long";

                let args_vec = [ctcore::ct_util_name(), "--physical", file_name];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_pwd_main_physical_short_with_file()
            {
                let file_name = "test_pwd_main_physical_short";

                let args_vec = [ctcore::ct_util_name(), "-P", file_name];
                let args = args_vec.iter().map(OsString::from);
                let result = pwd_main(args);
                assert!(result.is_ok());
            }

            // test_physical_path_basic()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                assert_eq!(result, temp_dir_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nested_symlink()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path = nested_dir.join("symlink");
                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                std::os::unix::fs::symlink(&target_path, &symlink_path)
                    .expect("failed to create symlink");
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&target_path, &symlink_path)
                    .expect("failed to create symlink");

                env::set_current_dir(&symlink_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_multiple_symlinks()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path1 = nested_dir.join("symlink1");
                let symlink_path2 = temp_dir_path.join("symlink2");

                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::unix::fs::symlink(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                #[cfg(windows)]
                {
                    std::os::windows::fs::symlink_dir(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::windows::fs::symlink_dir(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                env::set_current_dir(&symlink_path2).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nonexistent_directory()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let nonexistent_path = temp_dir_path.join("nonexistent");

                env::set_current_dir(&nonexistent_path)
                    .expect_err("should fail to change directory");
                let result = pwd_physical_path();
                assert!(result.is_ok());

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_long_path()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let long_path = temp_dir_path.join("a".repeat(255));
                std::fs::create_dir_all(&long_path).expect("failed to create long path dir");
                env::set_current_dir(&long_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = long_path
                    .canonicalize()
                    .expect("failed to canonicalize long path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_basic()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                assert_eq!(result, temp_dir_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nested_symlink()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path = nested_dir.join("symlink");
                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                std::os::unix::fs::symlink(&target_path, &symlink_path)
                    .expect("failed to create symlink");
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&target_path, &symlink_path)
                    .expect("failed to create symlink");

                env::set_current_dir(&symlink_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_multiple_symlinks()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let nested_dir = temp_dir_path.join("nested");
                std::fs::create_dir(&nested_dir).expect("failed to create nested dir");

                let target_path = nested_dir.join("target");
                let symlink_path1 = nested_dir.join("symlink1");
                let symlink_path2 = temp_dir_path.join("symlink2");

                std::fs::create_dir(&target_path).expect("failed to create target dir");

                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::unix::fs::symlink(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                #[cfg(windows)]
                {
                    std::os::windows::fs::symlink_dir(&target_path, &symlink_path1)
                        .expect("failed to create symlink1");
                    std::os::windows::fs::symlink_dir(&symlink_path1, &symlink_path2)
                        .expect("failed to create symlink2");
                }

                env::set_current_dir(&symlink_path2).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = target_path
                    .canonicalize()
                    .expect("failed to canonicalize target path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_nonexistent_directory()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");

                let nonexistent_path = temp_dir_path.join("nonexistent");

                env::set_current_dir(&nonexistent_path)
                    .expect_err("should fail to change directory");
                let result = pwd_physical_path();
                assert!(result.is_ok());

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_physical_path_with_long_path()
            {
                let temp_dir = TempDir::new().unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let long_path = temp_dir_path.join("a".repeat(255));
                std::fs::create_dir_all(&long_path).expect("failed to create long path dir");
                env::set_current_dir(&long_path).expect("failed to change directory");

                let result = pwd_physical_path().expect("failed to get physical path");
                let expected_path = long_path
                    .canonicalize()
                    .expect("failed to canonicalize long path");
                assert_eq!(result, expected_path);

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_unix()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                let logical_path_buf = temp_dir_path.join("logical");
                fs::create_dir(&logical_path_buf).expect("failed to create logical dir");

                let symlink_path = temp_dir_path.join("symlink");
                #[cfg(unix)]
                std::os::unix::fs::symlink(&logical_path_buf, &symlink_path)
                    .expect("failed to create symlink");
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(&logical_path_buf, &symlink_path)
                    .expect("failed to create symlink");

                // 切换到符号链接目录
                env::set_current_dir(&symlink_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", &symlink_path) };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, symlink_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_invalid_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", "/invalid/path") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_unix_no_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::remove_var("PWD") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_unix_relative_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", "relative/path") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_windows_valid_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", &temp_dir_path) };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_windows_invalid_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::set_var("PWD", "C:\\invalid\\path") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);
                unsafe { env::remove_var("PWD") };

                temp_dir.close().expect("failed to close temp dir");
            }

            // test_logical_path_windows_no_pwd()
            {
                let temp_dir = TempDir::with_prefix("test_logical_path_").unwrap();
                let temp_dir_path = temp_dir.path().to_path_buf();

                env::set_current_dir(&temp_dir_path).expect("failed to change directory");
                unsafe { env::remove_var("PWD") };

                let result = pwd_logical_path().expect("failed to get logical path");
                assert_eq!(result, temp_dir_path);

                temp_dir.close().expect("failed to close temp dir");
            }
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        // pwd 接口: pwd [OPTION]...
        //
        // Options:
        //   -L, --logical   use PWD from environment, even if it contains symlinks
        //   -P, --physical  avoid all symlinks
        //   -h, --help      Print help
        //   -V, --version   Print version

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];

            let result = command.try_get_matches_from(args);

            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_help_short() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-h"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();

            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();

            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name()];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn posixly_correct_stops_option_parsing_after_first_operand() {
            let command = ct_app_with_posixly_correct(true);
            let matches = command
                .try_get_matches_from([ctcore::ct_util_name(), "operand", "-P"])
                .expect("arguments should parse");

            assert!(!matches.get_flag(pwd_flags::PWD_PHYSICAL));
            assert_eq!(
                matches
                    .get_many::<String>(pwd_flags::PWD_ARG_OTHERS)
                    .expect("non-option arguments")
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["operand", "-P"]
            );
        }

        #[test]
        fn test_ct_app_logical_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--logical"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_logical_short() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-L"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_long() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--physical"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_short() {
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-P"];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_logical_long_with_file() {
            let file_name = "test_ct_app_logical_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--logical", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_logical_short_with_file() {
            let file_name = "test_ct_app_logical_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-L", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_long_with_file() {
            let file_name = "test_ct_app_physical_long";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "--physical", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_physical_short_with_file() {
            let file_name = "test_ct_app_physical_short";
            let command = ct_app();

            let args = vec![ctcore::ct_util_name(), "-P", file_name];
            let result = command.try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
