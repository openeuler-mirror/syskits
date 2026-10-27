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

fn write_stderr(args: std::fmt::Arguments<'_>) {
    use std::io::Write;

    let _ = std::io::stderr().lock().write_fmt(args);
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    #[link_name = "stdout"]
    static mut C_STDOUT: *mut ctcore::libc::FILE;
}

#[cfg(target_os = "linux")]
fn close_standard_output() -> std::io::Result<()> {
    if unsafe { ctcore::libc::fclose(C_STDOUT) } == ctcore::libc::EOF {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn close_standard_output() -> std::io::Result<()> {
    Ok(())
}

pub fn main() {
    use ctcore::Tool;
    use std::io::Write;

    ctcore::ct_panic::ct_mute_set_panic_hook();
    ctcore::ct_ensure_standard_fds();
    let _ = ctcore::ct_signals::enable_pipe_errors();

    let args = ctcore::ct_os_args().collect::<Vec<_>>();
    let tool = ct_fold::Fold;
    let result = tool.execute(&args);
    let mut write_error_reported = false;
    let mut code = match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(error) => {
            let message = error.to_string();
            write_error_reported = ct_fold::is_fold_write_error_message(&message);
            if !message.is_empty() {
                write_stderr(format_args!("{}: {message}\n", ctcore::ct_util_name()));
            }
            if error.usage() {
                write_stderr(format_args!(
                    "Try '{} --help' for more information.\n",
                    ctcore::ct_help_utility_name()
                ));
            }
            error.code()
        }
    };

    if let Err(error) = std::io::stdout().flush() {
        if !write_error_reported {
            write_stderr(format_args!(
                "{}: {}\n",
                ctcore::ct_util_name(),
                ct_fold::fold_write_error_message(&error)
            ));
            write_error_reported = true;
        }
        code = 1;
    }

    if let Err(error) = close_standard_output() {
        if !write_error_reported {
            write_stderr(format_args!(
                "{}: {}\n",
                ctcore::ct_util_name(),
                ct_fold::fold_write_error_message(&error)
            ));
        }
        code = 1;
    }

    std::process::exit(code);
}
