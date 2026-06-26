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

use cpp::cpp;
use libc::{c_char, c_int, fileno, size_t, FILE, _IOFBF, _IOLBF, _IONBF};
use std::env;
use std::ptr;

// 使用 CPP 宏定义 C++ 代码块，与 C 标准库进行交互
cpp! {{
    #include <cstdio>

    extern "C" {
        void __stdbuf(void);

        // 构造函数，在库加载时自动调用 __stdbuf
        void __attribute((constructor))
        __stdbuf_init(void) {
            __stdbuf();
        }

        // 获取标准输入流的函数
        FILE *__stdbuf_get_stdin() { return stdin; }
        // 获取标准输出流的函数
        FILE *__stdbuf_get_stdout() { return stdout; }
        // 获取标准错误流的函数
        FILE *__stdbuf_get_stderr() { return stderr; }
    }
}}

// 标记外部函数块为不安全
// 声明从 C++ 代码块导出的函数
unsafe extern "C" {
    fn __stdbuf_get_stdin() -> *mut FILE;
    fn __stdbuf_get_stdout() -> *mut FILE;
    fn __stdbuf_get_stderr() -> *mut FILE;
}

/// 设置流缓冲区模式和大小
///
/// # 参数
/// * `stream` - 要设置缓冲的文件流指针
/// * `value` - 缓冲模式字符串，可以是：
///   - "0": 无缓冲
///   - "L": 行缓冲
///   - 数字字符串: 完全缓冲，指定大小（字节）
fn set_buffer(stream: *mut FILE, value: &str) {
    // 根据输入值确定缓冲模式和大小
    let (mode, size): (c_int, size_t) = match value {
        // 无缓冲模式
        "0" => (_IONBF, 0_usize),
        // 行缓冲模式
        "L" => (_IOLBF, 0_usize),
        // 完全缓冲模式，使用指定大小
        input => {
            let buff_size: usize = match input.parse() {
                Ok(num) => num,
                Err(_) => {
                    // 解析缓冲区大小失败时输出错误并退出
                    eprintln!("failed to allocate a {} byte stdio buffer", value);
                    std::process::exit(1);
                }
            };
            (_IOFBF, buff_size as size_t)
        }
    };
    let res: c_int;
    unsafe {
        // 使用空指针作为缓冲区，让系统自动分配
        let buffer: *mut c_char = ptr::null_mut();
        assert!(buffer.is_null());
        // 调用 C 函数设置缓冲区
        res = libc::setvbuf(stream, buffer, mode, size);
    }
    // 检查设置是否成功
    if res != 0 {
        eprintln!(
            "could not set buffering of {} to mode {}",
            unsafe { fileno(stream) },
            mode
        );
    }
}

/// 主要函数，设置标准输入、输出和错误流的缓冲模式
///
/// 从环境变量读取缓冲设置并应用到相应的标准流
///
/// # 安全性
/// 此函数与 C FFI 交互以修改标准 IO 缓冲。
/// 它应该只在 stdbuf 实用程序预期的上下文中调用。
/// 
/// # Safety
/// 此函数使用 unsafe 代码与 C 标准库进行交互，修改标准 IO 流的缓冲设置。
/// 调用此函数可能导致未定义行为，如果：
/// - 在无效的上下文中调用
/// - 环境变量包含不正确的值
/// - 在不支持的平台上使用
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdbuf() {
    // 设置标准错误流的缓冲
    if let Ok(val) = env::var("_STDBUF_E") {
        unsafe {
            set_buffer(__stdbuf_get_stderr(), &val);
        }
    }
    // 设置标准输入流的缓冲
    if let Ok(val) = env::var("_STDBUF_I") {
        unsafe {
            set_buffer(__stdbuf_get_stdin(), &val);
        }
    }
    // 设置标准输出流的缓冲
    if let Ok(val) = env::var("_STDBUF_O") {
        unsafe {
            set_buffer(__stdbuf_get_stdout(), &val);
        }
    }
}
