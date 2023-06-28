/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

pub fn main() {
    if let Ok(profile) = env::var("PROFILE") {
        println!("cargo:rustc-cfg=build={profile:?}");
    }

    // 特征名前缀常量定义
    // 对于正在构建的包的每个激活特性，此环境变量会让<name>功能名称存在，名称的-会转换成_.
    const CT_ENV_FEATURE_PREFIX: &str = "CARGO_FEATURE_"; // 例如: CARGO_FEATURE_CAT=1
    const CT_FEATURE_PREFIX: &str = "feat_";
    const CT_OVERRIDE_PREFIX: &str = "ct_";

    let out = env::var("OUT_DIR").unwrap();

    let mut crates_vec = Vec::new();
    for (key, val) in env::vars() {
        if val == "1" && key.starts_with(CT_ENV_FEATURE_PREFIX) {
            let crates_feature = key[CT_ENV_FEATURE_PREFIX.len()..].to_lowercase();
            // 允许此项，因为我们在注释中包含了大量信息
            #[allow(clippy::match_same_arms)]
            match crates_feature.as_ref() {
                "default" | "unix" | "windows" | "selinux" | "zip" => continue, // 常见的标准特征名跳过
                "nightly" | "test_unimplemented" => continue, // 本地特定的自定义特征名跳过
                "ctdoc" => continue,                          // 不是一个工具
                s if s.starts_with(CT_FEATURE_PREFIX) => continue, // 以feat_开头的包特征集跳过
                _ => crates_vec.push(crates_feature),         // 记录工具特征名， 例如:ls
            }
        }
    }
    crates_vec.sort();

    let mut app_map_file = File::create(Path::new(&out).join("syskits_app_map.rs")).unwrap();

    // 创建实用工具映射的类型和函数
    app_map_file
        .write_all(
            "type AppMap<T> = phf::OrderedMap<&'static str, (fn(T) -> i32, fn() -> Command)>;\n\
          \n\
          #[allow(clippy::too_many_lines)]
          fn util_map<T: ctcore::Args>() -> AppMap<T> {\n"
                .as_bytes(),
        )
        .unwrap();

    let mut phf_app_map = phf_codegen::OrderedMap::<&str>::new();
    // 为每个crate构建函数映射
    for crates in &crates_vec {
        let map_value = format!("({crates}::ctmain, {crates}::ct_app)");
        match crates.as_ref() {
            app if app.starts_with(CT_OVERRIDE_PREFIX) => {
                // 如果以 'ct_' 开头，则去掉ct_
                phf_app_map.entry(&app[CT_OVERRIDE_PREFIX.len()..], &map_value);
            }
            "false" | "true" => {
                // 特殊处理 'false' 和 'true'
                phf_app_map.entry(crates, &format!("(r#{crates}::ctmain, r#{crates}::ct_app)"));
            }
            _ => {
                phf_app_map.entry(crates, &map_value);
            }
        }
    }
    write!(app_map_file, "{}", phf_app_map.build()).unwrap();
    app_map_file.write_all(b"\n}\n").unwrap();

    app_map_file.flush().unwrap();
}
