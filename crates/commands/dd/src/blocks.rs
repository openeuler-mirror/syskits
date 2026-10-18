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

// spell-checker:ignore datastructures rstat rposition cflags ctable

use crate::conversion_tables::ConversionTable;
use crate::datastructures::ConversionMode;
use crate::progress::ReadStat;

const NEWLINE: u8 = b'\n';
const SPACE: u8 = b' ';

#[derive(Default)]
struct BlockState {
    record: Vec<u8>,
    column: usize,
}

#[derive(Default)]
pub(crate) struct ConversionState {
    block: BlockState,
}

fn finish_block_record(state: &mut BlockState, cbs: usize, output: &mut Vec<u8>) {
    state.record.resize(cbs, SPACE);
    output.append(&mut state.record);
    state.column = 0;
}

fn block_stream(
    buf: &[u8],
    cbs: usize,
    sync: bool,
    rstat: &mut ReadStat,
    state: &mut BlockState,
    finish: bool,
) -> Vec<u8> {
    let mut output = Vec::new();

    for &byte in buf {
        if byte == NEWLINE {
            finish_block_record(state, cbs, &mut output);
        } else {
            if state.column == cbs {
                rstat.records_truncated += 1;
            } else if state.column < cbs {
                state.record.push(byte);
            }
            state.column += 1;
        }
    }

    if finish && state.column > 0 {
        if sync && rstat.reads_partial > 0 || state.record.iter().any(|&byte| byte != SPACE) {
            finish_block_record(state, cbs, &mut output);
        } else {
            state.record.clear();
            state.column = 0;
        }
    }

    output
}

/// Convert a complete buffer in one call, for focused conversion tests.
#[cfg(test)]
fn block(buf: &[u8], cbs: usize, sync: bool, rstat: &mut ReadStat) -> Vec<Vec<u8>> {
    let mut state = BlockState::default();
    block_stream(buf, cbs, sync, rstat, &mut state, true)
        .chunks(cbs)
        .map(<[u8]>::to_vec)
        .collect()
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
    state: &mut ConversionState,
    finish: bool,
) -> Vec<u8> {
    fn apply_conversion(buf: Vec<u8>, ct: &ConversionTable) -> impl Iterator<Item = u8> + '_ {
        buf.into_iter().map(|b| ct[b as usize])
    }

    match mode {
        ConversionMode::ConvertOnly(ct) => apply_conversion(buf, ct).collect(),
        ConversionMode::BlockThenConvert(ct, cbs, sync) => {
            let buf = block_stream(&buf, *cbs, *sync, rstat, &mut state.block, finish);
            apply_conversion(buf, ct).collect()
        }
        ConversionMode::ConvertThenBlock(ct, cbs, sync) => {
            let buf: Vec<_> = apply_conversion(buf, ct).collect();
            block_stream(&buf, *cbs, *sync, rstat, &mut state.block, finish)
        }
        ConversionMode::BlockOnly(cbs, sync) => {
            block_stream(&buf, *cbs, *sync, rstat, &mut state.block, finish)
        }
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

#[cfg(test)]
mod tests {
    use crate::blocks::{BlockState, block, block_stream, unblock};
    use crate::progress::ReadStat;

    const NEWLINE: u8 = b'\n';
    const SPACE: u8 = b' ';

    #[test]
    fn block_test_no_nl() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, 3u8];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(res, vec![vec![0u8, 1u8, 2u8, 3u8],]);
    }

    #[test]
    fn block_test_record_spans_input_chunks() {
        let mut rs = ReadStat::default();
        let mut state = BlockState::default();

        assert!(block_stream(b"abc", 8, false, &mut rs, &mut state, false).is_empty());
        assert!(block_stream(b"def", 8, false, &mut rs, &mut state, false).is_empty());
        assert_eq!(
            block_stream(b"\n", 8, false, &mut rs, &mut state, false),
            b"abcdef  "
        );
    }

    #[test]
    fn block_test_no_nl_short_record() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, 3u8];
        let res = block(&buf, 8, false, &mut rs);

        assert_eq!(
            res,
            vec![vec![0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE],]
        );
    }

    #[test]
    fn block_test_no_nl_trunc() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, 3u8, 4u8];
        let res = block(&buf, 4, false, &mut rs);

        // Commented section(s) should be truncated and appear for reference only.
        assert_eq!(res, vec![vec![0u8, 1u8, 2u8, 3u8 /*, 4u8*/],]);
        assert_eq!(rs.records_truncated, 1);
    }

    #[test]
    fn block_test_nl_gt_cbs_trunc() {
        let mut rs = ReadStat::default();
        let buf = [
            0u8, 1u8, 2u8, 3u8, 4u8, NEWLINE, 0u8, 1u8, 2u8, 3u8, 4u8, NEWLINE, 5u8, 6u8, 7u8, 8u8,
        ];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(
            res,
            vec![
                // Commented section(s) should be truncated and appear for reference only.
                vec![0u8, 1u8, 2u8, 3u8],
                // vec![4u8, SPACE, SPACE, SPACE],
                vec![0u8, 1u8, 2u8, 3u8],
                // vec![4u8, SPACE, SPACE, SPACE],
                vec![5u8, 6u8, 7u8, 8u8],
            ]
        );
        assert_eq!(rs.records_truncated, 2);
    }

    #[test]
    fn block_test_surrounded_nl() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, 3u8, NEWLINE, 4u8, 5u8, 6u8, 7u8, 8u8];
        let res = block(&buf, 8, false, &mut rs);

        assert_eq!(
            res,
            vec![
                vec![0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE],
                vec![4u8, 5u8, 6u8, 7u8, 8u8, SPACE, SPACE, SPACE],
            ]
        );
    }

    #[test]
    fn block_test_multiple_nl_same_cbs_block() {
        let mut rs = ReadStat::default();
        let buf = [
            0u8, 1u8, 2u8, 3u8, NEWLINE, 4u8, NEWLINE, 5u8, 6u8, 7u8, 8u8, 9u8,
        ];
        let res = block(&buf, 8, false, &mut rs);

        assert_eq!(
            res,
            vec![
                vec![0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE],
                vec![4u8, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE],
                vec![5u8, 6u8, 7u8, 8u8, 9u8, SPACE, SPACE, SPACE],
            ]
        );
    }

    #[test]
    fn block_test_multiple_nl_diff_cbs_block() {
        let mut rs = ReadStat::default();
        let buf = [
            0u8, 1u8, 2u8, 3u8, NEWLINE, 4u8, 5u8, 6u8, 7u8, NEWLINE, 8u8, 9u8,
        ];
        let res = block(&buf, 8, false, &mut rs);

        assert_eq!(
            res,
            vec![
                vec![0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE],
                vec![4u8, 5u8, 6u8, 7u8, SPACE, SPACE, SPACE, SPACE],
                vec![8u8, 9u8, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE],
            ]
        );
    }

    #[test]
    fn block_test_end_nl_diff_cbs_block() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, 3u8, NEWLINE];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(res, vec![vec![0u8, 1u8, 2u8, 3u8],]);
    }

    #[test]
    fn block_test_end_nl_same_cbs_block() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, NEWLINE];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(res, vec![vec![0u8, 1u8, 2u8, SPACE]]);
    }

    #[test]
    fn block_test_double_end_nl() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, NEWLINE, NEWLINE];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(
            res,
            vec![vec![0u8, 1u8, 2u8, SPACE], vec![SPACE, SPACE, SPACE, SPACE],]
        );
    }

    #[test]
    fn block_test_start_nl() {
        let mut rs = ReadStat::default();
        let buf = [NEWLINE, 0u8, 1u8, 2u8, 3u8];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(
            res,
            vec![vec![SPACE, SPACE, SPACE, SPACE], vec![0u8, 1u8, 2u8, 3u8],]
        );
    }

    #[test]
    fn block_test_double_surrounded_nl_no_trunc() {
        let mut rs = ReadStat::default();
        let buf = [0u8, 1u8, 2u8, 3u8, NEWLINE, NEWLINE, 4u8, 5u8, 6u8, 7u8];
        let res = block(&buf, 8, false, &mut rs);

        assert_eq!(
            res,
            vec![
                vec![0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE],
                vec![SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE],
                vec![4u8, 5u8, 6u8, 7u8, SPACE, SPACE, SPACE, SPACE],
            ]
        );
    }

    #[test]
    fn block_test_double_surrounded_nl_double_trunc() {
        let mut rs = ReadStat::default();
        let buf = [
            0u8, 1u8, 2u8, 3u8, NEWLINE, NEWLINE, 4u8, 5u8, 6u8, 7u8, 8u8,
        ];
        let res = block(&buf, 4, false, &mut rs);

        assert_eq!(
            res,
            vec![
                // Commented section(s) should be truncated and appear for reference only.
                vec![0u8, 1u8, 2u8, 3u8],
                vec![SPACE, SPACE, SPACE, SPACE],
                vec![4u8, 5u8, 6u8, 7u8 /*, 8u8*/],
            ]
        );
        assert_eq!(rs.records_truncated, 1);
    }

    #[test]
    fn unblock_test_full_cbs() {
        let buf = [0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8];
        let res = unblock(&buf, 8);

        assert_eq!(res, vec![0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, NEWLINE],);
    }

    #[test]
    fn unblock_test_all_space() {
        let buf = [SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE];
        let res = unblock(&buf, 8);

        assert_eq!(res, vec![NEWLINE],);
    }

    #[test]
    fn unblock_test_decoy_spaces() {
        let buf = [0u8, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, 7u8];
        let res = unblock(&buf, 8);

        assert_eq!(
            res,
            vec![0u8, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, 7u8, NEWLINE],
        );
    }

    #[test]
    fn unblock_test_strip_single_cbs() {
        let buf = [0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE];
        let res = unblock(&buf, 8);

        assert_eq!(res, vec![0u8, 1u8, 2u8, 3u8, NEWLINE],);
    }

    #[test]
    fn unblock_test_strip_multi_cbs() {
        let buf = vec![
            vec![0u8, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE],
            vec![0u8, 1u8, SPACE, SPACE, SPACE, SPACE, SPACE, SPACE],
            vec![0u8, 1u8, 2u8, SPACE, SPACE, SPACE, SPACE, SPACE],
            vec![0u8, 1u8, 2u8, 3u8, SPACE, SPACE, SPACE, SPACE],
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        let res = unblock(&buf, 8);

        let exp = vec![
            vec![0u8, NEWLINE],
            vec![0u8, 1u8, NEWLINE],
            vec![0u8, 1u8, 2u8, NEWLINE],
            vec![0u8, 1u8, 2u8, 3u8, NEWLINE],
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        assert_eq!(res, exp);
    }
}
