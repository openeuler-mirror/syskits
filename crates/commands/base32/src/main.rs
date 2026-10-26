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

pub fn main() {
    use ctcore::Tool;
    use std::io::Write;

    ctcore::ct_panic::ct_mute_set_panic_hook();
    ctcore::ct_ensure_standard_fds();

    let args = ctcore::ct_os_args().collect::<Vec<_>>();
    let tool = ct_base32::Base32::default();
    let result = tool.execute(&args);
    let mut code = match result {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(err) => {
            let message = err.to_string();
            if !message.is_empty() {
                write_stderr(format_args!(
                    "{}: {message}\n",
                    ctcore::ct_util_name()
                ));
            }
            if err.usage() {
                write_stderr(format_args!(
                    "Try '{} --help' for more information.\n",
                    ctcore::ct_help_utility_name()
                ));
            }
            err.code()
        }
    };

    if let Err(err) = std::io::stdout().flush() {
        write_stderr(format_args!(
            "{}: write error: {}\n",
            ctcore::ct_util_name(),
            ctcore::ct_error::strip_errno(&err)
        ));
        code = 1;
    }

    std::process::exit(code);
}
