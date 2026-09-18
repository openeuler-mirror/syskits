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
use crate::split_quote_path;
use ctcore::ct_error::{CTError, strip_errno};
use ctcore::ct_fs;
use ctcore::ct_fs::CtFileInformation;
use ctcore::ct_signals::get_ct_signal_name_by_value;
use std::borrow::Cow;
use std::cell::RefCell;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::io::Write;
use std::io::{BufWriter, Error, Result};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};

thread_local! {
    static FILTER_FAILURE: RefCell<Option<FilterFailure>> = const { RefCell::new(None) };
}

enum FilterFailure {
    Exit {
        file_name: OsString,
        command: OsString,
        code: i32,
    },
    Signal {
        file_name: OsString,
        command: OsString,
        signal: i32,
    },
    Wait(String),
}

#[derive(Debug)]
struct FilterFailureError {
    code: i32,
    diagnostic: Vec<u8>,
}

impl Display for FilterFailureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        String::from_utf8_lossy(&self.diagnostic).fmt(formatter)
    }
}

impl std::error::Error for FilterFailureError {}

impl CTError for FilterFailureError {
    fn diagnostic_bytes(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.diagnostic)
    }

    fn code(&self) -> i32 {
        self.code
    }
}

fn record_filter_failure(failure: FilterFailure) {
    FILTER_FAILURE.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(failure);
        }
    });
}

pub fn reset_filter_failure() {
    FILTER_FAILURE.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

pub fn filter_failure_recorded() -> bool {
    FILTER_FAILURE.with(|slot| slot.borrow().is_some())
}

pub fn take_filter_failure() -> Option<Box<dyn CTError>> {
    FILTER_FAILURE.with(|slot| slot.borrow_mut().take().map(filter_failure_error))
}

fn filter_failure_error(failure: FilterFailure) -> Box<dyn CTError> {
    let (code, diagnostic) = match failure {
        FilterFailure::Exit {
            file_name,
            command,
            code,
        } => {
            let mut diagnostic = format!(
                "with FILE={}, exit {code} from command: ",
                split_quote_path(file_name.as_os_str(), false)
            )
            .into_bytes();
            diagnostic.extend_from_slice(command.as_os_str().as_bytes());
            (code, diagnostic)
        }
        FilterFailure::Signal {
            file_name,
            command,
            signal,
        } => {
            let signal_name = get_ct_signal_name_by_value(signal as usize)
                .map(str::to_owned)
                .unwrap_or_else(|| signal.to_string());
            let mut diagnostic = format!(
                "with FILE={}, signal {signal_name} from command: ",
                split_quote_path(file_name.as_os_str(), false)
            )
            .into_bytes();
            diagnostic.extend_from_slice(command.as_os_str().as_bytes());
            (signal + 128, diagnostic)
        }
        FilterFailure::Wait(error) => (
            1,
            format!("waiting for child process: {error}").into_bytes(),
        ),
    };

    Box::new(FilterFailureError { code, diagnostic })
}

/// A writer that writes to a shell_process' stdin
///
/// We use a shell process (not directly calling a sub-process) so we can forward the name of the
/// corresponding output file (xaa, xab, xac… ). This is the way it was implemented in GNU split.
struct UnixFilterWriter {
    /// Running shell process
    shell_process: Child,
    file_name: OsString,
    command: OsString,
}

impl Write for UnixFilterWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.shell_process
            .stdin
            .as_mut()
            .expect("failed to get shell stdin")
            .write(buf)
    }
    fn flush(&mut self) -> Result<()> {
        self.shell_process
            .stdin
            .as_mut()
            .expect("failed to get shell stdin")
            .flush()
    }
}

impl UnixFilterWriter {
    /// Create a new filter running a command with $FILE pointing at the output name
    ///
    /// #Arguments
    ///
    /// * `command` - The shell command to execute
    /// * `filepath` - Path of the output file (forwarded to command as $FILE)
    fn new(command: &OsStr, filepath: &OsStr) -> Result<Self> {
        let shell_program = filter_shell_program(env::var_os("SHELL"));
        let shell_argv0 = filter_shell_argv0(shell_program.as_os_str()).to_os_string();
        let shell_process = match Command::new(&shell_program)
            .arg0(shell_argv0)
            .arg("-c")
            .arg(command)
            .env("FILE", filepath)
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if filter_exec_error(&error) => {
                write_filter_exec_error(shell_program.as_os_str(), command, &error);
                record_filter_failure(FilterFailure::Exit {
                    file_name: filepath.to_os_string(),
                    command: command.to_os_string(),
                    code: 1,
                });
                return Err(Error::from_raw_os_error(ctcore::libc::EPIPE));
            }
            Err(error) => return Err(error),
        };

        Ok(Self {
            shell_process,
            file_name: filepath.to_os_string(),
            command: command.to_os_string(),
        })
    }
}

fn filter_exec_error(error: &Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ctcore::libc::ENOENT)
            | Some(ctcore::libc::EACCES)
            | Some(ctcore::libc::ENOEXEC)
            | Some(ctcore::libc::ETXTBSY)
    )
}

fn filter_exec_error_message(shell_program: &OsStr, command: &OsStr, error: &Error) -> Vec<u8> {
    let mut diagnostic = b"failed to run command: \"".to_vec();
    diagnostic.extend_from_slice(shell_program.as_bytes());
    diagnostic.extend_from_slice(b" -c ");
    diagnostic.extend_from_slice(command.as_bytes());
    diagnostic.extend_from_slice(b"\": ");
    diagnostic.extend_from_slice(strip_errno(error).as_bytes());
    diagnostic
}

fn write_filter_exec_error(shell_program: &OsStr, command: &OsStr, error: &Error) {
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(ctcore::ct_util_name().as_bytes());
    let _ = stderr.write_all(b": ");
    let _ = stderr.write_all(&filter_exec_error_message(shell_program, command, error));
    let _ = stderr.write_all(b"\n");
}

fn filter_shell_program(shell: Option<OsString>) -> OsString {
    shell.unwrap_or_else(|| OsString::from("/bin/sh"))
}

fn filter_shell_argv0(shell: &OsStr) -> &OsStr {
    let bytes = shell.as_bytes();
    let mut base = bytes
        .iter()
        .position(|byte| *byte != b'/')
        .unwrap_or(bytes.len());
    let mut last_was_slash = false;

    for (index, byte) in bytes.iter().enumerate().skip(base) {
        if *byte == b'/' {
            last_was_slash = true;
        } else if last_was_slash {
            base = index;
            last_was_slash = false;
        }
    }

    OsStr::from_bytes(&bytes[base..])
}

impl Drop for UnixFilterWriter {
    /// Close stdin and wait on the filter process before dropping the writer.
    fn drop(&mut self) {
        drop(self.shell_process.stdin.take());
        match self.shell_process.wait() {
            Ok(exit_status) => match exit_status.code() {
                Some(code) if code != 0 => record_filter_failure(FilterFailure::Exit {
                    file_name: self.file_name.clone(),
                    command: self.command.clone(),
                    code,
                }),
                Some(_) => {}
                None => {
                    if let Some(signal) = exit_status.signal()
                        && signal != ctcore::libc::SIGPIPE
                    {
                        record_filter_failure(FilterFailure::Signal {
                            file_name: self.file_name.clone(),
                            command: self.command.clone(),
                            signal,
                        });
                    }
                }
            },
            Err(error) => record_filter_failure(FilterFailure::Wait(error.to_string())),
        }
    }
}

/// Instantiate either a file writer or a "write to shell process's stdin" writer
pub fn instantiate_current_writer(
    opt_filter: &Option<OsString>,
    file_name: impl AsRef<OsStr>,
    new: bool,
) -> Result<BufWriter<Box<dyn Write>>> {
    let file_name = file_name.as_ref();
    match opt_filter {
        None => {
            let file = if new {
                // 创建新文件
                std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(std::path::Path::new(file_name))
                    .map_err(|_| {
                        Error::other(format!(
                            "unable to open '{}'; aborting",
                            split_quote_path(file_name, false)
                        ))
                    })?
            } else {
                // 重新打开之前创建的文件以便追加写入
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(std::path::Path::new(file_name))
                    .map_err(|_| {
                        Error::other(format!(
                            "unable to re-open '{}'; aborting",
                            split_quote_path(file_name, false)
                        ))
                    })?
            };
            Ok(BufWriter::new(Box::new(file) as Box<dyn Write>))
        }
        Some(filter_command) => Ok(BufWriter::new(Box::new(
            // spawn a shell command and write to it
            UnixFilterWriter::new(filter_command, file_name)?,
        ) as Box<dyn Write>)),
    }
}

pub fn paths_refer_to_same_file(path1: impl AsRef<OsStr>, path2: impl AsRef<OsStr>) -> bool {
    let path1 = path1.as_ref();
    let path2 = path2.as_ref();
    // 我们必须考虑符号链接和相对路径。
    let p1 = if path1 == OsStr::new("-") {
        CtFileInformation::from_file(&std::io::stdin())
    } else {
        CtFileInformation::from_path(Path::new(path1), true)
    };
    ct_fs::infos_refer_to_same_file(p1, CtFileInformation::from_path(Path::new(path2), true))
}

#[cfg(test)]
mod tests {
    use super::{
        FilterFailure, filter_exec_error_message, filter_failure_error, filter_shell_argv0,
        filter_shell_program,
    };
    use crate::SpliceSettings;
    use crate::ct_app;
    use crate::platform::instantiate_current_writer;
    use crate::platform::paths_refer_to_same_file;
    use std::fs;
    use std::fs::File;

    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;
    use tempfile::Builder;

    #[test]
    fn filter_shell_program_preserves_non_utf8_environment_value() {
        let shell = std::ffi::OsString::from_vec(b"./shell-\xff".to_vec());

        let selected = filter_shell_program(Some(shell));

        assert_eq!(selected.as_os_str().as_bytes(), b"./shell-\xff");
    }

    #[test]
    fn filter_shell_argv0_uses_last_path_component() {
        let shell = std::ffi::OsStr::from_bytes(b"/opt/shells/bash-\xff");

        let argv0 = filter_shell_argv0(shell);

        assert_eq!(argv0.as_bytes(), b"bash-\xff");
    }

    #[test]
    fn filter_failure_error_preserves_non_utf8_command_bytes() {
        let error = filter_failure_error(FilterFailure::Exit {
            file_name: OsString::from("out-aa"),
            command: OsString::from_vec(b"exit 42 #\xff".to_vec()),
            code: 42,
        });

        assert_eq!(
            error.diagnostic_bytes().as_ref(),
            b"with FILE=out-aa, exit 42 from command: exit 42 #\xff"
        );
        assert_eq!(error.code(), 42);
    }

    #[test]
    fn filter_exec_error_message_preserves_non_utf8_shell_and_command_bytes() {
        let shell = OsString::from_vec(b"./shell-\xff".to_vec());
        let command = OsString::from_vec(b"cat > \"$FILE\" #\xfe".to_vec());
        let error = std::io::Error::from_raw_os_error(ctcore::libc::ENOENT);

        assert_eq!(
            filter_exec_error_message(shell.as_os_str(), command.as_os_str(), &error),
            b"failed to run command: \"./shell-\xff -c cat > \"$FILE\" #\xfe\": No such file or directory"
        );
    }

    fn split_test_base_dir() -> std::path::PathBuf {
        let base = std::env::temp_dir()
            .join("syskits-split-tests")
            .join(std::process::id().to_string());
        let _ = std::fs::create_dir_all(&base);
        base
    }

    fn unique_output_filename() -> &'static str {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Box::leak(
            split_test_base_dir()
                .join(format!("output.{}.{}.txt", std::process::id(), seq))
                .to_string_lossy()
                .into_owned()
                .into_boxed_str(),
        )
    }

    #[test]
    fn test_same_absolute_paths() {
        let p1 = "/path/to/file.txt";
        let p2 = "/path/to/file.txt";
        assert!(!paths_refer_to_same_file(p1, p2));
    }

    #[test]
    fn test_different_absolute_paths() {
        let p3 = "/path/to/file.txt";
        let p4 = "/path/to/another_file.txt";
        assert!(!paths_refer_to_same_file(p3, p4));
    }

    // TODO: Implement the following tests after addressing the TODO comments in the original code.

    // #[test]
    // fn test_stdin_and_file_path() {
    //     let p5 = "-";
    //     let p6 = "/path/to/file.txt";
    //     // Redirect stdin to a file and test
    // }

    // #[test]
    // fn test_symlink_and_file_path() {
    //     let p7 = "/path/to/symlink.txt";
    //     let p8 = "/path/to/another_file.txt";
    //     // Create a symlink and test
    // }

    #[test]
    fn test_same_relative_paths() {
        let p9 = "file.txt";
        let p10 = "file.txt";
        assert!(!paths_refer_to_same_file(p9, p10));
    }

    #[test]
    fn test_different_relative_paths() {
        let p11 = "file.txt";
        let p12 = "another_file.txt";
        assert!(!paths_refer_to_same_file(p11, p12));
    }

    #[test]
    fn test_relative_and_absolute_paths_same_file() {
        let p13 = "file.txt";
        let p14 = "/path/to/file.txt";
        assert!(!paths_refer_to_same_file(p13, p14));
    }

    #[test]
    fn test_instantiate_current_writer_same_file() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
        let result = command.try_get_matches_from(args);

        let mut settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = "input.txt";
        // Set the `input` to the same as `filename`
        settings.input = filename.to_string();

        // Call the `instantiate_current_writer` method and assert the result is `Err`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_different_file() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }
    #[test]
    fn test_instantiate_current_writer_b() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-b", "5"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_b_15() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-b", "15"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_100() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "100"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_1000() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "1000"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10k() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10K"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10m() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10M"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10g() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10G"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10t() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10T"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10p() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10P"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10e() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10E"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10z() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Z"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10y() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Y"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10r() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10R"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_bytes_10q() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--bytes", "10Q"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_c() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-C", "5"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_lines_bytes_10() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "10"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_lines_bytes_100() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "100"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_lines_bytes_1000() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--line-bytes", "1000"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_l() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-l", "5"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_lines_10() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--lines", "10"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_lines_100() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--lines", "100"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_lines_1000() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--lines", "1000"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_n() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-n", "5"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_number_10() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--number", "10"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_number_100() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--number", "100"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_number_1000() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--number", "1000"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_additional_suffix_10() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--additional-suffix",
            "10",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_additional_suffix_100() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--additional-suffix",
            "100",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_additional_suffix_1000() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--additional-suffix",
            "1000",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_filter_ls() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--filter", "ls"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_filter_cat() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cat"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_filter_cd() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--filter", "cd"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_filter_tail() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--filter", "tail"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_number_filter() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--number",
            "10",
            "--filter",
            "ls",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_number_additional_suffix() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--number",
            "10",
            "--additional-suffix",
            ".txt",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_filter_additional_suffix() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--filter",
            "ls",
            "--additional-suffix",
            ".txt",
        ];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_number_additional_suffix_filter() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--number",
            "10",
            "--additional-suffix",
            ".txt",
            "--filter",
            "ls",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_elide_empty_files() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--elide-empty-files"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_e() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-e"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_d() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-d", "txt"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_numeric_suffixes() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--numeric-suffixes=333"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    // #[test]
    #[test]
    fn test_instantiate_current_writer_x() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-x", "111"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_hex_suffixes() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--hex-suffixes=11"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_d_hex_suffixes() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-d", "--hex-suffixes=11"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_a() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-a", "11"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_suffix_length() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--suffix-length=11"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_d_suffix_length() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "-d",
            "--suffix-length=11",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_verbose() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "--verbose"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_a_verbose() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), filename1, "-a", "111", "--verbose"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_suffix_length_verbose() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "--suffix-length=11",
            "--verbose",
        ];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_d_suffix_length_verbose() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            filename1,
            "-d",
            "--suffix-length=11",
            "--verbose",
        ];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_t() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), filename1, "-t", "\0"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_separator_zero() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\0"];
        let result = command.try_get_matches_from(args);
        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_separator_n() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\n"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_separator_r() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\r"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiate_current_writer_separator_t() {
        let temp_dir = Builder::new()
            .prefix("tests_instantiate_current_writer_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_111");
        File::create(&test_file_1).unwrap();
        let filename1 = test_file_1.to_str().unwrap();

        let command = ct_app();

        let args = vec![ctcore::ct_util_name(), filename1, "--separator", "\t"];
        let result = command.try_get_matches_from(args);

        let settings = SpliceSettings::from(&result.unwrap(), &None).unwrap();
        let filename = unique_output_filename();

        // Call the `instantiate_current_writer` method and assert the result is `Ok`
        let result = instantiate_current_writer(&settings.filter, filename, true);
        let file_path = Path::new(filename);
        match fs::remove_file(file_path) {
            Ok(()) => {
                // println!("文件删除成功");
            }
            Err(_e) => {
                // eprintln!("File remove fail: {}", e)
            }
        }

        assert!(result.is_ok());
    }
}
