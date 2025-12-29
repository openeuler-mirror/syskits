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

use std::error::Error;
use std::fmt::{Display, Formatter, Result};

use ctcore::ct_error::CTError;

#[derive(Debug)]
pub enum MvError {
    NoSuchFile(String),
    CannotStatNotADirectory(String),
    SameFile(String, String),
    SelfSubdirectory(String),
    SelfTargetSubdirectory(String, String),
    DirectoryToNonDirectory(String),
    NonDirectoryToDirectory(String, String),
    NotADirectory(String),
    TargetNotADirectory(String),
    FailedToAccessNotADirectory(String),
}

impl Error for MvError {}
impl CTError for MvError {}
impl Display for MvError {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            Self::NoSuchFile(s) => write!(f, "cannot stat {s}: No such file or directory"),
            Self::CannotStatNotADirectory(s) => write!(f, "cannot stat {s}: Not a directory"),
            Self::SameFile(s, t) => write!(f, "{s} and {t} are the same file"),
            Self::SelfSubdirectory(s) => write!(
                f,
                "cannot move '{s}' to a subdirectory of itself, '{s}/{s}'"
            ),
            Self::SelfTargetSubdirectory(s, t) => write!(
                f,
                "cannot move '{s}' to a subdirectory of itself, '{t}/{s}'"
            ),
            Self::DirectoryToNonDirectory(t) => {
                write!(f, "cannot overwrite directory {t} with non-directory")
            }
            Self::NonDirectoryToDirectory(s, t) => {
                write!(f, "cannot overwrite non-directory {t} with directory {s}")
            }
            Self::NotADirectory(t) => write!(f, "target {t}: Not a directory"),
            Self::TargetNotADirectory(t) => write!(f, "target directory {t}: Not a directory"),

            Self::FailedToAccessNotADirectory(t) => {
                write!(f, "failed to access {t}: Not a directory")
            }
        }
    }
}

#[test]
fn test_mv_error_display() {
    let cases = [
        (
            MvError::NoSuchFile("file.txt".to_string()),
            "cannot stat file.txt: No such file or directory",
        ),
        (
            MvError::CannotStatNotADirectory("dir".to_string()),
            "cannot stat dir: Not a directory",
        ),
        (
            MvError::SameFile("file1.txt".to_string(), "file2.txt".to_string()),
            "file1.txt and file2.txt are the same file",
        ),
        (
            MvError::SelfSubdirectory("file.txt".to_string()),
            "cannot move 'file.txt' to a subdirectory of itself, 'file.txt/file.txt'",
        ),
        (
            MvError::SelfTargetSubdirectory("file.txt".to_string(), "dir".to_string()),
            "cannot move 'file.txt' to a subdirectory of itself, 'dir/file.txt'",
        ),
        (
            MvError::DirectoryToNonDirectory("dir".to_string()),
            "cannot overwrite directory dir with non-directory",
        ),
        (
            MvError::NonDirectoryToDirectory("file.txt".to_string(), "dir".to_string()),
            "cannot overwrite non-directory dir with directory file.txt",
        ),
        (
            MvError::NotADirectory("target".to_string()),
            "target target: Not a directory",
        ),
        (
            MvError::TargetNotADirectory("target_dir".to_string()),
            "target directory target_dir: Not a directory",
        ),
        (
            MvError::FailedToAccessNotADirectory("access_dir".to_string()),
            "failed to access access_dir: Not a directory",
        ),
    ];

    for (error, expected) in cases.iter() {
        assert_eq!(format!("{}", error), *expected);
    }
}
