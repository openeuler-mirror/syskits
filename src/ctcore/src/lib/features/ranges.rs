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

// spell-checker:ignore (ToDO) inval

use std::cmp::max;
use std::str::FromStr;

use crate::ct_display::Quotable;

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Range {
    pub low: usize,
    pub high: usize,
}

impl FromStr for Range {
    type Err = &'static str;

    /// Parse a string of the form `a-b` into a `Range`
    ///
    /// ```
    /// use std::str::FromStr;
    /// use ctcore::ranges::Range;
    /// assert_eq!(Range::from_str("5"), Ok(Range { low: 5, high: 5 }));
    /// assert_eq!(Range::from_str("4-"), Ok(Range { low: 4, high: usize::MAX - 1 }));
    /// assert_eq!(Range::from_str("-4"), Ok(Range { low: 1, high: 4 }));
    /// assert_eq!(Range::from_str("2-4"), Ok(Range { low: 2, high: 4 }));
    /// assert!(Range::from_str("0-4").is_err());
    /// assert!(Range::from_str("4-2").is_err());
    /// assert!(Range::from_str("-").is_err());
    /// assert!(Range::from_str("a").is_err());
    /// assert!(Range::from_str("a-b").is_err());
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

impl Range {
    pub fn from_list(list: &str) -> Result<Vec<Self>, String> {
        let mut ranges = Vec::new();

        for item in list.split(&[',', ' ']) {
            let range_item = FromStr::from_str(item)
                .map_err(|e| format!("range {} was invalid: {}", item.quote(), e))?;
            ranges.push(range_item);
        }

        Ok(Self::merge(ranges))
    }

    /// Merge any overlapping ranges
    ///
    /// Is guaranteed to return only disjoint ranges in a sorted order.
    fn merge(mut ranges: Vec<Self>) -> Vec<Self> {
        ranges.sort();

        // merge overlapping ranges
        for i in 0..ranges.len() {
            let j = i + 1;

            // The +1 is a small optimization, because we can merge adjacent Ranges.
            // For example (1,3) and (4,6), because in the integers, there are no
            // possible values between 3 and 4, this is equivalent to (1,6).
            while j < ranges.len() && ranges[j].low <= ranges[i].high + 1 {
                let j_high = ranges.remove(j).high;
                ranges[i].high = max(ranges[i].high, j_high);
            }
        }
        ranges
    }
}

pub fn complement(ranges: &[Range]) -> Vec<Range> {
    let mut prev_high = 0;
    let mut complements = Vec::with_capacity(ranges.len() + 1);

    for range in ranges {
        if range.low > prev_high + 1 {
            complements.push(Range {
                low: prev_high + 1,
                high: range.low - 1,
            });
        }
        prev_high = range.high;
    }

    if prev_high < usize::MAX - 1 {
        complements.push(Range {
            low: prev_high + 1,
            high: usize::MAX - 1,
        });
    }

    complements
}

/// Test if at least one of the given Ranges contain the supplied value.
///
/// Examples:
///
/// ```
/// let ranges = ctcore::ranges::Range::from_list("11,2,6-8").unwrap();
///
/// assert!(!ctcore::ranges::contain(&ranges, 0));
/// assert!(!ctcore::ranges::contain(&ranges, 1));
/// assert!(!ctcore::ranges::contain(&ranges, 5));
/// assert!(!ctcore::ranges::contain(&ranges, 10));
///
/// assert!(ctcore::ranges::contain(&ranges, 2));
/// assert!(ctcore::ranges::contain(&ranges, 6));
/// assert!(ctcore::ranges::contain(&ranges, 7));
/// assert!(ctcore::ranges::contain(&ranges, 8));
/// assert!(ctcore::ranges::contain(&ranges, 11));
/// ```
pub fn contain(ranges: &[Range], n: usize) -> bool {
    for range in ranges {
        if n >= range.low && n <= range.high {
            return true;
        }
    }

    false
}

