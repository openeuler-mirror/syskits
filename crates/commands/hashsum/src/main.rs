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

pub fn main() {
    use ctcore::Tool;
    use std::io::Write;

    ctcore::ct_panic::ct_mute_set_panic_hook();
    ctcore::ct_ensure_standard_fds();

    let args = ctcore::ct_os_args().collect::<Vec<_>>();
    let tool = ct_hashsum::Hashsum;
    let code = match tool.execute(&args) {
        Ok(()) => ctcore::ct_error::get_ct_exit_code(),
        Err(error) => {
            let message = error.to_string();
            if !message.is_empty() {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "{}: {message}",
                    ctcore::ct_util_name()
                );
            }
            if error.usage() {
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "Try '{} --help' for more information.",
                    ctcore::ct_help_utility_name()
                );
            }
            error.code()
        }
    };

    std::process::exit(code);
}
