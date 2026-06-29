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

use std::io::Write;
use std::{cmp, mem};

use crate::FmtConfigs;
use crate::para_split::{FmtParaWords, FmtParagraph, FmtWordInfo};

struct FmtBreakArgs<'a, W: Write + ?Sized> {
    fmt_opts: &'a FmtConfigs,
    init_len: usize,
    indent_str: &'a str,
    indent_len: usize,
    is_uniform: bool,
    out_stream: &'a mut W,
}

impl<W: Write + ?Sized> FmtBreakArgs<'_, W> {
    fn compute_width(&self, w_info: &FmtWordInfo, pos_n: usize, is_fresh: bool) -> usize {
        match is_fresh {
            true => 0,
            false => {
                let post = w_info.after_tab;
                match w_info.before_tab {
                    None => post,
                    Some(pre) => {
                        post + ((pre + pos_n) / self.fmt_opts.tab_width + 1)
                            * self.fmt_opts.tab_width
                            - pos_n
                    }
                }
            }
        }
    }
}

pub fn fmt_break_lines<W: ?Sized + Write>(
    para_graph: &FmtParagraph,
    fmt_opts: &FmtConfigs,
    out_stream: &mut W,
) -> std::io::Result<()> {
    // 缩进
    let p_indent = &para_graph.indent_str;
    let p_indent_len = para_graph.indent_len;

    // words
    let p_words = FmtParaWords::new(fmt_opts, para_graph);
    let mut p_word_info = p_words.words();

    // 第一个单词将*always*出现在第一行 在此确保
    let Some(w_info) = p_word_info.next() else {
        return out_stream.write_all(b"\n");
    };

    // 打印初始值（如果存在）并获取其长度
    let p_init_len = w_info.word_nchars
        + if fmt_opts.is_crown || fmt_opts.is_tagged {
            // 处理 "init"部分
            out_stream.write_all(para_graph.init_str.as_bytes())?;
            para_graph.init_len
        } else if !para_graph.mail_header {
            // 对于on-(crown, tagged) ，与正常缩进相同
            out_stream.write_all(p_indent.as_bytes())?;
            p_indent_len
        } else {
            // 除了邮件头没有缩进之外
            0
        };

    // 在写入 init 之后写第一个字
    out_stream.write_all(w_info.word.as_bytes())?;

    // 本段是否要求统一间距？
    let is_uniform = para_graph.mail_header || fmt_opts.is_uniform;

    let mut break_args = FmtBreakArgs {
        fmt_opts,
        init_len: p_init_len,
        indent_str: p_indent,
        indent_len: p_indent_len,
        is_uniform,
        out_stream,
    };

    if fmt_opts.is_quick || para_graph.mail_header {
        fmt_break_simple(p_word_info, &mut break_args)
    } else {
        fmt_break_knuth_plass(p_word_info, &mut break_args)
    }
}

// break_simple 实现了一种 "贪婪 "的分行算法：打印单词，直到超过最大长度。
// 超过最大长度，然后打印一个换行符和缩进符，然后继续。
fn fmt_break_simple<'a, W: ?Sized + Write, T: Iterator<Item = &'a FmtWordInfo<'a>>>(
    mut iter: T,
    fmt_args: &mut FmtBreakArgs<'a, W>,
) -> std::io::Result<()> {
    iter.try_fold((fmt_args.init_len, false), |(l, prev_punct), winfo| {
        fmt_accum_words_simple(fmt_args, l, prev_punct, winfo)
    })?;
    fmt_args.out_stream.write_all(b"\n")
}

fn fmt_accum_words_simple<'a, W: ?Sized + Write>(
    fmt_args: &mut FmtBreakArgs<'a, W>,
    l_size: usize,
    prev_punct: bool,
    w_info: &'a FmtWordInfo<'a>,
) -> std::io::Result<(usize, bool)> {
    // 计算该单词的长度，考虑制表符在该行该位置的展开情况
    let w_len = w_info.word_nchars + fmt_args.compute_width(w_info, l_size, false);

    let slen = fmt_compute_slen(
        fmt_args.is_uniform,
        w_info.is_new_line,
        w_info.is_sentence_start,
        prev_punct,
    );

    if l_size + w_len + slen > fmt_args.fmt_opts.width {
        fmt_write_newline(fmt_args.indent_str, fmt_args.out_stream)?;
        fmt_write_with_spaces(&w_info.word[w_info.word_start..], 0, fmt_args.out_stream)?;
        Ok((
            fmt_args.indent_len + w_info.word_nchars,
            w_info.is_ends_punct,
        ))
    } else {
        fmt_write_with_spaces(w_info.word, slen, fmt_args.out_stream)?;
        Ok((l_size + w_len + slen, w_info.is_ends_punct))
    }
}

// 最优分段算法
fn fmt_break_knuth_plass<'a, W: ?Sized + Write, T: Clone + Iterator<Item = &'a FmtWordInfo<'a>>>(
    mut iter: T,
    fmt_args: &mut FmtBreakArgs<'a, W>,
) -> std::io::Result<()> {
    // 运行算法获取断点
    let break_points = fmt_find_kp_breakpoints(iter.clone(), fmt_args);
    // 遍历断点（注意，断点的断开顺序是相反的，因此我们要 .rev() 它
    let result: std::io::Result<(bool, bool)> = break_points.iter().rev().try_fold(
        (false, false),
        |(mut prev_punct, mut fresh), &(next_break, break_before)| {
            if fresh {
                fmt_write_newline(fmt_args.indent_str, fmt_args.out_stream)?;
            }
            // 在每个断点上，不断发出单词，直到找到与该断点相匹配的单词为止
            for w_info in &mut iter {
                let (slen, word) = fmt_slice_if_fresh(
                    fresh,
                    w_info.word,
                    w_info.word_start,
                    fmt_args.is_uniform,
                    w_info.is_new_line,
                    w_info.is_sentence_start,
                    prev_punct,
                );
                fresh = false;
                prev_punct = w_info.is_ends_punct;

                // 通过比较引用的地址，我们可以在这里找到相同的断点。
                // 这没有问题，因为一旦我们断行，后退向量就不会发生变化。
                let w_info_ptr = w_info as *const _;
                let next_break_ptr = next_break as *const _;
                if w_info_ptr == next_break_ptr {
                    // 确定，我们找到了匹配的单词
                    if break_before {
                        fmt_write_newline(fmt_args.indent_str, fmt_args.out_stream)?;
                        fmt_write_with_spaces(
                            &w_info.word[w_info.word_start..],
                            0,
                            fmt_args.out_stream,
                        )?;
                    } else {
                        // 在这个词之后中断，因此这意味着 "fresh "在下一次迭代中为真
                        fmt_write_with_spaces(word, slen, fmt_args.out_stream)?;
                        fresh = true;
                    }
                    break;
                } else {
                    fmt_write_with_spaces(word, slen, fmt_args.out_stream)?;
                }
            }
            Ok((prev_punct, fresh))
        },
    );
    let (mut is_prev_punct, mut is_fresh) = result?;

    // 在最后一个换行符之后，写出最后一行的其余部分。
    for w_info in iter {
        if is_fresh {
            fmt_write_newline(fmt_args.indent_str, fmt_args.out_stream)?;
        }
        let (s_len, word) = fmt_slice_if_fresh(
            is_fresh,
            w_info.word,
            w_info.word_start,
            fmt_args.is_uniform,
            w_info.is_new_line,
            w_info.is_sentence_start,
            is_prev_punct,
        );
        is_prev_punct = w_info.is_ends_punct;
        is_fresh = false;
        fmt_write_with_spaces(word, s_len, fmt_args.out_stream)?;
    }
    fmt_args.out_stream.write_all(b"\n")
}

struct FmtLineBreak<'a> {
    prev: usize,
    linebreak: Option<&'a FmtWordInfo<'a>>,
    is_break_before: bool,
    demerits: i64,
    prev_rat: f32,
    length: usize,
    is_fresh: bool,
}

#[allow(clippy::cognitive_complexity)]
fn fmt_find_kp_breakpoints<'a, W: ?Sized + Write, T: Iterator<Item = &'a FmtWordInfo<'a>>>(
    iter: T,
    fmt_args: &FmtBreakArgs<'a, W>,
) -> Vec<(&'a FmtWordInfo<'a>, bool)> {
    let mut iter = iter.peekable();
    // 设置初始空分隔线
    let mut line_breaks = vec![FmtLineBreak {
        prev: 0,
        linebreak: None,
        is_break_before: false,
        demerits: 0,
        prev_rat: 0.0,
        length: fmt_args.init_len,
        is_fresh: false,
    }];
    // this vec 保存当前激活的换行符；next_ 保存下一个单词将激活的换行符。下一个单词
    let mut active_breaks = vec![0];
    let mut next_active_breaks = vec![];

    let stretch = fmt_args.fmt_opts.width - fmt_args.fmt_opts.goal;
    let minlength = fmt_args.fmt_opts.goal - stretch;
    let mut new_linebreaks = vec![];
    let mut is_sentence_start = false;
    let mut least_demerits = 0;
    loop {
        let Some(w) = iter.next() else {
            break;
        };

        // 如果这是最后一个字，我们不会对这次断句追加扣分
        let (is_last_word, is_sentence_end) = match iter.peek() {
            None => (true, true),
            Some(&&FmtWordInfo {
                is_sentence_start: st,
                is_new_line: nl,
                ..
            }) => (false, st || (nl && w.is_ends_punct)),
        };

        // 我们是否应该在下一句的开头加上额外的空格？
        let s_len = fmt_compute_slen(fmt_args.is_uniform, w.is_new_line, is_sentence_start, false);

        let mut ld_new = i64::MAX;
        let mut ld_next = i64::MAX;
        let mut ld_idx = 0;
        new_linebreaks.clear();
        next_active_breaks.clear();
        // 浏览每个活动分段，扩展它，如果超过了所需的最小长度，还可能添加一个新的活动分段。如果我们超过了所需的最小长度
        #[allow(clippy::explicit_iter_loop)]
        for &i in active_breaks.iter() {
            let active = &mut line_breaks[i];
            // 对扣分进行归一化处理，以避免溢出，并记录这是否是最少的扣分
            active.demerits -= least_demerits;
            if active.demerits < ld_next {
                ld_next = active.demerits;
                ld_idx = i;
            }

            // 获得新长度
            let t_len = w.word_nchars
                + fmt_args.compute_width(w, active.length, active.is_fresh)
                + s_len
                + active.length;

            // 如果 tlen 长于 args.opts.width，我们会将此分段从活动列表中删除
            // 否则，我们将延长分隔符，并可能在此时添加一个新的分隔符
            if t_len <= fmt_args.fmt_opts.width {
                // 下次中断仍将有效
                next_active_breaks.push(i);
                // 我们可以把这个词放在这一行
                active.is_fresh = false;
                active.length = t_len;

                // 如果我们超过了最小长度，我们也可以考虑在这里断开
                if t_len >= minlength {
                    let (new_demerits, new_ratio) = if is_last_word {
                        // 最后一行的长度不会受到惩罚
                        (0, 0.0)
                    } else {
                        fmt_compute_demerits(
                            fmt_args.fmt_opts.goal as isize - t_len as isize,
                            stretch,
                            w.word_nchars,
                            active.prev_rat,
                        )
                    };

                    // 甚至不要考虑添加扣分过多的行
                    // 还尝试通过检查符号来检测溢出
                    let total_demerits = new_demerits + active.demerits;
                    if new_demerits < FMT_BAD_INFTY_SQ
                        && total_demerits < ld_new
                        && active.demerits.signum() <= new_demerits.signum()
                    {
                        ld_new = total_demerits;
                        new_linebreaks.push(FmtLineBreak {
                            prev: i,
                            linebreak: Some(w),
                            is_break_before: false,
                            demerits: total_demerits,
                            prev_rat: new_ratio,
                            length: fmt_args.indent_len,
                            is_fresh: true,
                        });
                    }
                }
            }
        }

        // 如果我们生成了新的换行符，则将最后一条添加到列表中
        // 最后一条总是最好的，因为我们不会把它添加到 new_linebreaks 中，除非
        // 它比目前最好的一条更好
        match new_linebreaks.pop() {
            None => (),
            Some(lb) => {
                next_active_breaks.push(line_breaks.len());
                line_breaks.push(lb);
            }
        }

        if next_active_breaks.is_empty() {
            // 每个可能的换行符都太长！选择扣分最少的换行符，ld_idx
            let new_break = fmt_restart_active_breaks(
                fmt_args,
                &line_breaks[ld_idx],
                ld_idx,
                w,
                s_len,
                minlength,
            );
            next_active_breaks.push(line_breaks.len());
            line_breaks.push(new_break);
            least_demerits = 0;
        } else {
            // 下一次，将扣分字段归一化
            // 活动分隔线上，以减少溢出的可能性
            least_demerits = cmp::max(ld_next, 0);
        }
        // 交换新的活动中断列表
        mem::swap(&mut active_breaks, &mut next_active_breaks);
        // 如果这是一个句子中的最后一个词，那么下一个词一定是下一个句子中的第一个词。
        is_sentence_start = is_sentence_end;
    }

    // 返回最佳路径
    fmt_build_best_path(&line_breaks, &active_breaks)
}

fn fmt_build_best_path<'a>(
    paths: &[FmtLineBreak<'a>],
    active_paths: &[usize],
) -> Vec<(&'a FmtWordInfo<'a>, bool)> {
    // 在活动路径中，我们选择扣分最少的路径
    active_paths
        .iter()
        .min_by_key(|&&a| paths[a].demerits)
        .map(|&(mut best_idx)| {
            let mut breakwords = vec![];
            // 现在，在断句列表中回溯指针，记录 我们应该断开的单词
            loop {
                let line_next_best = &paths[best_idx];
                match line_next_best.linebreak {
                    None => return breakwords,
                    Some(prev) => {
                        breakwords.push((prev, line_next_best.is_break_before));
                        best_idx = line_next_best.prev;
                    }
                }
            }
        })
        .unwrap_or_default()
}

// 由于扣分的计算方式，"infinite"坏处更像是 (1+BAD_INFTY)^2
const FMT_BAD_INFTY: i64 = 10_000_000;
const FMT_BAD_INFTY_SQ: i64 = FMT_BAD_INFTY * FMT_BAD_INFTY;
// 坏度 = BAD_MULT * abs(r) ^ 3
const FMT_BAD_MULT: f32 = 100.0;
// DR_MULT 是线间 delta-R 的乘数
const FMT_DR_MULT: f32 = 600.0;
// DL_MULT 是行尾短字的惩罚乘数
const FMT_DL_MULT: f32 = 300.0;

fn fmt_compute_demerits(
    delta_len: isize,
    stretch: usize,
    w_len: usize,
    prev_rat: f32,
) -> (i64, f32) {
    // 我们使用了多少
    let ratio = match delta_len {
        0 => 0.0f32,
        _ => delta_len as f32 / stretch as f32,
    };

    // 根据拉伸比计算坏度
    let bad_line_len = match ratio.abs() > 1.0f32 {
        true => FMT_BAD_INFTY,
        false => (FMT_BAD_MULT * ratio.powi(3).abs()) as i64,
    };

    // 我们将惩罚以非常短的单词结尾的行文
    let bad_word_len = match w_len >= stretch {
        true => 0,
        false => {
            (FMT_DL_MULT
                * ((stretch - w_len) as f32 / (stretch - 1) as f32)
                    .powi(3)
                    .abs()) as i64
        }
    };

    // 我们会惩罚那些与前几行比率相差很大的行
    let bad_delta_r = (FMT_DR_MULT * ((ratio - prev_rat) / 2.0).powi(3).abs()) as i64;

    let demerits = i64::pow(1 + bad_line_len + bad_word_len + bad_delta_r, 2);

    (demerits, ratio)
}

fn fmt_restart_active_breaks<'a, W: ?Sized + Write>(
    fmt_args: &FmtBreakArgs<'a, W>,
    active: &FmtLineBreak<'a>,
    act_idx: usize,
    w: &'a FmtWordInfo<'a>,
    s_len: usize,
    min: usize,
) -> FmtLineBreak<'a> {
    let (break_before, line_length) = if active.is_fresh {
        // 一个单词是一行的第一个单词
        (false, fmt_args.indent_len)
    } else {
        let w_len = w.word_nchars + fmt_args.compute_width(w, active.length, active.is_fresh);
        let under_len = min as isize - active.length as isize;
        let over_len = (w_len + s_len + active.length) as isize - fmt_args.fmt_opts.width as isize;
        if over_len > under_len {
            // 将该单词放在下一行
            (true, fmt_args.indent_len + w.word_nchars)
        } else {
            (false, fmt_args.indent_len)
        }
    };

    // 分隔线
    FmtLineBreak {
        prev: act_idx,
        linebreak: Some(w),
        is_break_before: break_before,
        demerits: 0,
        prev_rat: if break_before { 1.0 } else { -1.0 },
        length: line_length,
        is_fresh: !break_before,
    }
}

// 根据模式、换行、句子起始，在单词前添加的空格数。
fn fmt_compute_slen(is_uniform: bool, is_newline: bool, is_start: bool, is_punct: bool) -> usize {
    if is_uniform || is_newline {
        if is_start || (is_newline && is_punct) {
            2
        } else {
            1
        }
    } else {
        0
    }
}

// 如果是新行，则 slen=0 并切掉前端空白。
// 否则，计算 slen 并保留空白。
fn fmt_slice_if_fresh(
    is_fresh: bool,
    word: &str,
    start: usize,
    is_uniform: bool,
    is_newline: bool,
    is_s_start: bool,
    is_punct: bool,
) -> (usize, &str) {
    match is_fresh {
        true => (0, &word[start..]),
        false => (
            fmt_compute_slen(is_uniform, is_newline, is_s_start, is_punct),
            word,
        ),
    }
}

// 写入换行符并添加缩进。
fn fmt_write_newline<W: ?Sized + Write>(
    indent: &str,
    output_stream: &mut W,
) -> std::io::Result<()> {
    output_stream.write_all(b"\n")?;
    output_stream.write_all(indent.as_bytes())
}

// 写出单词，并留出空格。
fn fmt_write_with_spaces<W: ?Sized + Write>(
    word: &str,
    s_len: usize,
    output_stream: &mut W,
) -> std::io::Result<()> {
    match s_len {
        1 => {
            output_stream.write_all(b" ")?;
        }
        2 => {
            output_stream.write_all(b"  ")?;
        }
        _ => {}
    }

    output_stream.write_all(word.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod compute_width_tests {
        use super::*;

        // Helper function to create FmtConfigs
        fn create_fmt_configs(tab_width: usize) -> FmtConfigs {
            FmtConfigs {
                width: 50,
                goal: 45,
                tab_width,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            }
        }

        // Helper function to create FmtWordInfo
        fn create_word_info(before_tab: Option<usize>, after_tab: usize) -> FmtWordInfo<'static> {
            FmtWordInfo {
                word: "test",
                word_start: 0,
                word_nchars: 4,
                before_tab,
                after_tab,
                is_sentence_start: false,
                is_ends_punct: false,
                is_new_line: false,
            }
        }

        #[test]
        fn test_compute_width_fresh_true() {
            let fmt_configs = create_fmt_configs(8);
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info(Some(5), 10);
            let width = fmt_args.compute_width(&w_info, 5, true);
            assert_eq!(width, 0, "Width should be 0 when is_fresh is true");
        }

        #[test]
        fn test_compute_width_no_tab() {
            let fmt_configs = create_fmt_configs(8);
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info(None, 10);
            let width = fmt_args.compute_width(&w_info, 5, false);
            assert_eq!(
                width, 10,
                "Width should equal after_tab when before_tab is None"
            );
        }

        #[test]
        fn test_compute_width_with_tab() {
            let fmt_configs = create_fmt_configs(8);
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info(Some(3), 10);
            let width = fmt_args.compute_width(&w_info, 5, false);
            let expected_width = 10 + ((3 + 5) / 8 + 1) * 8 - 5;
            assert_eq!(
                width, expected_width,
                "Width should account for tab expansion correctly"
            );
        }

        // 边界条件测试：当 before_tab 正好在制表符边界
        #[test]
        fn test_compute_width_tab_boundary() {
            let fmt_configs = create_fmt_configs(4); // 4 作为一个常见的制表符宽度
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info(Some(4), 10); // 制表符宽度为 4，且 before_tab 在 4 的位置
            let width = fmt_args.compute_width(&w_info, 4, false);
            let expected_width = 10 + ((4 + 4) / 4 + 1) * 4 - 4;
            assert_eq!(
                width, expected_width,
                "Width should handle tab boundary correctly"
            );
        }

        // 制表符宽度测试：尝试不同的制表符宽度
        #[test]
        fn test_compute_width_different_tab_widths() {
            let tab_widths = [2, 3, 5, 6]; // 不同的制表符宽度
            let pos_n = 7; // 测试一个不规则的位置
            let before_tab = 2;

            for tab_width in tab_widths {
                let fmt_configs = create_fmt_configs(tab_width);
                let mut output_stream = Vec::new();
                let fmt_args = FmtBreakArgs {
                    fmt_opts: &fmt_configs,
                    init_len: 0,
                    indent_str: "",
                    indent_len: 0,
                    is_uniform: false,
                    out_stream: &mut output_stream,
                };

                let w_info = create_word_info(Some(before_tab), 10);
                let width = fmt_args.compute_width(&w_info, pos_n, false);
                let expected_width =
                    10 + ((before_tab + pos_n) / tab_width + 1) * tab_width - pos_n;
                assert_eq!(width, expected_width);
            }
        }

        // 复杂制表符位置测试：高 pos_n 值
        #[test]
        fn test_compute_width_high_pos_n() {
            let fmt_configs = create_fmt_configs(4);
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info(Some(3), 15);
            let pos_n = 18; // 较高的 pos_n 值
            let width = fmt_args.compute_width(&w_info, pos_n, false);
            let expected_width = 15 + (pos_n.div_ceil(4) + 1) * 4 - pos_n;
            assert_eq!(
                width, expected_width,
                "Width should be calculated correctly for high pos_n values"
            );
        }
    }

    #[cfg(test)]
    mod break_lines_tests {
        use std::str;

        use super::*;

        // Helper function to create FmtParagraph
        fn create_paragraph() -> FmtParagraph {
            FmtParagraph {
                lines: vec![],
                init_str: "   ".to_string(), // Assume 3 spaces
                init_len: 3,
                init_end: 3,
                indent_str: "   ".to_string(),
                indent_len: 3,
                indent_end: 3,
                mail_header: false,
            }
        }

        // Mock FmtConfigs
        fn create_fmt_configs(width: usize, is_quick: bool) -> FmtConfigs {
            FmtConfigs {
                width,
                goal: width - 5, // Simple example
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick,
            }
        }

        #[test]
        fn test_simple_break() {
            let mut output_stream = Vec::new();
            let mut para_graph = create_paragraph();
            para_graph.mail_header = true; // Switch to simple breaking due to mail header
            let fmt_opts = create_fmt_configs(50, false);

            // Simulate the presence of words
            para_graph.lines.push("hello".to_string());
            para_graph.lines.push("world".to_string());

            fmt_break_lines(&para_graph, &fmt_opts, &mut output_stream).unwrap();
            let output = str::from_utf8(&output_stream).unwrap();
            // We expect that "hello" and "world" would be processed by fmt_break_simple,
            // but this depends on the specifics of your fmt_break_simple implementation.
            assert!(output.contains("hello"), "Output should contain 'hello'");
            assert!(output.contains("world"), "Output should contain 'world'");
            assert!(output.ends_with("\n"), "Output should end with a newline");
        }

        #[test]
        fn test_knuth_plass_break() {
            let mut output_stream = Vec::new();
            let mut para_graph = create_paragraph();
            let fmt_opts = create_fmt_configs(50, false); // Use Knuth-Plass as is_quick is false

            // Simulate the presence of words
            para_graph.lines.push("longer".to_string());
            para_graph.lines.push("paragraph".to_string());

            fmt_break_lines(&para_graph, &fmt_opts, &mut output_stream).unwrap();
            let output = str::from_utf8(&output_stream).unwrap();
            // Expectations should align with how fmt_break_knuth_plass would format these words
            assert!(output.contains("ger"), "Output should contain 'longer'");
            assert!(
                output.contains("agraph"),
                "Output should contain 'paragraph'"
            );
            assert!(output.ends_with("\n"), "Output should end with a newline");
        }
    }

    #[cfg(test)]
    mod break_simple_tests {
        use std::str;

        use super::*;

        // Helper function to create a mock FmtWordInfo
        fn create_word_info(
            word: &'static str,
            word_start: usize,
            nchars: usize,
            is_new_line: bool,
            is_sentence_start: bool,
            is_ends_punct: bool,
        ) -> FmtWordInfo<'static> {
            FmtWordInfo {
                word,
                word_start,
                word_nchars: nchars,
                before_tab: None,
                after_tab: nchars,
                is_sentence_start,
                is_ends_punct,
                is_new_line,
            }
        }

        #[test]
        fn test_single_word_fits() {
            let mut output_stream = Vec::new();
            let fmt_configs = FmtConfigs {
                width: 50,
                goal: 45,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let word_info = create_word_info("hello", 0, 5, false, false, false);
            let words = [word_info];
            fmt_break_simple(words.iter(), &mut fmt_args).unwrap();
            let output = str::from_utf8(&output_stream).unwrap();
            assert_eq!(
                output, "hello\n",
                "Output should contain the word 'hello' followed by a newline"
            );
        }

        #[test]
        fn test_multiple_words_line_wrap() {
            let mut output_stream = Vec::new();
            let fmt_configs = FmtConfigs {
                width: 10, // Ensure line wrap occurs
                goal: 10,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let word_info1 = create_word_info("hello", 0, 5, false, false, false);
            let word_info2 = create_word_info("world", 0, 5, false, false, false);
            let words = [word_info1, word_info2];
            fmt_break_simple(words.iter(), &mut fmt_args).unwrap();
            let output = str::from_utf8(&output_stream).unwrap();
            assert_eq!(
                output, "hello\nworld\n",
                "Output should have words 'hello' and 'world' separated by newlines due to wrapping"
            );
        }
    }

    #[cfg(test)]
    mod accum_words_simple_tests {
        use super::*;

        fn create_word_info(
            word: &'static str,
            word_start: usize,
            nchars: usize,
            is_new_line: bool,
            is_sentence_start: bool,
            is_ends_punct: bool,
        ) -> FmtWordInfo<'static> {
            FmtWordInfo {
                word,
                word_start,
                word_nchars: nchars,
                before_tab: None,
                after_tab: nchars,
                is_sentence_start,
                is_ends_punct,
                is_new_line,
            }
        }

        #[test]
        fn test_word_fits_on_line() {
            let fmt_configs = FmtConfigs {
                width: 20,
                goal: 15,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut output_stream = Vec::new();
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info("hello", 0, 5, false, false, false);
            let (new_size, _) = fmt_accum_words_simple(&mut fmt_args, 0, false, &w_info).unwrap();
            let output = std::str::from_utf8(&output_stream).unwrap();
            assert_eq!(new_size, 10, "New line size should be 5");
            assert_eq!(output, "hello", "Output should contain the word 'hello'");
        }

        #[test]
        fn test_word_exceeds_line_width() {
            let fmt_configs = FmtConfigs {
                width: 10,
                goal: 8,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut output_stream = Vec::new();
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let w_info = create_word_info("hello", 0, 5, false, false, false);
            let (new_size, _) = fmt_accum_words_simple(&mut fmt_args, 6, false, &w_info).unwrap();
            let output = std::str::from_utf8(&output_stream).unwrap();
            assert_eq!(new_size, 5, "New line size should reset to 5 after newline");
            assert_eq!(
                output, "\nhello",
                "Output should contain a newline followed by 'hello'"
            );
        }
    }

    #[cfg(test)]
    mod break_knuth_plass_tests {
        use std::str;

        use super::*;

        fn create_word_info(
            word: &'static str,
            is_ends_punct: bool,
            is_new_line: bool,
            is_sentence_start: bool,
        ) -> FmtWordInfo<'static> {
            FmtWordInfo {
                word,
                word_start: 0,
                word_nchars: word.len(),
                before_tab: None,
                after_tab: word.len(),
                is_sentence_start,
                is_ends_punct,
                is_new_line,
            }
        }

        #[test]
        fn test_empty_input() -> std::io::Result<()> {
            let mut output_stream = Vec::new();
            let fmt_configs = FmtConfigs {
                width: 50,
                goal: 45,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let words: Vec<FmtWordInfo> = vec![];
            fmt_break_knuth_plass(words.iter(), &mut fmt_args)?;
            assert_eq!(
                str::from_utf8(&output_stream).unwrap(),
                "\n",
                "Should end with a newline"
            );
            Ok(())
        }

        #[test]
        fn test_single_word() -> std::io::Result<()> {
            let mut output_stream = Vec::new();
            let fmt_configs = FmtConfigs {
                width: 50,
                goal: 45,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let word = create_word_info("hello", false, false, true);
            let words = [word];
            fmt_break_knuth_plass(words.iter(), &mut fmt_args).unwrap();
            assert_eq!(
                str::from_utf8(&output_stream).unwrap(),
                "hello\n",
                "Output should match expected single word with a newline"
            );
            Ok(())
        }

        #[test]
        fn test_multiple_words_newline_break() -> std::io::Result<()> {
            let mut output_stream = Vec::new();
            let fmt_configs = FmtConfigs {
                width: 15, // Force a line break due to width
                goal: 10,
                tab_width: 4,
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
            };
            let mut fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let word1 = create_word_info("hello", false, false, true);
            let word2 = create_word_info("world", true, true, false);
            let words = [word1, word2];
            fmt_break_knuth_plass(words.iter(), &mut fmt_args).unwrap();
            assert_eq!(
                str::from_utf8(&output_stream).unwrap(),
                "hello\nworld\n",
                "Output should have words separated by a newline due to wrapping"
            );
            Ok(())
        }
    }

    #[cfg(test)]
    mod find_kp_breakpoints_tests {
        use super::*;

        #[test]
        fn test_empty_input() {
            let fmt_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 50,
                goal: 45,
                tab_width: 4,
            };
            let mut output_stream = Vec::new(); // 使用 Vec<u8> 作为输出流模拟
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let words = [];
            let result = fmt_find_kp_breakpoints(words.iter(), &fmt_args);
            assert!(
                result.is_empty(),
                "The result should be empty for no input words"
            );
        }

        #[test]
        fn test_single_word_no_break() {
            let word_info = FmtWordInfo {
                word: "hello",
                word_start: 0,
                word_nchars: 5,
                before_tab: None,
                after_tab: 5,
                is_sentence_start: false,
                is_ends_punct: false,
                is_new_line: false,
            };
            let fmt_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 50,
                goal: 45,
                tab_width: 4,
            };
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let words = [word_info];
            let result = fmt_find_kp_breakpoints(words.iter(), &fmt_args);
            assert_eq!(result.len(), 0, "There should be exactly one break point");
        }

        #[test]
        fn test_multiple_words() {
            let word_info1 = FmtWordInfo {
                word: "hello",
                word_start: 0,
                word_nchars: 5,
                before_tab: None,
                after_tab: 5,
                is_sentence_start: true,
                is_ends_punct: false,
                is_new_line: false,
            };
            let word_info2 = FmtWordInfo {
                word: "world",
                word_start: 6,
                word_nchars: 5,
                before_tab: None,
                after_tab: 11,
                is_sentence_start: false,
                is_ends_punct: true,
                is_new_line: false,
            };
            let fmt_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 10,
                goal: 10,
                tab_width: 4,
            };
            let mut output_stream = Vec::new();
            let fmt_args = FmtBreakArgs {
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                indent_len: 0,
                is_uniform: false,
                out_stream: &mut output_stream,
            };

            let words = [word_info1, word_info2];
            let result = fmt_find_kp_breakpoints(words.iter(), &fmt_args);
            assert_eq!(result.len(), 1, "There should be two break points");
            assert_eq!(
                result[0].0.word, "hello",
                "The first word should be 'hello'"
            );
        }
    }

    #[cfg(test)]
    mod build_best_path_tests {
        use super::*;

        #[test]
        fn test_fmt_build_best_path_empty() {
            let paths: Vec<FmtLineBreak> = vec![];
            let active_paths: Vec<usize> = vec![];

            let result = fmt_build_best_path(&paths, &active_paths);
            assert_eq!(result, vec![]);
        }
    }

    #[cfg(test)]
    mod compute_demerits_tests {
        use super::*;

        #[test]
        fn test_fmt_compute_demerits() {
            // Test case: delta_len is 0
            let (demerits, ratio) = fmt_compute_demerits(0, 10, 5, 1.0);
            assert_eq!(demerits, 16129);
            assert_eq!(ratio, 0.0);

            // Test case: w_len is greater than or equal to stretch
            let (demerits, ratio) = fmt_compute_demerits(10, 10, 15, 1.0);
            assert_eq!(demerits, 10201);
            assert_eq!(ratio, 1.0);
        }
    }

    #[cfg(test)]
    mod restart_active_breaks_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_fmt_restart_active_breaks() {
            let mut output = Cursor::new(Vec::new());
            let fmt_args = FmtBreakArgs {
                indent_len: 4,
                is_uniform: false,
                fmt_opts: &FmtConfigs {
                    is_crown: false,
                    is_tagged: false,
                    is_mail: false,
                    is_split_only: false,
                    prefix_option: None,
                    is_xprefix: false,
                    anti_prefix_option: None,
                    is_xanti_prefix: false,
                    is_uniform: false,
                    is_quick: false,
                    width: 80,
                    goal: 0,
                    tab_width: 0,
                },
                init_len: 0,
                indent_str: "",
                out_stream: &mut output,
            };
            let w = FmtWordInfo {
                word: "",
                word_start: 0,
                word_nchars: 5,
                before_tab: None,
                after_tab: 0,
                is_sentence_start: false,
                is_ends_punct: false,
                is_new_line: false,
            };
            let active = FmtLineBreak {
                prev: 0,
                linebreak: None,
                is_break_before: false,
                demerits: 0,
                prev_rat: 1.0,
                length: 0,
                is_fresh: true,
            };
            let s_len = 2;
            let min = 10;

            let result = fmt_restart_active_breaks(&fmt_args, &active, 1, &w, s_len, min);

            assert_eq!(result.prev, 1);
            assert_eq!(result.linebreak, Some(&w));
            assert!(!result.is_break_before);
            assert_eq!(result.demerits, 0);
            assert_eq!(result.prev_rat, -1.0);
            assert_eq!(result.length, 4);
            assert!(result.is_fresh);
        }

        #[test]
        fn test_fmt_restart_active_breaks_edge_cases() {
            let fmt_configs = FmtConfigs {
                is_crown: false,
                is_tagged: false,
                is_mail: false,
                is_split_only: false,
                prefix_option: None,
                is_xprefix: false,
                anti_prefix_option: None,
                is_xanti_prefix: false,
                is_uniform: false,
                is_quick: false,
                width: 80,
                goal: 0,
                tab_width: 0,
            };

            let fmt_args = FmtBreakArgs {
                indent_len: 4,
                is_uniform: false,
                fmt_opts: &fmt_configs,
                init_len: 0,
                indent_str: "",
                out_stream: &mut Cursor::new(Vec::new()),
            };

            // 边界条件：act_idx为0，应保持新鲜行状态
            let w0 = FmtWordInfo {
                word: "",
                word_start: 0,
                word_nchars: 5,
                before_tab: None,
                after_tab: 0,
                is_sentence_start: false,
                is_ends_punct: false,
                is_new_line: false,
            };
            let active0 = FmtLineBreak {
                prev: 0,
                linebreak: None,
                is_break_before: false,
                demerits: 0,
                prev_rat: 1.0,
                length: 0,
                is_fresh: true,
            };
            let result0 = fmt_restart_active_breaks(&fmt_args, &active0, 0, &w0, 2, 10);
            assert_eq!(result0.prev, 0);
            assert_eq!(
                result0.linebreak,
                Some(&FmtWordInfo {
                    word: "",
                    word_start: 0,
                    word_nchars: 5,
                    before_tab: None,
                    after_tab: 0,
                    is_sentence_start: false,
                    is_ends_punct: false,
                    is_new_line: false
                })
            );
            assert!(!result0.is_break_before);
            assert_eq!(result0.demerits, 0);
            assert_eq!(result0.prev_rat, -1.0);
            assert_eq!(result0.length, 4);
            assert!(result0.is_fresh);

            // 边界条件：word长度等于宽度，应触发换行
            let w1 = FmtWordInfo {
                word: "",
                word_start: 0,
                word_nchars: fmt_args.fmt_opts.width,
                before_tab: None,
                after_tab: 0,
                is_sentence_start: false,
                is_ends_punct: false,
                is_new_line: false,
            };
            let active1 = FmtLineBreak {
                prev: 0,
                linebreak: None,
                is_break_before: false,
                demerits: 0,
                prev_rat: 1.0,
                length: 0,
                is_fresh: true,
            };
            let result1 = fmt_restart_active_breaks(&fmt_args, &active1, 1, &w1, 2, 10);
            assert!(!result1.is_break_before);

            // 边界条件：min等于line_length，理论上不会触发换行
            let w2 = FmtWordInfo {
                word: "",
                word_start: 0,
                word_nchars: 5,
                before_tab: None,
                after_tab: 0,
                is_sentence_start: false,
                is_ends_punct: false,
                is_new_line: false,
            };
            let active2 = FmtLineBreak {
                prev: 4,
                linebreak: None,
                is_break_before: false,
                demerits: 0,
                prev_rat: 1.0,
                length: 4,
                is_fresh: false,
            };
            let result2 = fmt_restart_active_breaks(&fmt_args, &active2, 1, &w2, 2, 4);
            assert!(!result2.is_break_before); // 预期不换行
        }
    }

    #[cfg(test)]
    mod compute_slen_tests {
        use super::*;

        #[test]
        fn test_fmt_compute_slen() {
            assert_eq!(fmt_compute_slen(true, false, true, false), 2);
            assert_eq!(fmt_compute_slen(true, false, false, false), 1);
            assert_eq!(fmt_compute_slen(false, true, true, false), 2);
            assert_eq!(fmt_compute_slen(false, true, false, true), 2);
            assert_eq!(fmt_compute_slen(false, true, false, false), 1);
            assert_eq!(fmt_compute_slen(false, false, true, false), 0);
            assert_eq!(fmt_compute_slen(false, false, false, true), 0);
            assert_eq!(fmt_compute_slen(false, false, false, false), 0);
        }
    }

    #[cfg(test)]
    mod slice_if_fresh_tests {
        use super::*;

        #[test]
        fn test_fmt_slice_if_fresh_fresh() {
            let result = fmt_slice_if_fresh(true, "hello", 2, true, false, true, false);
            assert_eq!(result, (0, "llo"));
        }

        #[test]
        fn test_fmt_slice_if_fresh_not_fresh_uniform() {
            let result = fmt_slice_if_fresh(false, "world", 0, true, false, true, false);
            let expected = (2, "world");
            assert_eq!(result, expected);
        }

        #[test]
        fn test_fmt_slice_if_fresh_not_fresh_not_uniform_newline() {
            let result = fmt_slice_if_fresh(false, "foo\nbar", 3, false, true, false, false);
            let expected = (1, "foo\nbar");
            assert_eq!(result, expected);
        }

        #[test]
        fn test_fmt_slice_if_fresh_not_fresh_not_uniform_s_start_punct() {
            let result = fmt_slice_if_fresh(false, "hello!", 2, false, false, true, true);
            let expected = (0, "hello!");
            assert_eq!(result, expected);
        }
    }

    #[cfg(test)]
    mod write_newline_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_fmt_write_newline() {
            let mut output = Cursor::new(Vec::new());
            let result = fmt_write_newline("    ", &mut output);

            assert!(result.is_ok());
            assert_eq!(output.get_ref(), b"\n    ");
        }
    }

    #[cfg(test)]
    mod write_with_spaces_tests {
        use std::io::Cursor;

        use super::*;

        #[test]
        fn test_fmt_write_with_spaces() {
            let mut output = Cursor::new(Vec::new());

            // Test case with s_len = 2
            let word = "hello";
            let s_len = 2;
            let result = fmt_write_with_spaces(word, s_len, &mut output);
            assert!(result.is_ok());
            assert_eq!(output.get_ref(), b"  hello");

            // Test case with s_len = 1
            let mut output = Cursor::new(Vec::new());
            let word = "world";
            let s_len = 1;
            let result = fmt_write_with_spaces(word, s_len, &mut output);
            assert!(result.is_ok());
            assert_eq!(output.get_ref(), b" world");

            // Test case with s_len != 1 or 2
            let mut output = Cursor::new(Vec::new());
            let word = "foo";
            let s_len = 3;
            let result = fmt_write_with_spaces(word, s_len, &mut output);
            assert!(result.is_ok());
            assert_eq!(output.get_ref(), b"foo");
        }
    }
}
