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

use std::cmp::max;
use std::str::FromStr;

use crate::ct_display::Quotable;

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct CtRange {
    pub low: usize,
    pub high: usize,
}

impl FromStr for CtRange {
    type Err = &'static str;

    /// Parse a string of the form `a-b` into a `Range`
    ///
    /// ```
    /// use std::str::FromStr;
    /// use ctcore::ct_ranges::CtRange;
    /// assert_eq!(CtRange::from_str("5"), Ok(CtRange { low: 5, high: 5 }));
    /// assert_eq!(CtRange::from_str("4-"), Ok(CtRange { low: 4, high: usize::MAX - 1 }));
    /// assert_eq!(CtRange::from_str("-4"), Ok(CtRange { low: 1, high: 4 }));
    /// assert_eq!(CtRange::from_str("2-4"), Ok(CtRange { low: 2, high: 4 }));
    /// assert!(CtRange::from_str("0-4").is_err());
    /// assert!(CtRange::from_str("4-2").is_err());
    /// assert!(CtRange::from_str("-").is_err());
    /// assert!(CtRange::from_str("a").is_err());
    /// assert!(CtRange::from_str("a-b").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, &'static str> {
        fn parse(s: &str) -> Result<usize, &'static str> {
            match s.parse::<usize>() {
                Ok(0) => Err("fields and positions are numbered from 1"),
                // GNU fails when we are at the limit. Match their behavior
                Ok(n) if n == usize::MAX => Err("byte/character offset is too large"),
                Ok(n) => Ok(n),
                Err(_) => Err("failed to parse range"),
            }
        }

        Ok(match s.split_once('-') {
            None => {
                let n = parse(s)?;
                Self { low: n, high: n }
            }
            Some(("", "")) => return Err("invalid range with no endpoint"),
            Some((low, "")) => Self {
                low: parse(low)?,
                high: usize::MAX - 1,
            },
            Some(("", high)) => Self {
                low: 1,
                high: parse(high)?,
            },
            Some((low, high)) => {
                let (low, high) = (parse(low)?, parse(high)?);
                if low <= high {
                    Self { low, high }
                } else {
                    return Err("high end of range less than low end");
                }
            }
        })
    }
}

impl CtRange {
    pub fn from_list(list: &str) -> Result<Vec<Self>, String> {
        let mut ct_ranges = Vec::new();

        for item in list.split(&[',', ' ']) {
            let range_item = FromStr::from_str(item)
                .map_err(|e| format!("range {} was invalid: {}", item.quote(), e))?;
            ct_ranges.push(range_item);
        }

        Ok(Self::merge(ct_ranges))
    }

    /// Merge any overlapping ranges
    ///
    /// Is guaranteed to return only disjoint ranges in a sorted order.
    fn merge(mut ranges: Vec<Self>) -> Vec<Self> {
        ranges.sort();

        // 合并重叠范围
        for i in 0..ranges.len() {
            let j = i + 1;

            // +1 是一个小优化，因为我们可以合并相邻的范围。
            // 例如 (1,3) 和 (4,6)，因为在整数中，3 和 4 之间没有可能的值，所以这相当于 (1,6)。
            while j < ranges.len() && ranges[j].low <= ranges[i].high + 1 {
                let j_high = ranges.remove(j).high;
                ranges[i].high = max(ranges[i].high, j_high);
            }
        }
        ranges
    }
}
/// 该函数假设输入范围是按照某种顺序给定的，不要求严格递增，但至少应该是逻辑上连续的。
pub fn complement(ranges: &[CtRange]) -> Vec<CtRange> {
    // 初始化前一个范围的高值为0，这对应于范围集合开始前的区域。
    let mut ct_prev_high = 0;
    // 初始化补集向量，预分配足够的空间以减少内存重新分配的需要。
    let mut ct_complements = Vec::with_capacity(ranges.len() + 1);

    for range in ranges {
        // 如果当前范围的低值大于前一个范围的高值加1，说明存在一个或多个遗漏的值，需要添加到补集中。
        if range.low > ct_prev_high + 1 {
            ct_complements.push(CtRange {
                low: ct_prev_high + 1,
                high: range.low - 1,
            });
        }
        // 更新前一个范围的高值为当前范围的高值，用于后续的比较。
        ct_prev_high = range.high;
    }

    // 检查是否需要添加最后一个范围之后的区域到补集中。
    if ct_prev_high < usize::MAX - 1 {
        ct_complements.push(CtRange {
            low: ct_prev_high + 1,
            high: usize::MAX - 1,
        });
    }

    ct_complements
}

/// Test if at least one of the given Ranges contain the supplied value.
///
/// Examples:
///
/// ```
/// let ranges = ctcore::ct_ranges::CtRange::from_list("11,2,6-8").unwrap();
///
/// assert!(!ctcore::ct_ranges::contain(&ranges, 0));
/// assert!(!ctcore::ct_ranges::contain(&ranges, 1));
/// assert!(!ctcore::ct_ranges::contain(&ranges, 5));
/// assert!(!ctcore::ct_ranges::contain(&ranges, 10));
///
/// assert!(ctcore::ct_ranges::contain(&ranges, 2));
/// assert!(ctcore::ct_ranges::contain(&ranges, 6));
/// assert!(ctcore::ct_ranges::contain(&ranges, 7));
/// assert!(ctcore::ct_ranges::contain(&ranges, 8));
/// assert!(ctcore::ct_ranges::contain(&ranges, 11));
/// ```
pub fn contain(ranges: &[CtRange], n: usize) -> bool {
    for range in ranges {
        if n >= range.low && n <= range.high {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod test {
    use super::{CtRange, complement};
    use std::str::FromStr;

    fn m(a: Vec<CtRange>, b: &[CtRange]) {
        assert_eq!(CtRange::merge(a), b);
    }

    fn r(low: usize, high: usize) -> CtRange {
        CtRange { low, high }
    }

    #[test]
    fn merging() {
        // Single element
        m(vec![r(1, 2)], &[r(1, 2)]);

        // Disjoint in wrong order
        m(vec![r(4, 5), r(1, 2)], &[r(1, 2), r(4, 5)]);

        // Two elements must be merged
        m(vec![r(1, 3), r(2, 4), r(6, 7)], &[r(1, 4), r(6, 7)]);

        // Two merges and a duplicate
        m(
            vec![r(1, 3), r(6, 7), r(2, 4), r(6, 7)],
            &[r(1, 4), r(6, 7)],
        );

        // One giant
        m(
            vec![
                r(110, 120),
                r(10, 20),
                r(100, 200),
                r(130, 140),
                r(150, 160),
            ],
            &[r(10, 20), r(100, 200)],
        );

        // Last one joins the previous two
        m(vec![r(10, 20), r(30, 40), r(20, 30)], &[r(10, 40)]);

        m(
            vec![r(10, 20), r(30, 40), r(50, 60), r(20, 30)],
            &[r(10, 40), r(50, 60)],
        );

        // Merge adjacent ranges
        m(vec![r(1, 3), r(4, 6)], &[r(1, 6)]);
    }

    #[test]
    fn complementing() {
        // 简单
        assert_eq!(complement(&[r(3, 4)]), vec![r(1, 2), r(5, usize::MAX - 1)]);

        // 开始
        assert_eq!(
            complement(&[r(1, 3), r(6, 10)]),
            vec![r(4, 5), r(11, usize::MAX - 1)]
        );

        // 结束
        assert_eq!(
            complement(&[r(2, 4), r(6, usize::MAX - 1)]),
            vec![r(1, 1), r(5, 5)]
        );

        // 开始和结束
        assert_eq!(complement(&[r(1, 4), r(6, usize::MAX - 1)]), vec![r(5, 5)]);

        let ranges = vec![CtRange { low: 1, high: 5 }, CtRange { low: 7, high: 10 }];
        let complements = complement(&ranges);
        assert_eq!(
            complements,
            vec![
                CtRange { low: 6, high: 6 }, // 从0到1之前
                CtRange {
                    low: 11,
                    high: 18446744073709551614
                }  // 从5到7之间
            ]
        );
    }

    #[test]
    fn test_from_str() {
        assert_eq!(CtRange::from_str("5"), Ok(CtRange { low: 5, high: 5 }));
        assert_eq!(CtRange::from_str("3-5"), Ok(CtRange { low: 3, high: 5 }));
        assert_eq!(
            CtRange::from_str("5-3"),
            Err("high end of range less than low end")
        );
        assert_eq!(
            CtRange::from_str("-"),
            Err("invalid range with no endpoint")
        );
        assert_eq!(
            CtRange::from_str("3-"),
            Ok(CtRange {
                low: 3,
                high: usize::MAX - 1
            })
        );
        assert_eq!(CtRange::from_str("-5"), Ok(CtRange { low: 1, high: 5 }));
        assert_eq!(
            CtRange::from_str("0"),
            Err("fields and positions are numbered from 1")
        );

        let max_value = format!("{}", usize::MAX);
        assert_eq!(
            CtRange::from_str(&max_value),
            Err("byte/character offset is too large")
        );
    }

    #[test]
    fn test_range_from_list() {
        let ranges = CtRange::from_list("11,2,6-8").unwrap();
        assert_eq!(ranges, vec![r(2, 2), r(6, 8), r(11, 11)]);
    }

    #[test]
    fn test_range_merge_simple() {
        assert_eq!(vec![r(1, 2)], CtRange::merge(vec![r(1, 2)]));
    }

    #[test]
    fn test_range_merge_disjoint_in_wrong_order() {
        assert_eq!(
            vec![r(1, 2), r(4, 5)],
            CtRange::merge(vec![r(4, 5), r(1, 2)])
        );
    }

    #[test]
    fn test_range_merge_two_elements_to_merge() {
        assert_eq!(
            vec![r(1, 4), r(6, 7)],
            CtRange::merge(vec![r(1, 3), r(2, 4), r(6, 7)])
        );
    }

    #[test]
    fn test_range_merge_multiple_merges_and_duplicates() {
        assert_eq!(
            vec![r(1, 4), r(6, 7)],
            CtRange::merge(vec![r(1, 3), r(6, 7), r(2, 4), r(6, 7)])
        );
    }

    #[test]
    fn test_range_merge_one_giant() {
        assert_eq!(
            vec![r(10, 20), r(100, 200)],
            CtRange::merge(vec![
                r(110, 120),
                r(10, 20),
                r(100, 200),
                r(130, 140),
                r(150, 160),
            ])
        );
    }

    #[test]
    fn test_range_merge_adjacent_ranges() {
        assert_eq!(vec![r(1, 6)], CtRange::merge(vec![r(1, 3), r(4, 6)]));
    }

    #[test]
    fn test_range_complement_simple() {
        assert_eq!(vec![r(1, 2), r(5, usize::MAX - 1)], complement(&[r(3, 4)]));
    }

    #[test]
    fn test_range_complement_with_start() {
        assert_eq!(
            vec![r(4, 5), r(11, usize::MAX - 1)],
            complement(&[r(1, 3), r(6, 10)])
        );
    }

    #[test]
    fn test_range_complement_with_end() {
        assert_eq!(
            vec![r(1, 1), r(5, 5)],
            complement(&[r(2, 4), r(6, usize::MAX - 1)])
        );
    }

    #[test]
    fn test_range_complement_with_start_and_end() {
        assert_eq!(vec![r(5, 5)], complement(&[r(1, 4), r(6, usize::MAX - 1)]));
    }
}
