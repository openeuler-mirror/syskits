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

use clap::Command;
use std::env;

pub const OBSOLETE: usize = 199209;
pub const TRADITIONAL: usize = 200112;
pub const MODERN: usize = 200809;

pub fn posixly_correct() -> bool {
    env::var_os("POSIXLY_CORRECT").is_some()
}

pub trait GnuGetoptCommandExt {
    fn gnu_getopt(self) -> Self;
    fn gnu_getopt_with_mode(self, posixly_correct: bool) -> Self;
}

impl GnuGetoptCommandExt for Command {
    fn gnu_getopt(self) -> Self {
        self.gnu_getopt_with_mode(posixly_correct())
    }

    fn gnu_getopt_with_mode(self, posixly_correct: bool) -> Self {
        self.trailing_var_arg(posixly_correct)
    }
}

pub fn ct_posix_version() -> Option<i32> {
    env::var("_POSIX2_VERSION")
        .ok()
        .and_then(|value| ct_parse_posix_version(&value))
}

/// Parse `_POSIX2_VERSION` with GNU `posix2_version` semantics.
///
/// GNU uses `strtol`, so values may have leading ASCII whitespace or a sign.
/// Values outside the `int` range saturate before callers compare them.
fn ct_parse_posix_version(value: &str) -> Option<i32> {
    let value = value.trim_start_matches(|character: char| character.is_ascii_whitespace());
    match value.parse::<i64>() {
        Ok(version) => Some(version.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32),
        Err(error) if error.kind() == &std::num::IntErrorKind::PosOverflow => Some(i32::MAX),
        Err(error) if error.kind() == &std::num::IntErrorKind::NegOverflow => Some(i32::MIN),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_posix::*;
    use clap::{Arg, ArgAction, Command};
    use std::env;

    fn getopt_test_command() -> Command {
        Command::new("test")
            .arg(Arg::new("verbose").short('v').action(ArgAction::SetTrue))
            .arg(Arg::new("files").action(ArgAction::Append))
    }

    #[test]
    fn test_gnu_getopt_posix_mode_stops_at_first_operand() {
        let normal = getopt_test_command()
            .gnu_getopt_with_mode(false)
            .try_get_matches_from(["test", "input", "-v"])
            .unwrap();
        assert!(normal.get_flag("verbose"));
        assert_eq!(
            normal
                .get_many::<String>("files")
                .unwrap()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["input"]
        );

        let posix = getopt_test_command()
            .gnu_getopt_with_mode(true)
            .try_get_matches_from(["test", "input", "-v"])
            .unwrap();
        assert!(!posix.get_flag("verbose"));
        assert_eq!(
            posix
                .get_many::<String>("files")
                .unwrap()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["input", "-v"]
        );
    }

    #[test]
    fn test_posix_version() {
        // Set a valid POSIX version in the environment
        unsafe { env::set_var("_POSIX2_VERSION", "200112") };
        assert_eq!(ct_posix_version(), Some(200112));
        // Clean up environment variable
        unsafe { env::remove_var("_POSIX2_VERSION") };

        // test_posix_version_invalid
        // Set an invalid POSIX version in the environment
        unsafe { env::set_var("_POSIX2_VERSION", "invalid_number") };
        assert_eq!(ct_posix_version(), None);
        // Clean up environment variable
        unsafe { env::remove_var("_POSIX2_VERSION") };

        // test_posix_version_missing
        // Ensure the environment variable is missing
        unsafe { env::remove_var("_POSIX2_VERSION") };
        assert_eq!(ct_posix_version(), None);

        // test_base_posix_version
        // default
        assert_eq!(None, ct_posix_version());
        // set specific version
        unsafe { env::set_var("_POSIX2_VERSION", OBSOLETE.to_string()) };
        assert_eq!(Some(OBSOLETE as i32), ct_posix_version());
        unsafe { env::set_var("_POSIX2_VERSION", TRADITIONAL.to_string()) };
        assert_eq!(Some(TRADITIONAL as i32), ct_posix_version());
        unsafe { env::set_var("_POSIX2_VERSION", MODERN.to_string()) };
        assert_eq!(Some(MODERN as i32), ct_posix_version());
    }

    #[test]
    fn test_ct_parse_posix_version_matches_gnu_strtol_forms() {
        assert_eq!(ct_parse_posix_version("-1"), Some(-1));
        assert_eq!(ct_parse_posix_version(" \t+200112"), Some(200112));
        assert_eq!(
            ct_parse_posix_version("999999999999999999999"),
            Some(i32::MAX)
        );
        assert_eq!(
            ct_parse_posix_version("-999999999999999999999"),
            Some(i32::MIN)
        );
        assert_eq!(ct_parse_posix_version("200112x"), None);
    }
}
