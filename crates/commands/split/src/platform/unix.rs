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
use ctcore::ct_fs;
use ctcore::ct_fs::CtFileInformation;
use ctcore::ct_signals::get_ct_signal_name_by_value;
use std::cell::RefCell;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::io::{BufWriter, Error, Result};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

thread_local! {
    static FILTER_FAILURE: RefCell<Option<FilterFailure>> = const { RefCell::new(None) };
}

enum FilterFailure {
    Exit {
        file_name: OsString,
        command: String,
        code: i32,
    },
    Signal {
        file_name: OsString,
        command: String,
        signal: i32,
    },
    Wait(String),
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

pub fn take_filter_failure() -> Option<(i32, String)> {
    FILTER_FAILURE.with(|slot| {
        slot.borrow_mut().take().map(|failure| match failure {
            FilterFailure::Exit {
                file_name,
                command,
                code,
            } => (
                code,
                format!(
                    "with FILE={}, exit {code} from command: {command}",
                    split_quote_path(file_name.as_os_str(), false)
                ),
            ),
            FilterFailure::Signal {
                file_name,
                command,
                signal,
            } => {
                let signal_name = get_ct_signal_name_by_value(signal as usize)
                    .map(str::to_owned)
                    .unwrap_or_else(|| signal.to_string());
                (
                    signal + 128,
                    format!(
                        "with FILE={}, signal {signal_name} from command: {command}",
                        split_quote_path(file_name.as_os_str(), false)
                    ),
                )
            }
            FilterFailure::Wait(error) => (1, format!("waiting for child process: {error}")),
        })
    })
}

/// A writer that writes to a shell_process' stdin
///
/// We use a shell process (not directly calling a sub-process) so we can forward the name of the
/// corresponding output file (xaa, xab, xac… ). This is the way it was implemented in GNU split.
struct UnixFilterWriter {
    /// Running shell process
    shell_process: Child,
    file_name: OsString,
    command: String,
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
    fn new(command: &str, filepath: &OsStr) -> Result<Self> {
        let shell_process =
            Command::new(env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()))
                .arg("-c")
                .arg(command)
                .env("FILE", filepath)
                .stdin(Stdio::piped())
                .spawn()?;

        Ok(Self {
            shell_process,
            file_name: filepath.to_os_string(),
            command: command.to_string(),
        })
    }
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
    opt_filter: &Option<String>,
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
    use crate::SpliceSettings;
    use crate::ct_app;
    use crate::platform::instantiate_current_writer;
    use crate::platform::paths_refer_to_same_file;
    use std::fs;
    use std::fs::File;

    use std::path::Path;
    use tempfile::Builder;

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
