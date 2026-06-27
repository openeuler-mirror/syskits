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

use std::env;
use std::fs;
use std::path::Path;

// 非Windows平台定义动态库扩展名
#[cfg(not(target_os = "windows"))]
mod platform {
    pub const DYLIB_EXT: &str = ".so";
}

// Windows平台定义动态库扩展名
#[cfg(target_os = "windows")]
mod platform {
    pub const DYLIB_EXT: &str = ".dll";
}

fn main() {
    // 获取编译输出目录
    let out_dir = env::var("OUT_DIR").unwrap();
    let mut target_dir = Path::new(&out_dir);

    // 根据不同的构建方式，目录结构会有所不同。
    // 这里的代码适用于以下几种情况：
    // - cargo run
    // - cross run
    // - cargo install --git
    // - cargo publish --dry-run
    // - cargo llvm-cov
    //
    // 目标是找到我们要安装的目录，但这取决于构建方法，这是很烦人的。
    // 另外，环境变量中的配置文件只能是"debug"或"release"，而不能是自定义
    // 配置文件名称，所以我们必须使用target目录中的目录名作为配置文件名称。

    // 从输出路径中提取配置文件名称（debug/release）
    let profile_name = out_dir
        .split(std::path::MAIN_SEPARATOR)
        .nth_back(3)
        .unwrap();

    // 向上遍历目录直到找到"target"目录或"cargo-install"开头的目录
    let mut name = target_dir.file_name().unwrap().to_string_lossy();
    while name != "target" && !name.starts_with("cargo-install") {
        target_dir = target_dir.parent().unwrap();
        name = target_dir.file_name().unwrap().to_string_lossy();
    }

    // 构建完整的依赖目录路径
    let mut dir = target_dir.to_path_buf();

    // 检查输出目录路径中是否包含llvm-cov-target
    if out_dir.contains("llvm-cov-target") {
        let llvm_cov_dir = target_dir.join("llvm-cov-target");
        if llvm_cov_dir.exists() {
            dir = llvm_cov_dir;
        }
    }

    dir.push(profile_name);
    dir.push("deps");
    let mut path = None;

    // 当运行cargo publish时，cargo会在编译后的文件名中添加哈希值。
    // 因此，直接获取liblibstdbuf.so不起作用。相反，我们使用模式
    // "liblibstdbuf*.so"（即以liblibstdbuf开头并以扩展名结尾）来查找文件。
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("liblibstdbuf") && name.ends_with(platform::DYLIB_EXT) {
            path = Some(entry.path());
        }
    }

    // 将找到的库文件复制到输出目录中，并重命名为标准的名称
    fs::copy(
        path.expect("liblibstdbuf was not found"),
        Path::new(&out_dir).join("libstdbuf.so"),
    )
    .unwrap();
}
