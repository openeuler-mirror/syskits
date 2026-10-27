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

fn write_error(program_name: &std::ffi::OsStr, message: &str) {
    use std::io::Write;

    let mut stderr = std::io::stderr().lock();
    let _ = stderr
        .write_all(program_name.as_encoded_bytes())
        .and_then(|_| stderr.write_all(b": "))
        .and_then(|_| stderr.write_all(message.as_bytes()))
        .and_then(|_| stderr.write_all(b"\n"));
}

fn write_usage(program_name: &std::ffi::OsStr) {
    use std::io::Write;

    let mut stderr = std::io::stderr().lock();
    let _ = stderr
        .write_all(b"Try '")
        .and_then(|_| stderr.write_all(program_name.as_encoded_bytes()))
        .and_then(|_| stderr.write_all(b" --help' for more information.\n"));
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    #[link_name = "stdout"]
    static mut C_STDOUT: *mut ctcore::libc::FILE;
    #[link_name = "stderr"]
    static mut C_STDERR: *mut ctcore::libc::FILE;
}

#[cfg(target_os = "linux")]
fn close_standard_output() -> std::io::Result<()> {
    if unsafe { ctcore::libc::fclose(C_STDOUT) } == ctcore::libc::EOF {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn close_standard_error() -> std::io::Result<()> {
    if unsafe { ctcore::libc::fclose(C_STDERR) } == ctcore::libc::EOF {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn close_standard_output() -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn close_standard_error() -> std::io::Result<()> {
    Ok(())
}

pub fn main() {
    use ctcore::Tool;
    use std::io::Write;

    ctcore::ct_panic::ct_mute_set_panic_hook();
    ctcore::ct_ensure_standard_fds();

    let args = ctcore::ct_os_args().collect::<Vec<_>>();
    let program_name = args
        .first()
        .map(AsRef::as_ref)
        .unwrap_or_else(|| std::ffi::OsStr::new("tsort"));
    let tool = ct_tsort::Tsort;
    let result = tool.execute(&args);
    let mut write_error_reported = false;
    let mut code = match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(error) => {
            let message = error.to_string();
            write_error_reported = ct_tsort::is_tsort_write_error_message(&message);
            if !message.is_empty() {
                write_error(program_name, &message);
            }
            if error.usage() {
                write_usage(program_name);
            }
            error.code()
        }
    };

    if let Err(error) = std::io::stdout().flush() {
        if !write_error_reported {
            write_error(program_name, &ct_tsort::tsort_write_error_message(&error));
            write_error_reported = true;
        }
        code = 1;
    }

    if let Err(error) = close_standard_output() {
        if !write_error_reported {
            write_error(program_name, &ct_tsort::tsort_write_error_message(&error));
        }
        code = 1;
    }

    if close_standard_error().is_err() {
        code = 1;
    }

    std::process::exit(code);
}
