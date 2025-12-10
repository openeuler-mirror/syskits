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
//! Iterate over lines, including the line ending character(s).
//!
//! This module provides the [`posix_version`] function, that returns
//! Some(usize) if the `_POSIX2_VERSION` environment variable is defined
//! and has value that can be parsed.
//! Otherwise returns None, so the calling utility would assume default behavior.
//!
//! NOTE: GNU (as of v9.4) recognizes three distinct values for POSIX version:
//! '199209' for POSIX 1003.2-1992, which would define Obsolete mode
//! '200112' for POSIX 1003.1-2001, which is the minimum version for Traditional mode
//! '200809' for POSIX 1003.1-2008, which is the minimum version for Modern mode
//!
//! Utilities that rely on this module:
//! `sort` (TBD)
//! `tail` (TBD)
//! `touch` (TBD)
//! `uniq`
//!
use std::env;

pub const OBSOLETE: usize = 199209;
pub const TRADITIONAL: usize = 200112;
pub const MODERN: usize = 200809;

pub fn posix_version() -> Option<usize> {
    // env::var("_POSIX2_VERSION")
    //     .ok()
    //     .and_then(|v| v.parse::<usize>().ok())
    let posix_var = "_POSIX2_VERSION";
    match env::var(posix_var) {
        Ok(value) => match value.parse::<usize>() {
            Ok(num) => Some(num), // Successful parse returns Some(usize)
            Err(_) => None,       // Parse error returns None
        },
        Err(_) => None, // Variable not found returns None
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_posix::*;
    use std::env;

    #[test]
    fn test_posix_version_valid() {
        // Set a valid POSIX version in the environment
        env::set_var("_POSIX2_VERSION", "200112");
        assert_eq!(posix_version(), Some(200112));
        // Clean up environment variable
        env::remove_var("_POSIX2_VERSION");
    }

    #[test]
    fn test_posix_version_invalid() {
        // Set an invalid POSIX version in the environment
        env::set_var("_POSIX2_VERSION", "invalid_number");
        assert_eq!(posix_version(), None);
        // Clean up environment variable
        env::remove_var("_POSIX2_VERSION");
    }

    #[test]
    fn test_posix_version_missing() {
        // Ensure the environment variable is missing
        env::remove_var("_POSIX2_VERSION");
        assert_eq!(posix_version(), None);
    }
    #[test]
    fn test_base_posix_version() {
        // default
        assert_eq!(None, posix_version());
        // set specific version
        env::set_var("_POSIX2_VERSION", OBSOLETE.to_string());
        assert_eq!(Some(OBSOLETE), posix_version());
        env::set_var("_POSIX2_VERSION", TRADITIONAL.to_string());
        assert_eq!(Some(TRADITIONAL), posix_version());
        env::set_var("_POSIX2_VERSION", MODERN.to_string());
        assert_eq!(Some(MODERN), posix_version());
    }
}
