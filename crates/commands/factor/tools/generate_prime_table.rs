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

//! 素数表生成工具
//!
//! 本工具用于生成预计算的素数逆元表，并将其写入table.rs文件。
//! 该表用于加速因式分解过程中的试除法步骤。
//!
//! 使用方法：
//! ```
//! cargo run --bin generate_prime_table
//! ```

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

// 导入模块
#[path = "../src/numeric.rs"]
mod numeric;
// 不再使用未使用的导入
// use numeric::{Int, DoubleInt};

mod sieve;
use sieve::Sieve;

// 常量定义
const DEFAULT_SIZE: usize = 320; // 生成的素数数量
const MAX_WIDTH: usize = 102; // 每行最大字符数
const TABLE_START_MARKER: &str = "// BEGIN GENERATED PRIME TABLE";
const TABLE_END_MARKER: &str = "// END GENERATED PRIME TABLE";

// 为了保持API兼容性，提供一个gcd函数
#[allow(dead_code)]
fn gcd(a: u64, b: u64) -> u64 {
    use gcd::Gcd;
    a.gcd(b)
}

fn main() {
    let start_time = Instant::now();
    println!("开始生成素数表...");

    // 测试calculate_modular_inverse函数是否每次都返回相同的结果
    let test_primes = [3, 5, 7, 11, 13];
    println!("测试calculate_modular_inverse函数:");
    for &p in &test_primes {
        let inv = calculate_modular_inverse(p);
        println!("{p}的逆元: {inv}");
    }

    // 测试Sieve::odd_primes()函数是否每次都返回相同的素数序列
    println!("测试Sieve::odd_primes()函数:");
    let primes = Sieve::odd_primes()
        .take(10)
        .filter(|&p| is_prime(p))
        .collect::<Vec<_>>();
    println!("前10个奇素数:");
    for p in primes {
        print!("{p} ");
    }
    println!();

    // 查找table.rs文件
    let table_path = find_table_file();

    // 生成素数表内容
    let prime_table = generate_prime_table();

    // 更新table.rs文件
    update_table_file(&table_path, &prime_table);

    let duration = start_time.elapsed();
    println!("素数表生成完成，用时: {duration:?}");
}

/// 查找table.rs文件
fn find_table_file() -> std::path::PathBuf {
    // 尝试不同的可能路径
    let possible_paths = [
        Path::new("crates/commands/factor/src/table.rs"),
        Path::new("src/table.rs"),
        Path::new("../src/table.rs"),
    ];

    for path in &possible_paths {
        if path.exists() {
            println!("找到table.rs文件: {}", path.display());
            return path.to_path_buf();
        }
    }

    // 如果找不到文件，提示用户并退出
    eprintln!("错误: 找不到table.rs文件");
    eprintln!("请确保您在正确的目录中运行此工具");
    std::process::exit(1);
}

/// 更新table.rs文件
fn update_table_file(table_path: &Path, prime_table: &str) {
    // 读取现有的table.rs文件内容
    let table_content = match fs::read_to_string(table_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("读取table.rs文件时出错: {e}");
            std::process::exit(1);
        }
    };

    // 查找标记之间的内容并替换
    if table_content.contains(TABLE_START_MARKER) && table_content.contains(TABLE_END_MARKER) {
        // 如果找到标记，替换它们之间的内容
        let start_pos = table_content.find(TABLE_START_MARKER).unwrap();
        let end_pos = table_content.find(TABLE_END_MARKER).unwrap() + TABLE_END_MARKER.len();

        let new_content = format!(
            "{}{}{}{}{}",
            &table_content[..start_pos],
            TABLE_START_MARKER,
            prime_table,
            TABLE_END_MARKER,
            &table_content[end_pos..]
        );

        // 写入更新后的内容
        let mut file = match File::create(table_path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("创建文件时出错: {e}");
                std::process::exit(1);
            }
        };

        if let Err(e) = file.write_all(new_content.as_bytes()) {
            eprintln!("写入文件时出错: {e}");
            std::process::exit(1);
        }

        println!("成功更新素数表: {}", table_path.display());
    } else {
        // 如果没有找到标记，提示用户添加标记
        eprintln!("在table.rs中找不到标记");
        eprintln!("请在table.rs中添加以下标记，以指定素数表的插入位置:");
        eprintln!("{TABLE_START_MARKER}");
        eprintln!("{TABLE_END_MARKER}");
        std::process::exit(1);
    }
}

/// 生成素数表内容
fn generate_prime_table() -> String {
    println!("生成素数逆元表...");

    let mut output = String::new();
    output.push_str("\n\n#[allow(clippy::unreadable_literal)]\npub const PRIME_INVERSIONS_U64: &[(u64, u64, u64)] = &[\n    ");

    let mut cols = 4; // 初始列位置
    let mut count = 0;

    // 生成真正的素数序列
    let mut real_primes = Vec::new();
    let mut prime_iter = Sieve::odd_primes();

    // 收集足够数量的真正素数
    while real_primes.len() < DEFAULT_SIZE {
        if let Some(p) = prime_iter.next() {
            if is_prime(p) {
                real_primes.push(p);
            }
        } else {
            break; // 如果迭代器结束，退出循环
        }
    }

    // 确保至少有一个素数
    if real_primes.is_empty() {
        eprintln!("错误: 无法生成素数");
        std::process::exit(1);
    }

    // 获取下一个素数作为NEXT_PRIME
    let mut next_prime = 0;
    for p in prime_iter {
        if is_prime(p) {
            next_prime = p;
            break;
        }
    }

    // 如果找不到下一个素数，使用最后一个素数加2
    if next_prime == 0 {
        next_prime = real_primes.last().unwrap() + 2;
    }

    // 生成素数逆元表
    for &p in real_primes.iter() {
        // 计算逆元和上限
        let inv = calculate_modular_inverse(p);
        let ceil = u64::MAX / p;

        // 格式化条目
        let entry = format!("({p}, {inv}, {ceil}),");

        // 添加到输出，考虑行宽
        if cols + entry.len() > MAX_WIDTH {
            output.push_str(&format!("\n    {entry}"));
            cols = 4 + entry.len();
        } else {
            output.push_str(&format!(" {entry}"));
            cols += 1 + entry.len();
        }

        count += 1;

        // 每100个素数显示一次进度
        if count % 100 == 0 {
            println!("已处理 {count} 个素数...");
        }
    }

    // 添加下一个素数常量
    output.push_str(&format!(
        "\n];\n\n#[allow(dead_code)]\npub const NEXT_PRIME: u64 = {next_prime};\n\n"
    ));

    println!("共生成 {count} 个素数的逆元表");
    println!("下一个素数: {next_prime}");
    output
}

/// 简单的素数检测函数
fn is_prime(n: u64) -> bool {
    if n <= 1 {
        return false;
    }
    if n <= 3 {
        return true;
    }
    if n % 2 == 0 || n % 3 == 0 {
        return false;
    }

    let mut i = 5;
    while i * i <= n {
        if n % i == 0 || n % (i + 2) == 0 {
            return false;
        }
        i += 6;
    }

    true
}

/// 计算模逆元
///
/// 计算a在模2^64下的乘法逆元
/// 对于奇数a，a * a^(-1) ≡ 1 (mod 2^64)
fn calculate_modular_inverse(a: u64) -> u64 {
    debug_assert!(a % 2 == 1, "输入必须是奇数");

    // 对于模2^64的情况，num-modular库没有直接的支持
    // 我们使用牛顿迭代法，这是计算2的幂模数下逆元的标准方法

    // 初始值：a在模2^2下的逆元
    // 对于任何奇数a，a在模4下的逆元就是a本身
    let mut x = a;

    // 牛顿迭代
    // 每次迭代将模数翻倍：2^2 -> 2^4 -> 2^8 -> 2^16 -> 2^32 -> 2^64
    for _ in 0..5 {
        // 牛顿迭代公式：x_{n+1} = x_n * (2 - a * x_n)
        // 使用wrapping_mul来模拟在2^64下的乘法
        x = x.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(x)));
    }

    x
}
