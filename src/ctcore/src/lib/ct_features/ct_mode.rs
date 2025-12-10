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

//! Set of functions to parse modes

use libc::{mode_t, umask, S_IRGRP, S_IROTH, S_IRUSR, S_IWGRP, S_IWOTH, S_IWUSR};

pub fn parse_numeric(fperm: u32, mut mode: &str, considering_dir: bool) -> Result<u32, String> {
    let (op, pos) = parse_op(mode).map_or_else(|_| (None, 0), |(op, pos)| (Some(op), pos));
    mode = mode[pos..].trim();
    let change = if mode.is_empty() {
        0
    } else {
        u32::from_str_radix(mode, 8).map_err(|e| e.to_string())?
    };
    if change > 0o7777 {
        Err(format!("mode is too large ({change} > 7777"))
    } else {
        Ok(match op {
            Some('+') => fperm | change,
            Some('-') => fperm & !change,
            // 如果这是一个目录，我们会保留setgid和setuid位，
            // 除非模式包含5个或更多的八进制数字或模式为“=”
            None if considering_dir && mode.len() < 5 => change | (fperm & (0o4000 | 0o2000)),
            None | Some('=') => change,
            Some(_) => unreachable!(),
        })
    }
}

pub fn parse_symbolic(
    mut fperm: u32,
    mut mode: &str,
    umask: u32,
    considering_dir: bool,
) -> Result<u32, String> {
    let (mask, pos) = parse_levels(mode);
    if pos == mode.len() {
        return Err(format!("invalid mode ({mode})"));
    }
    let respect_umask = pos == 0;
    mode = &mode[pos..];
    while !mode.is_empty() {
        let (op, pos) = parse_op(mode)?;
        mode = &mode[pos..];
        let (mut srwx, pos) = parse_change(mode, fperm, considering_dir);
        if respect_umask {
            srwx &= !umask;
        }
        mode = &mode[pos..];
        match op {
            '+' => fperm |= srwx & mask,
            '-' => fperm &= !(srwx & mask),
            '=' => {
                if considering_dir {
                    // 保留目录的setgid和setuid位
                    srwx |= fperm & (0o4000 | 0o2000);
                }
                fperm = (fperm & !mask) | (srwx & mask);
            }
            _ => unreachable!(),
        }
    }
    Ok(fperm)
}

fn parse_levels(mode: &str) -> (u32, usize) {
    let mut mask = 0;
    let mut pos = 0;
    for ch in mode.chars() {
        mask |= match ch {
            'u' => 0o4700,
            'g' => 0o2070,
            'o' => 0o1007,
            'a' => 0o7777,
            _ => break,
        };
        pos += 1;
    }
    if pos == 0 {
        mask = 0o7777; // default to 'a'
    }
    (mask, pos)
}

fn parse_op(mode: &str) -> Result<(char, usize), String> {
    let ch = mode
        .chars()
        .next()
        .ok_or_else(|| "unexpected end of mode".to_owned())?;
    match ch {
        '+' | '-' | '=' => Ok((ch, 1)),
        _ => Err(format!(
            "invalid operator (expected +, -, or =, but found {ch})"
        )),
    }
}

fn parse_change(mode: &str, fperm: u32, considering_dir: bool) -> (u32, usize) {
    let mut srwx = 0;
    let mut pos = 0;
    for ch in mode.chars() {
        match ch {
            'r' => srwx |= 0o444,
            'w' => srwx |= 0o222,
            'x' => srwx |= 0o111,
            'X' => {
                if considering_dir || (fperm & 0o0111) != 0 {
                    srwx |= 0o111;
                }
            }
            's' => srwx |= 0o4000 | 0o2000,
            't' => srwx |= 0o1000,
            'u' => srwx = (fperm & 0o700) | ((fperm >> 3) & 0o070) | ((fperm >> 6) & 0o007),
            'g' => srwx = ((fperm << 3) & 0o700) | (fperm & 0o070) | ((fperm >> 3) & 0o007),
            'o' => srwx = ((fperm << 6) & 0o700) | ((fperm << 3) & 0o070) | (fperm & 0o007),
            _ => break,
        };
        if ch == 'u' || ch == 'g' || ch == 'o' {
            // 符号模式只允许perms为'u'、'g'或'o'中的单个字母，因此它必须是第一个字符，否则就是意外的
            if pos != 0 {
                break;
            }
            pos = 1;
            break;
        }
        pos += 1;
    }
    if pos == 0 {
        srwx = 0;
    }
    (srwx, pos)
}

#[allow(clippy::unnecessary_cast)]
pub fn parse_mode(mode: &str) -> Result<mode_t, String> {
    #[cfg(all(
        not(target_os = "freebsd"),
        not(target_vendor = "apple"),
        not(target_os = "android")
    ))]
    let fperm = S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH;
    #[cfg(any(target_os = "freebsd", target_vendor = "apple", target_os = "android"))]
    let fperm = (S_IRUSR | S_IWUSR | S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH) as u32;

    let result = if mode.chars().any(|c| c.is_ascii_digit()) {
        parse_numeric(fperm as u32, mode, true)
    } else {
        parse_symbolic(fperm as u32, mode, get_umask(), true)
    };
    result.map(|mode| mode as mode_t)
}

pub fn get_umask() -> u32 {
    // 没有一种便携式方法可以在不更改umask的情况下读取它。
    // 我们必须替换它，然后快速将其恢复原状，希望在这之前不会影响到其他线程。
    // 在现代Linux内核中，当前umask可以从/proc/self/status中读取。但这很麻烦。
    // 安全性：umask总是成功，并且不操作内存。可能存在竞态条件，但它不能违反Rust的保证。
    let mask = unsafe { umask(0) };
    unsafe { umask(mask) };
    #[cfg(all(
        not(target_os = "freebsd"),
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "redox")
    ))]
    return mask;
    #[cfg(any(
        target_os = "freebsd",
        target_vendor = "apple",
        target_os = "android",
        target_os = "redox"
    ))]
    return mask as u32;
}

// 遍历'args'并删除与MODE关联的首个前缀'-'
// 例如："chmod -v -xw -R FILE" -> "chmod -v xw -R FILE"

pub fn strip_minus_from_mode(args: &mut [String]) -> bool {
    for arg in args.iter_mut() {
        if arg == "--" {
            break;
        }

        // 检查参数是否以减号（-）开头
        if arg.starts_with('-') {
            // 获取减号后的第一个字符
            let second_char_opt = arg.chars().nth(1);

            // 如果存在第二个字符，检查其有效性
            if let Some(second_char) = second_char_opt {
                // mode标志的有效字符
                const VALID_CHARS: &[char] = &[
                    'r', 'w', 'x', 'X', 's', 't', 'u', 'g', 'o', '0', '1', '2', '3', '4', '5', '6',
                    '7',
                ];

                // 如果第二个字符有效，则移除减号并返回true
                if VALID_CHARS.contains(&second_char) {
                    arg.replace_range(0..1, "");
                    return true;
                }
            }
        }
    }

    //没有发现有效标志，返回false
    false
}

#[cfg(test)]
mod test {

    #[test]
    fn symbolic_modes() {
        assert_eq!(super::parse_mode("u+x").unwrap(), 0o766);
        assert_eq!(
            super::parse_mode("+x").unwrap(),
            if crate::ct_os::ct_wsl_1() {
                0o776
            } else {
                0o777
            }
        );
        assert_eq!(super::parse_mode("a-w").unwrap(), 0o444);
        assert_eq!(super::parse_mode("g-r").unwrap(), 0o626);
    }

    #[test]
    fn numeric_modes() {
        assert_eq!(super::parse_mode("644").unwrap(), 0o644);
        assert_eq!(super::parse_mode("+100").unwrap(), 0o766);
        assert_eq!(super::parse_mode("-4").unwrap(), 0o662);
    }
    #[test]
    fn test_parse_numeric() {
        // Test case 1: Valid input with no operator
        let fperm = 0o644;
        let mode = "400";
        let considering_dir = false;
        let expected = Ok(0o400);
        assert_eq!(super::parse_numeric(fperm, mode, considering_dir), expected);

        // Test case 2: Valid input with '+' operator
        let fperm = 0o644;
        let mode = "+400";
        let considering_dir = false;
        let expected = Ok(0o644);
        assert_eq!(super::parse_numeric(fperm, mode, considering_dir), expected);

        // Test case 3: Valid input with '-' operator
        let fperm = 0o644;
        let mode = "-400";
        let considering_dir = false;
        let expected = Ok(0o244);
        assert_eq!(super::parse_numeric(fperm, mode, considering_dir), expected);

        // Test case 4: Valid input with '=' operator
        let fperm = 0o644;
        let mode = "=400";
        let considering_dir = false;
        let expected = Ok(0o400);
        assert_eq!(super::parse_numeric(fperm, mode, considering_dir), expected);
    }

    #[test]
    fn test_parse_symbolic() {
        // Test case 1: Valid input with '+'
        let fperm = 0o644;
        let mode = "u+r";
        let umask = 0o022;
        let considering_dir = false;
        let expected = Ok(0o644);
        assert_eq!(
            super::parse_symbolic(fperm, mode, umask, considering_dir),
            expected
        );

        // Test case 2: Valid input with '-'
        let fperm = 0o777;
        let mode = "o-w";
        let umask = 0o022;
        let considering_dir = false;
        let expected = Ok(0o775);
        assert_eq!(
            super::parse_symbolic(fperm, mode, umask, considering_dir),
            expected
        );

        // Test case 3: Valid input with '='
        let fperm = 0o644;
        let mode = "g=r";
        let umask = 0o022;
        let considering_dir = false;
        let expected = Ok(0o644);
        assert_eq!(
            super::parse_symbolic(fperm, mode, umask, considering_dir),
            expected
        );

        // Test case 4: Invalid input with empty mode
        let fperm = 0o644;
        let mode = "";
        let umask = 0o022;
        let considering_dir = false;
        let expected = Err("invalid mode ()".to_owned());
        assert_eq!(
            super::parse_symbolic(fperm, mode, umask, considering_dir),
            expected
        );
    }

    #[test]
    fn test_parse_levels() {
        // Test case 1: Valid input with 'u'
        let mode = "u";
        let expected = (0o4700, 1);
        assert_eq!(super::parse_levels(mode), expected);

        // Test case 2: Valid input with 'ugo'
        let mode = "ugo";
        let expected = (0o7777, 3);
        assert_eq!(super::parse_levels(mode), expected);

        // Test case 3: Valid input with 'a'
        let mode = "a";
        let expected = (0o7777, 1);
        assert_eq!(super::parse_levels(mode), expected);

        // Test case 4: Invalid input with invalid character
        let mode = "x";
        let expected = (0o7777, 0);
        assert_eq!(super::parse_levels(mode), expected);
    }

    #[test]
    fn test_parse_op() {
        // Test case 1: Valid input with '+'
        let mode = "+";
        let expected = Ok(('+', 1));
        assert_eq!(super::parse_op(mode), expected);

        // Test case 2: Valid input with '-'
        let mode = "-";
        let expected = Ok(('-', 1));
        assert_eq!(super::parse_op(mode), expected);

        // Test case 3: Valid input with '='
        let mode = "=";
        let expected = Ok(('=', 1));
        assert_eq!(super::parse_op(mode), expected);
    }
}
