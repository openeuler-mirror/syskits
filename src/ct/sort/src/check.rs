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

