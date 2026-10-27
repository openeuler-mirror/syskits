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
    use std::os::unix::ffi::OsStrExt;

    std::io::stderr()
        .lock()
        .write_all(program_name.as_bytes())
        .and_then(|_| std::io::stderr().lock().write_all(b": "))
        .and_then(|_| std::io::stderr().lock().write_all(message.as_bytes()))
        .and_then(|_| std::io::stderr().lock().write_all(b"\n"))
        .unwrap();
}

fn write_usage(program_name: &std::ffi::OsStr) {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;

    std::io::stderr()
        .lock()
        .write_all(b"Try '")
        .and_then(|_| std::io::stderr().lock().write_all(program_name.as_bytes()))
        .and_then(|_| {
            std::io::stderr()
                .lock()
                .write_all(b" --help' for more information.\n")
        })
        .unwrap();
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
        .unwrap_or_else(|| std::ffi::OsStr::new("logname"));
    let tool = ct_logname::Logname;
    let result = tool.execute(&args);
    let code = match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(err) => {
            let message = err.to_string();
            if !message.is_empty() {
                write_error(program_name, &message);
            }
            if err.usage() {
                write_usage(program_name);
            }
            err.code()
        }
    };

    if let Err(error) = std::io::stdout().flush() {
        eprintln!("Error flushing stdout: {error}");
    }

    std::process::exit(code);
}
