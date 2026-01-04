/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use std::env;

pub const OBSOLETE: usize = 199209;
pub const TRADITIONAL: usize = 200112;
pub const MODERN: usize = 200809;

pub fn ct_posix_version() -> Option<usize> {
    let ct_posix = "_POSIX2_VERSION";
    match env::var(ct_posix) {
        Ok(var) => match var.parse::<usize>() {
            Ok(size) => Some(size), // Successful parse returns Some(usize)
            Err(_) => None,         // Parse error returns None
        },
        Err(_) => None, // Variable not found returns None
    }
}

#[cfg(test)]
mod tests {
    use crate::ct_posix::*;
    use std::env;

    #[test]
    fn test_posix_version() {
        // Set a valid POSIX version in the environment
        env::set_var("_POSIX2_VERSION", "200112");
        assert_eq!(ct_posix_version(), Some(200112));
        // Clean up environment variable
        env::remove_var("_POSIX2_VERSION");

        // test_posix_version_invalid
        // Set an invalid POSIX version in the environment
        env::set_var("_POSIX2_VERSION", "invalid_number");
        assert_eq!(ct_posix_version(), None);
        // Clean up environment variable
        env::remove_var("_POSIX2_VERSION");

        // test_posix_version_missing
        // Ensure the environment variable is missing
        env::remove_var("_POSIX2_VERSION");
        assert_eq!(ct_posix_version(), None);

        // test_base_posix_version
        // default
        assert_eq!(None, ct_posix_version());
        // set specific version
        env::set_var("_POSIX2_VERSION", OBSOLETE.to_string());
        assert_eq!(Some(OBSOLETE), ct_posix_version());
        env::set_var("_POSIX2_VERSION", TRADITIONAL.to_string());
        assert_eq!(Some(TRADITIONAL), ct_posix_version());
        env::set_var("_POSIX2_VERSION", MODERN.to_string());
        assert_eq!(Some(MODERN), ct_posix_version());
    }
}
