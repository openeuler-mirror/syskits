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

/// Takes a user-supplied string and tries to parse to u16 mode bitmask.
pub fn install_parse(mode_string: &str, considering_dir: bool, umask: u32) -> Result<u32, String> {
    if mode_string.chars().any(|c| c.is_ascii_digit()) {
        ct_mode::parse_numeric(0, mode_string, considering_dir)
    } else {
        let mut mode = 0;
        for part in mode_string.split(',') {
            if part.is_empty() {
                return Err(format!("invalid mode ({mode_string})"));
            }
            mode = ct_mode::parse_symbolic(mode, part, umask, considering_dir)?;
        }
        Ok(mode)
    }
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
    use super::install_parse;

    #[test]
    fn parse_symbolic_mode_list() {
        assert_eq!(install_parse("u=rw,go=r", false, 0o022), Ok(0o644));
        assert_eq!(install_parse("u=rw,go=r,a+x", false, 0o022), Ok(0o755));
    }

    #[test]
    fn reject_empty_symbolic_mode_list_entry() {
        assert!(install_parse("u=rw,", false, 0o022).is_err());
        assert!(install_parse(",u=rw", false, 0o022).is_err());
        assert!(install_parse("u=rw,,go=r", false, 0o022).is_err());
    }
}
