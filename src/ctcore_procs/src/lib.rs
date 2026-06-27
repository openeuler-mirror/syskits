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

use proc_macro::TokenStream;
use quote::quote;

#[proc_macro_attribute]
pub fn main(_ct_args: TokenStream, ct_stream: TokenStream) -> TokenStream {
    // 将输入TokenStream解析为proc_macro2::TokenStream
    let my_stream = proc_macro2::TokenStream::from(ct_stream);

    // 生成新的main函数
    let ct_main = quote!(
        pub fn ctmain(args: impl ctcore::Args) -> i32 {
            #my_stream
            let result = ctmain(args);
            match result {
                Ok(()) => ctcore::ct_error::get_ct_exit_code(),
                Err(err) => {
                    let s_err = format!("{}", err);
                    if !s_err.is_empty() {
                        ctcore::ct_show_error!("{}", s_err);
                    }
                    if err.usage() {
                        eprintln!("Try '{} --help' for more information.", ctcore::ct_execute_phrase());
                    }
                    err.code()
                }
            }
        }
    );

    // 将生成的新main函数转换为TokenStream
    TokenStream::from(ct_main)
}
