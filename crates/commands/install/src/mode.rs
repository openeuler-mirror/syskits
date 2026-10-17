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
#[cfg(not(windows))]
use ctcore::ct_mode;
use std::fs;
use std::path::Path;

pub const CHMOD_MODE_BITS: u32 = 0o7777;
const SET_ID_BITS: u32 = 0o6000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedMode {
    pub mode: u32,
    pub affected_bits: u32,
}

/// Parse a mode and report which bits were explicitly affected.
///
/// GNU install uses this mask when updating directories so that chmod does not
/// clear set-ID or permission bits omitted by the mode expression.
pub fn install_parse_with_bits(
    mode_string: &str,
    considering_dir: bool,
    umask: u32,
) -> Result<ParsedMode, String> {
    if mode_string.chars().any(|c| c.is_ascii_digit()) {
        let mode = ct_mode::parse_numeric(0, mode_string, considering_dir)?;
        let trimmed = mode_string.trim();
        let (op, digits) = match trimmed.as_bytes().first() {
            Some(op @ (b'+' | b'-' | b'=')) => (Some(*op as char), trimmed[1..].trim()),
            _ => (None, trimmed),
        };
        let value = u32::from_str_radix(digits, 8).map_err(|e| e.to_string())?;
        let affected_bits = match op {
            Some('+') | Some('-') => value,
            Some('=') => CHMOD_MODE_BITS,
            None if considering_dir && digits.len() < 5 => {
                let mentioned = (value & SET_ID_BITS) | 0o1777;
                CHMOD_MODE_BITS & !(SET_ID_BITS & !mentioned)
            }
            None => CHMOD_MODE_BITS,
            Some(_) => unreachable!(),
        };
        Ok(ParsedMode {
            mode,
            affected_bits,
        })
    } else {
        let mut mode = 0;
        let mut affected_bits = 0;
        for part in mode_string.split(',') {
            if part.is_empty() {
                return Err(format!("invalid mode ({mode_string})"));
            }
            let parsed = parse_symbolic_with_bits(mode, part, umask, considering_dir)?;
            mode = parsed.mode;
            affected_bits |= parsed.affected_bits;
        }
        Ok(ParsedMode {
            mode,
            affected_bits,
        })
    }
}

fn parse_symbolic_with_bits(
    mut mode: u32,
    mut expression: &str,
    umask: u32,
    considering_dir: bool,
) -> Result<ParsedMode, String> {
    let (mask, pos) = parse_levels(expression);
    if pos == expression.len() {
        return Err(format!("invalid mode ({expression})"));
    }
    let respect_umask = pos == 0;
    expression = &expression[pos..];
    let mut affected_bits = 0;

    while !expression.is_empty() {
        let (op, pos) = parse_op(expression)?;
        expression = &expression[pos..];
        let (mut value, pos) = parse_change(expression, mode, considering_dir);
        expression = &expression[pos..];

        let mentioned = if mask == CHMOD_MODE_BITS {
            value
        } else {
            value & mask
        };
        let omitted = if considering_dir {
            SET_ID_BITS & !mentioned
        } else {
            0
        };
        if respect_umask {
            value &= !umask;
        }
        let change = value & mask & !omitted;

        match op {
            '+' => {
                mode |= change;
                affected_bits |= change;
            }
            '-' => {
                mode &= !change;
                affected_bits |= change;
            }
            '=' => {
                let preserved = if mask == CHMOD_MODE_BITS { 0 } else { !mask } | omitted;
                mode = (mode & preserved) | change;
                affected_bits |= CHMOD_MODE_BITS & !preserved;
            }
            _ => unreachable!(),
        }
    }

    Ok(ParsedMode {
        mode,
        affected_bits,
    })
}

fn parse_levels(mode: &str) -> (u32, usize) {
    let mut mask = 0;
    let mut pos = 0;
    for ch in mode.chars() {
        mask |= match ch {
            'u' => 0o4700,
            'g' => 0o2070,
            'o' => 0o1007,
            'a' => CHMOD_MODE_BITS,
            _ => break,
        };
        pos += 1;
    }
    if pos == 0 {
        mask = CHMOD_MODE_BITS;
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

fn parse_change(mode: &str, current_mode: u32, considering_dir: bool) -> (u32, usize) {
    let mut value = 0;
    let mut pos = 0;
    for ch in mode.chars() {
        match ch {
            'r' => value |= 0o444,
            'w' => value |= 0o222,
            'x' => value |= 0o111,
            'X' => {
                if considering_dir || current_mode & 0o111 != 0 {
                    value |= 0o111;
                }
            }
            's' => value |= SET_ID_BITS,
            't' => value |= 0o1000,
            'u' => {
                value = (current_mode & 0o700)
                    | ((current_mode >> 3) & 0o070)
                    | ((current_mode >> 6) & 0o007);
            }
            'g' => {
                value = ((current_mode << 3) & 0o700)
                    | (current_mode & 0o070)
                    | ((current_mode >> 3) & 0o007);
            }
            'o' => {
                value = ((current_mode << 6) & 0o700)
                    | ((current_mode << 3) & 0o070)
                    | (current_mode & 0o007);
            }
            _ => break,
        }
        if matches!(ch, 'u' | 'g' | 'o') {
            if pos != 0 {
                break;
            }
            return (value, 1);
        }
        pos += 1;
    }
    (value, pos)
}

/// chmod a file or directory on UNIX.
///
/// Adapted from mkdir.rs.  Handles own error printing.
///
#[cfg(target_os = "linux")]
pub fn install_chmod(path: &Path, mode: u32) -> Result<(), ()> {
    use ctcore::{ct_display::Quotable, ct_show_error};
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|err| {
        ct_show_error!("{}: chmod failed with error {}", path.maybe_quote(), err);
    })
}

/// chmod a file or directory on Windows.
///
/// Adapted from mkdir.rs.
///
#[cfg(windows)]
pub fn chmod(path: &Path, mode: u32) -> Result<(), ()> {
    // chmod on Windows only sets the readonly flag, which isn't even honored on directories
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CHMOD_MODE_BITS, ParsedMode, install_parse_with_bits};

    #[test]
    fn parse_symbolic_mode_list() {
        assert_eq!(
            install_parse_with_bits("u=rw,go=r", false, 0o022).map(|parsed| parsed.mode),
            Ok(0o644)
        );
        assert_eq!(
            install_parse_with_bits("u=rw,go=r,a+x", false, 0o022).map(|parsed| parsed.mode),
            Ok(0o755)
        );
    }

    #[test]
    fn reject_empty_symbolic_mode_list_entry() {
        assert!(install_parse_with_bits("u=rw,", false, 0o022).is_err());
        assert!(install_parse_with_bits(",u=rw", false, 0o022).is_err());
        assert!(install_parse_with_bits("u=rw,,go=r", false, 0o022).is_err());
    }

    #[test]
    fn directory_numeric_mode_preserves_unmentioned_set_id_bits() {
        assert_eq!(
            install_parse_with_bits("755", true, 0o022),
            Ok(ParsedMode {
                mode: 0o755,
                affected_bits: 0o1777,
            })
        );
        assert_eq!(
            install_parse_with_bits("2755", true, 0o022),
            Ok(ParsedMode {
                mode: 0o2755,
                affected_bits: 0o3777,
            })
        );
        assert_eq!(
            install_parse_with_bits("00755", true, 0o022),
            Ok(ParsedMode {
                mode: 0o755,
                affected_bits: CHMOD_MODE_BITS,
            })
        );
    }

    #[test]
    fn directory_symbolic_mode_only_affects_selected_permissions() {
        assert_eq!(
            install_parse_with_bits("g=rx", true, 0),
            Ok(ParsedMode {
                mode: 0o050,
                affected_bits: 0o070,
            })
        );
        assert_eq!(
            install_parse_with_bits("g-s", true, 0),
            Ok(ParsedMode {
                mode: 0,
                affected_bits: 0o2000,
            })
        );
        assert_eq!(
            install_parse_with_bits("=755", true, 0),
            Ok(ParsedMode {
                mode: 0o755,
                affected_bits: CHMOD_MODE_BITS,
            })
        );
    }
}
