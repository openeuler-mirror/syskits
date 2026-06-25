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

//! Pollard's Rho 算法测试程序

use std::env;
use std::process;
use std::time::Instant;

use ct_factor::rho::find_divisor;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <number>", args[0]);
        process::exit(1);
    }

    let number: u64 = match args[1].parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("Error: '{}' is not a valid positive integer", args[1]);
            process::exit(1);
        }
    };

    println!(
        "Finding a factor of {} using Pollard's Rho algorithm...",
        number
    );

    let start = Instant::now();
    let factor = find_divisor(number);
    let duration = start.elapsed();

    println!("Found factor: {}", factor);
    println!("Time taken: {:?}", duration);

    // 验证结果
    if number % factor == 0 {
        println!("Verification: {} is indeed a factor of {}", factor, number);
    } else {
        println!("ERROR: {} is NOT a factor of {}", factor, number);
        process::exit(1);
    }
}
