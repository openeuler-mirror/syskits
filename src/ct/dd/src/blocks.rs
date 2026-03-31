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

// spell-checker:ignore datastructures rstat rposition cflags ctable

use crate::conversion_tables::ConversionTable;
use crate::datastructures::ConversionMode;
use crate::progress::ReadStat;

const NEWLINE: u8 = b'\n';
const SPACE: u8 = b' ';

/// Split a slice into chunks, padding or truncating as necessary.
///
/// The slice `buf` is split on newlines, then each block is resized
/// to `cbs` bytes, padding with spaces if necessary. This function
/// expects the input bytes to be ASCII-encoded.
///
/// If `sync` is true and there has been at least one partial record
/// read from the input (as indicated in `rstat`), then leave an
/// all-spaces block at the end. Otherwise, remove the last block if
/// it is all spaces.
fn block(buf: &[u8], cbs: usize, sync: bool, rstat: &mut ReadStat) -> Vec<Vec<u8>> {
    let mut blocks = buf
        .split(|&e| e == NEWLINE)
        .map(|split| split.to_vec())
        .fold(Vec::new(), |mut blocks, mut split| {
            if split.len() > cbs {
                rstat.records_truncated += 1;
            }
            split.resize(cbs, SPACE);
            blocks.push(split);

            blocks
        });

    // If `sync` is true and there has been at least one partial
    // record read from the input, then leave the all-spaces block at
    // the end. Otherwise, remove it.
    if let Some(last) = blocks.last() {
        if (!sync || rstat.reads_partial == 0) && last.iter().all(|&e| e == SPACE) {
            blocks.pop();
        }
    }

    blocks
}

/// Trims padding from each cbs-length partition of buf
/// as specified by conv=unblock and cbs=N
/// Expects ascii encoded data
fn unblock(buf: &[u8], cbs: usize) -> Vec<u8> {
    buf.chunks(cbs).fold(Vec::new(), |mut acc, block| {
        if let Some(last_char_idx) = block.iter().rposition(|&e| e != SPACE) {
            // Include text up to last space.
            acc.extend(&block[..=last_char_idx]);
        }

        acc.push(NEWLINE);
        acc
    })
}

/// Apply the specified conversion, blocking, and/or unblocking in the right order.
///
/// The `mode` specifies the combination of conversion, blocking, and
/// unblocking to apply and the order in which to apply it. This
/// function is responsible only for applying the operations.
///
/// `buf` is the buffer of input bytes to transform. This function
/// mutates this input and also returns a new buffer of bytes
/// representing the result of the transformation.
///
/// `rstat` maintains a running total of the number of partial and
/// complete blocks read before calling this function. In certain
/// settings of `mode`, this function will update the number of
/// records truncated; that's why `rstat` is borrowed mutably.
pub(crate) fn conv_block_unblock_helper(
    buf: Vec<u8>,
    mode: &ConversionMode,
    rstat: &mut ReadStat,
) -> Vec<u8> {
    fn apply_conversion(buf: Vec<u8>, ct: &ConversionTable) -> impl Iterator<Item = u8> + '_ {
        buf.into_iter().map(|b| ct[b as usize])
    }

    match mode {
        ConversionMode::ConvertOnly(ct) => apply_conversion(buf, ct).collect(),
        ConversionMode::BlockThenConvert(ct, cbs, sync) => {
            let blocks = block(&buf, *cbs, *sync, rstat);
            blocks
                .into_iter()
                .flat_map(|block| apply_conversion(block, ct))
                .collect()
        }
        ConversionMode::ConvertThenBlock(ct, cbs, sync) => {
            let buf: Vec<_> = apply_conversion(buf, ct).collect();
            block(&buf, *cbs, *sync, rstat)
                .into_iter()
                .flatten()
                .collect()
        }
        ConversionMode::BlockOnly(cbs, sync) => block(&buf, *cbs, *sync, rstat)
            .into_iter()
            .flatten()
            .collect(),
        ConversionMode::UnblockThenConvert(ct, cbs) => {
            let buf = unblock(&buf, *cbs);
            apply_conversion(buf, ct).collect()
        }
        ConversionMode::ConvertThenUnblock(ct, cbs) => {
            let buf: Vec<_> = apply_conversion(buf, ct).collect();
            unblock(&buf, *cbs)
        }
        ConversionMode::UnblockOnly(cbs) => unblock(&buf, *cbs),
    }
}

