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

use itertools::Itertools;
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::io::Read;
use std::iter;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread;

use ctcore::ct_error::CTResult;

use crate::chunks::{self, Chunk, ChunkRecycled};
use crate::{sort_compare_by, sort_open, SortError, SortGlobalConfigs};

/// 检查位于 `path` 的文件是否有序。
///
/// # 返回
///
/// 我们应该退出的代码。
pub fn check(path: &OsStr, settings: &SortGlobalConfigs) -> CTResult<()> {
    const DEFAULT_BUF_SIZE: usize = 100 * 1024;
    let max_allowed_cmp = match settings.is_unique {
        true => Ordering::Less,
        false => Ordering::Equal,
    };

    let file = sort_open(path)?;
    let (recycled_sender, recycled_receiver) = sync_channel(2);
    let (loaded_sender, loaded_receiver) = sync_channel(2);
    thread::spawn({
        let settings = settings.clone();
        move || check_reader(file, &recycled_receiver, &loaded_sender, &settings)
    });
    for _ in 0..2 {
        let _ = recycled_sender.send(ChunkRecycled::new(
            match settings.buffer_size < DEFAULT_BUF_SIZE {
                true => settings.buffer_size,
                false => DEFAULT_BUF_SIZE,
            },
        ));
    }

    let mut prev_chunk: Option<Chunk> = None;
    let mut line_idx = 0;
    for chunk in loaded_receiver {
        line_idx += 1;
        if let Some(prev_chunk) = prev_chunk.take() {
            // 检查新块的第一个元素是否大于上一个块的最后一个元素
            // 前一个数据块中的元素
            let prev_last = prev_chunk.lines().last().unwrap();
            let new_first = chunk.lines().first().unwrap();
            let compare_order = sort_compare_by(
                prev_last,
                new_first,
                settings,
                prev_chunk.line_data(),
                chunk.line_data(),
            );

            if compare_order > max_allowed_cmp {
                let disorder_err = SortError::SortDisorder {
                    file: path.to_owned(),
                    line_number: line_idx,
                    line: new_first.line.to_owned(),
                    is_silent: settings.is_check_silent,
                };
                return Err(disorder_err.into());
            }

            let _ = recycled_sender.send(prev_chunk.recycle());
        }

        for (a, b) in chunk.lines().iter().tuple_windows() {
            line_idx += 1;
            let compare_order =
                sort_compare_by(a, b, settings, chunk.line_data(), chunk.line_data());
            if compare_order > max_allowed_cmp {
                let disorder_err = SortError::SortDisorder {
                    file: path.to_owned(),
                    line_number: line_idx,
                    line: b.line.to_owned(),
                    is_silent: settings.is_check_silent,
                };
                return Err(disorder_err.into());
            }
        }

        prev_chunk = Some(chunk);
    }
    Ok(())
}

/// 在阅读器线程上运行的函数。
fn check_reader(
    mut file: Box<dyn Read + Send>,
    receiver: &Receiver<ChunkRecycled>,
    sender: &SyncSender<Chunk>,
    settings: &SortGlobalConfigs,
) -> CTResult<()> {
    let mut carry_over = vec![];
    for recycled_chunk in receiver {
        let should_continue = chunks::chunk_read(
            sender,
            recycled_chunk,
            None,
            &mut carry_over,
            &mut file,
            &mut iter::empty(),
            settings.line_ending.into(),
            settings,
        )?;
        if !should_continue {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::{Cursor, Write};

    use tempfile::tempdir;

    use ctcore::ct_line_ending::CtLineEnding;

    use crate::{
        SortFieldSelector, SortKeyPosition, SortKeySettings, SortMode, SortPrecomputed,
        SORT_DEFAULT_BUF_SIZE,
    };

    use super::*;

    #[test]
    fn test_check_success() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_unique: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_debug_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_debug: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_debug_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_debug: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_ignore_leading_blanks_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_ignore_leading_blanks: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_ignore_leading_blanks_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_ignore_leading_blanks: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_ignore_case_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_ignore_case: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_ignore_case_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_ignore_case: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_dictionary_order_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_dictionary_order: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_dictionary_order_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_dictionary_order: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_ignore_non_printing_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_ignore_non_printing: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_ignore_non_printing_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_ignore_non_printing: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_merge_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_merge: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_merge_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_merge: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_reverse_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_reverse: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), false);
    }

    #[test]
    fn test_check_reverse_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_reverse: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_stable_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_stable: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_stable_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_stable: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_default() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortDefault,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_general_numeric() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortGeneralNumeric,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_numeric() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortNumeric,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_human_numeric() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortHumanNumeric,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_month() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortMonth,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_random() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortRandom,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_mode_sort_mode_version() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            mode: SortMode::SortVersion,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_check_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_check: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_check_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_check: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_check_silent_true() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_check_silent: true,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_check_silent_false() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            is_check_silent: false,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_salt_none() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            salt: None,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_salt_some_0() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            salt: Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_salt_some_value() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            salt: Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 10, 11, 12, 13, 14, 15]),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_none() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: None,
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some(','),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some_digital() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some('1'),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some_letter() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some('a'),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some_uppercase_letter() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some('A'),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some_colon() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some(':'),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some_horizontal() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some('-'),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_separator_some_space() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            separator: Some(' '),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

    #[test]
    fn test_check_threads_string() {
        let contents = b"apple\nbanana\ncarrot\n";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        let mut file = File::create(&file_path).unwrap();
        file.write_all(contents).unwrap();

        let settings = SortGlobalConfigs {
            threads: String::from("test"),
            // Set other necessary fields as default or as required
            ..Default::default()
        };

        assert_eq!(check(file_path.as_ref(), &settings).is_ok(), true);
    }

}