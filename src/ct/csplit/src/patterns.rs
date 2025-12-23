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

use crate::csplit_error::CsplitError;
use ctcore::ct_show_warning;
use regex::Regex;

/// The definition of a pattern to match on a line.
#[derive(Debug)]
pub enum CsplitPattern {
    /// Copy the file's content to a split up to, not including, the given line number. The number
    /// of times the pattern is executed is detailed in [`CsplitExecutePattern`].
    UpToLine(usize, CsplitExecutePattern),
    /// Copy the file's content to a split up to, not including, the line matching the regex. The
    /// integer is an offset relative to the matched line of what to include (if positive) or
    /// to exclude (if negative). The number of times the pattern is executed is detailed in
    /// [`CsplitExecutePattern`].
    UpToMatch(Regex, i32, CsplitExecutePattern),
    /// Skip the file's content up to, not including, the line matching the regex. The integer
    /// is an offset relative to the matched line of what to include (if positive) or to exclude
    /// (if negative). The number of times the pattern is executed is detailed in [`CsplitExecutePattern`].
    SkipToMatch(Regex, i32, CsplitExecutePattern),
}

impl ToString for CsplitPattern {
    fn to_string(&self) -> String {
        match self {
            Self::UpToLine(n, _) => n.to_string(),
            Self::UpToMatch(regex, 0, _) => format!("/{}/", regex.as_str()),
            Self::UpToMatch(regex, offset, _) => format!("/{}/{:+}", regex.as_str(), offset),
            Self::SkipToMatch(regex, 0, _) => format!("%{}%", regex.as_str()),
            Self::SkipToMatch(regex, offset, _) => format!("%{}%{:+}", regex.as_str(), offset),
        }
    }
}

/// The number of times a pattern can be used.
#[derive(Debug)]
pub enum CsplitExecutePattern {
    /// Execute the pattern as many times as possible
    Always,
    /// Execute the pattern a fixed number of times
    Times(usize),
}

impl CsplitExecutePattern {
    pub fn iter(&self) -> CsplitExecutePatternIter {
        match self {
            Self::Times(n) => CsplitExecutePatternIter::new(Some(*n)),
            Self::Always => CsplitExecutePatternIter::new(None),
        }
    }
}

pub struct CsplitExecutePatternIter {
    max: Option<usize>,
    cur: usize,
}

impl CsplitExecutePatternIter {
    fn new(max: Option<usize>) -> Self {
        Self { max, cur: 0 }
    }
}

impl Iterator for CsplitExecutePatternIter {
    type Item = (Option<usize>, usize);

    fn next(&mut self) -> Option<(Option<usize>, usize)> {
        match self.max {
            // iterate until m is reached
            Some(m) => {
                if self.cur == m {
                    None
                } else {
                    self.cur += 1;
                    Some((self.max, self.cur))
                }
            }
            // no limit, just increment a counter
            None => {
                self.cur += 1;
                Some((None, self.cur))
            }
        }
    }
}

/// Parses the definitions of patterns given on the command line into a list of [`CsplitPattern`]s.
///
/// # Errors
///
/// If a pattern is incorrect, a [`CsplitError::InvalidPattern`] error is returned, which may be
/// due to, e.g.,:
/// - an invalid regular expression;
/// - an invalid number for, e.g., the offset.
pub fn get_patterns(args: &[String]) -> Result<Vec<CsplitPattern>, CsplitError> {
    let csplit_patterns = extract_patterns(args)?;
    validate_line_numbers(&csplit_patterns)?;
    Ok(csplit_patterns)
}

/**
 * 从命令行参数中提取CSplit模式。
 *
 * 此函数解析一系列字符串参数，并将它们转换为CSplit操作所需的模式集合。
 * 每个参数可以是一个正则表达式模式、行号或者控制重复次数的特殊格式字符串。
 *
 * @param args 需要解析的命令行参数，为一个字符串切片。
 * @return Result<Vec<CsplitPattern>, CsplitError> 如果解析成功，返回一个包含CSplit模式的向量；
 *         如果遇到错误，返回一个包含错误信息的CsplitError。
 */
fn extract_patterns(args: &[String]) -> Result<Vec<CsplitPattern>, CsplitError> {
    // 初始化用于储存解析后模式的向量
    let mut csplit_patterns = Vec::with_capacity(args.len());

    // 正则表达式用于匹配模式中的"upto"、"skipto"和可选的偏移量
    let to_match_reg =
        Regex::new(r"^(/(?P<UPTO>.+)/|%(?P<SKIPTO>.+)%)(?P<OFFSET>[\+-]\d+)?$").unwrap();
    // 正则表达式用于匹配模式重复的次数
    let execute_n_times_reg = Regex::new(r"^\{(?P<TIMES>\d+)|\*\}$").unwrap();

    // 使用peekable迭代器以便于向前查看下一个参数
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        // 解析模式的重复执行次数
        let execute_n_times = match iter.peek() {
            None => CsplitExecutePattern::Times(1),
            Some(&next_item) => {
                match execute_n_times_reg.captures(next_item) {
                    None => CsplitExecutePattern::Times(1),
                    Some(r) => {
                        // 跳过当前参数，因为它已经被用于重复次数的解析
                        iter.next();
                        if let Some(times) = r.name("TIMES") {
                            CsplitExecutePattern::Times(
                                times.as_str().parse::<usize>().unwrap() + 1,
                            )
                        } else {
                            CsplitExecutePattern::Always
                        }
                    }
                }
            }
        };

        // 解析模式定义
        if let Some(captures) = to_match_reg.captures(arg) {
            // 解析偏移量
            let offset = match captures.name("OFFSET") {
                None => 0,
                Some(m) => m.as_str().parse().unwrap(),
            };
            // 根据匹配的类型，构建相应的模式
            if let Some(up_to_match) = captures.name("UPTO") {
                let pattern = Regex::new(up_to_match.as_str())
                    .map_err(|_| CsplitError::InvalidPattern(arg.to_string()))?;
                csplit_patterns.push(CsplitPattern::UpToMatch(pattern, offset, execute_n_times));
            } else if let Some(skip_to_match) = captures.name("SKIPTO") {
                let pattern = Regex::new(skip_to_match.as_str())
                    .map_err(|_| CsplitError::InvalidPattern(arg.to_string()))?;
                csplit_patterns.push(CsplitPattern::SkipToMatch(pattern, offset, execute_n_times));
            }
        } else if let Ok(line_number) = arg.parse::<usize>() {
            // 如果参数是行号，构建相应的行号模式
            csplit_patterns.push(CsplitPattern::UpToLine(line_number, execute_n_times));
        } else {
            // 如果参数无法解析为有效的模式，返回错误
            return Err(CsplitError::InvalidPattern(arg.to_string()));
        }
    }
    // 成功解析所有参数后，返回模式向量
    Ok(csplit_patterns)
}

/// Asserts the line numbers are in increasing order, starting at 1.
/**
 * 验证拆分模式中涉及的行号是否有效。
 *
 * 此函数遍历提供的拆分模式列表，检查行号是否满足以下条件：
 * 1. 行号不能为零。
 * 2. 相邻的行号不能相等。
 * 3. 行号必须按升序出现。
 *
 * @param patterns 指向CsplitPattern结构体 slice的引用，表示待验证的拆分模式。
 * @return Result<(), CsplitError> 如果所有行号都有效，则返回Ok(())；如果发现任何无效行号，则返回Err，携带相应的错误信息。
 */
fn validate_line_numbers(csplit_patterns: &[CsplitPattern]) -> Result<(), CsplitError> {
    // 过滤并映射出所有以行号为条件的拆分模式，忽略其他类型。
    csplit_patterns
        .iter()
        .filter_map(|pattern| match pattern {
            CsplitPattern::UpToLine(line_number, _) => Some(line_number),
            _ => None,
        })
        // 通过try_fold迭代验证行号，确保它们按照升序且没有重复。
        .try_fold(0, |prev_ln, &current_ln| match (prev_ln, current_ln) {
            // 行号不能为零。
            (_, 0) => Err(CsplitError::LineNumberIsZero),
            // 相邻的行号不能相等。
            (n, m) if n == m => {
                ct_show_warning!("line number '{}' is the same as preceding line number", n);
                Ok(n)
            }
            // 行号必须按升序出现。
            (n, m) if n > m => Err(CsplitError::LineNumberSmallerThanPrevious(m, n)),
            (_, m) => Ok(m),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bad_pattern() {
        let input = vec!["bad".to_string()];
        assert!(get_patterns(input.as_slice()).is_err());
    }

    #[test]
    fn test_up_to_line_pattern() {
        let input: Vec<String> = vec!["24", "42", "{*}", "50", "{4}"]
            .into_iter()
            .map(|v| v.to_string())
            .collect();
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 3);
        match patterns.first() {
            Some(CsplitPattern::UpToLine(24, CsplitExecutePattern::Times(1))) => (),
            _ => panic!("expected UpToLine pattern"),
        };
        match patterns.get(1) {
            Some(CsplitPattern::UpToLine(42, CsplitExecutePattern::Always)) => (),
            _ => panic!("expected UpToLine pattern"),
        };
        match patterns.get(2) {
            Some(CsplitPattern::UpToLine(50, CsplitExecutePattern::Times(5))) => (),
            _ => panic!("expected UpToLine pattern"),
        };
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_up_to_match_pattern() {
        let input: Vec<String> = vec![
            "/test1.*end$/",
            "/test2.*end$/",
            "{*}",
            "/test3.*end$/",
            "{4}",
            "/test4.*end$/+3",
            "/test5.*end$/-3",
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect();
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 5);
        match patterns.first() {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test1.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(1) {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Always)) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test2.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(2) {
            Some(CsplitPattern::UpToMatch(reg, 0, CsplitExecutePattern::Times(5))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test3.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(3) {
            Some(CsplitPattern::UpToMatch(reg, 3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test4.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
        match patterns.get(4) {
            Some(CsplitPattern::UpToMatch(reg, -3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test5.*end$");
            }
            _ => panic!("expected UpToMatch pattern"),
        };
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_skip_to_match_pattern() {
        let input: Vec<String> = vec![
            "%test1.*end$%",
            "%test2.*end$%",
            "{*}",
            "%test3.*end$%",
            "{4}",
            "%test4.*end$%+3",
            "%test5.*end$%-3",
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect();
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 5);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test1.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(1) {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Always)) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test2.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(2) {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Times(5))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test3.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(3) {
            Some(CsplitPattern::SkipToMatch(reg, 3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test4.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
        match patterns.get(4) {
            Some(CsplitPattern::SkipToMatch(reg, -3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test5.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_skip_to_match_pattern_test1() {
        let input: Vec<String> = vec!["%test1.*end$%".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, 0, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test1.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }
    #[test]
    fn test_skip_to_match_pattern_test2() {
        let input: Vec<String> = vec!["%test2.*end$%".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);

        // match patterns.first() {
        //     Some(Pattern::SkipToMatch(reg, 0, ExecutePattern::Always)) => {
        //         let parsed_reg = format!("{reg}");
        //         assert_eq!(parsed_reg, "test2.*end$");
        //     }
        //     _ => panic!("expected SkipToMatch pattern"),
        // };
    }
    #[test]
    fn test_skip_to_match_pattern_test3() {
        let input: Vec<String> = vec!["%test3.*end$%".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        // match patterns.first() {
        //     Some(Pattern::SkipToMatch(reg, 0, ExecutePattern::Times(5))) => {
        //         let parsed_reg = format!("{reg}");
        //         assert_eq!(parsed_reg, "test3.*end$");
        //     }
        //     _ => panic!("expected SkipToMatch pattern"),
        // };
    }
    #[test]
    fn test_skip_to_match_pattern_test4() {
        let input: Vec<String> = vec!["%test4.*end$%+3".parse().unwrap()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, 3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test4.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_skip_to_match_pattern_test5() {
        let input: Vec<String> = vec!["%test5.*end$%-3".to_string()];
        let patterns = get_patterns(input.as_slice()).unwrap();
        assert_eq!(patterns.len(), 1);
        match patterns.first() {
            Some(CsplitPattern::SkipToMatch(reg, -3, CsplitExecutePattern::Times(1))) => {
                let parsed_reg = format!("{reg}");
                assert_eq!(parsed_reg, "test5.*end$");
            }
            _ => panic!("expected SkipToMatch pattern"),
        };
    }

    #[test]
    fn test_line_number_zero() {
        let patterns = vec![CsplitPattern::UpToLine(0, CsplitExecutePattern::Times(1))];
        match validate_line_numbers(&patterns) {
            Err(CsplitError::LineNumberIsZero) => (),
            _ => panic!("expected LineNumberIsZero error"),
        }
    }

    #[test]
    fn test_line_number_smaller_than_previous() {
        let input: Vec<String> = vec!["10".to_string(), "5".to_string()];
        match get_patterns(input.as_slice()) {
            Err(CsplitError::LineNumberSmallerThanPrevious(5, 10)) => (),
            _ => panic!("expected LineNumberSmallerThanPrevious error"),
        }
    }

    #[test]
    fn test_line_number_smaller_than_previous_separate() {
        let input: Vec<String> = vec!["10".to_string(), "/20/".to_string(), "5".to_string()];
        match get_patterns(input.as_slice()) {
            Err(CsplitError::LineNumberSmallerThanPrevious(5, 10)) => (),
            _ => panic!("expected LineNumberSmallerThanPrevious error"),
        }
    }
}