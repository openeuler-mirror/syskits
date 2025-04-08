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
//!
//! 使用 [ct_mute_set_panic_hook] 函数来静默由管道破裂错误导致的恐慌。这种情况可能发生在生产进程仍在生成数据时，消费进程终止并关闭管道。例如，
//!
//! ```sh
//! $ seq inf | head -n 1
//! ```

use std::panic;
use std::panic::PanicHookInfo;

/// 判断一个恐慌是否是由管道破裂（SIGPIPE）错误导致的。
fn ct_pipe_states(msg: &PanicHookInfo) -> bool {
    msg.payload()
        .downcast_ref::<String>()
        .is_some_and(|message| message.contains("BrokenPipe") || message.contains("Broken pipe"))
}

/// 当由于管道破裂错误发生恐慌时，无错误地终止程序。
pub fn ct_mute_set_panic_hook() {
    // 获取当前全局恐慌钩子
    let ct_previous_hook = panic::take_hook();

    // 创建一个忽略"broken pipe"恐慌的新恐慌钩子
    let ct_current_hook = Box::new(move |info: &PanicHookInfo| {
        if !ct_pipe_states(info) {
            // 如果不是由管道破裂导致的恐慌，则调用原始钩子
            ct_previous_hook(info);
        }
    });

    // 将新钩子设为全局恐慌钩子
    panic::set_hook(ct_current_hook);
}

#[cfg(test)]
mod tests {
    use super::*;
    // use std::panic::catch_unwind;
    use std::panic::AssertUnwindSafe;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn test_mute_sigpipe() {
        let did_panic = Arc::new(AtomicBool::new(false));
        let did_panic_clone = Arc::clone(&did_panic);

        // 设置自定义钩子
        ct_mute_set_panic_hook();

        // 模拟带有"broken pipe"消息的恐慌
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            std::panic::set_hook(Box::new(move |_| {
                did_panic_clone.store(true, Ordering::SeqCst);
            }));
            panic!("broken pipe");
        }));

        assert!(result.is_err()); //预期会发生一次恐慌

        // 测试后重置恐慌钩子为默认值以清理
        let _ = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
    }
}
