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
//!
//! Use the [`Number`] enum to represent an arbitrary number in an
//! arbitrary radix. A number can be incremented and can be
//! displayed. See the [`Number`] documentation for more information.
//!
//! See the Wikipedia articles on [radix] and [positional notation]
//! for more background information on those topics.
//!
//! [radix]: https://en.wikipedia.org/wiki/Radix
//! [positional notation]: https://en.wikipedia.org/wiki/Positional_notation
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// An overflow due to incrementing a number beyond its representable limit.
#[derive(Debug)]
pub struct NumberOverflow;

impl fmt::Display for NumberOverflow {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Overflow")
    }
}

impl Error for NumberOverflow {}

/// A number in arbitrary radix expressed in a positional notation.
///
/// Use the [`Number`] enum to represent an arbitrary number in an
/// arbitrary radix. A number can be incremented with
/// [`Number::number_increment`].  The [`NumberFixedWidthNumber`] overflows when
/// attempting to increment it beyond the maximum number that can be
/// represented in the specified width. The [`DynamicWidthNumber`]
/// follows a non-standard incrementing procedure that is used
/// specifically for the `split` program. See the
/// [`DynamicWidthNumber`] documentation for more information.
///
/// Numbers of radix
///
/// * 10 are displayable and rendered as decimal numbers (for example,
///   "00" or "917"),
/// * 16 are displayable and rendered as hexadecimal numbers (for example,
///   "00" or "e7f"),
/// * 26 are displayable and rendered as lowercase ASCII alphabetic
///   characters (for example, "aa" or "zax").
///
/// Numbers of other radices cannot be displayed. The display of a
/// [`DynamicWidthNumber`] includes a prefix whose length depends on
/// the width of the number. See the [`DynamicWidthNumber`]
/// documentation for more information.
///
/// The digits of a number are accessible via the [`Number::number_digits`]
/// method. The digits are represented as a [`Vec<u8>`] with the most
/// significant digit on the left and the least significant digit on
/// the right. Each digit is a nonnegative integer less than the
/// radix. For example, if the radix is 3, then `vec![1, 0, 2]`
/// represents the decimal number 11:
///
/// ```ignore
/// 1 * 3^2 + 0 * 3^1 + 2 * 3^0 = 9 + 0 + 2 = 11
/// ```
///
/// For the [`DynamicWidthNumber`], the digits are not unique in the
/// sense that repeatedly incrementing the number will eventually
/// yield `vec![0, 0]`, `vec![0, 0, 0]`, `vec![0, 0, 0, 0]`, etc.
/// That's okay because each of these numbers will be displayed
/// differently and we only intend to use these numbers for display
/// purposes and not for mathematical purposes.
#[derive(Clone)]
pub enum Number {
    /// A fixed-width representation of a number.
    FixedWidth(NumberFixedWidthNumber),

    /// A representation of a number with a dynamically growing width.
    DynamicWidth(DynamicWidthNumber),
}

impl Number {
    /// The digits of this number in decreasing order of significance.
    ///
    /// The digits are represented as a [`Vec<u8>`] with the most
    /// significant digit on the left and the least significant digit
    /// on the right. Each digit is a nonnegative integer less than
    /// the radix. For example, if the radix is 3, then `vec![1, 0,
    /// 2]` represents the decimal number 11:
    ///
    /// ```ignore
    /// 1 * 3^2 + 0 * 3^1 + 2 * 3^0 = 9 + 0 + 2 = 11
    /// ```
    ///
    /// For the [`DynamicWidthNumber`], the digits are not unique in the
    /// sense that repeatedly incrementing the number will eventually
    /// yield `vec![0, 0]`, `vec![0, 0, 0]`, `vec![0, 0, 0, 0]`, etc.
    /// That's okay because each of these numbers will be displayed
    /// differently and we only intend to use these numbers for display
    /// purposes and not for mathematical purposes.
    #[allow(dead_code)]
    fn number_digits(&self) -> Vec<u8> {
        match self {
            Self::FixedWidth(number) => number.digits.clone(),
            Self::DynamicWidth(number) => number.digits(),
        }
    }

    /// Increment this number to its successor.
    ///
    /// If incrementing this number would result in an overflow beyond
    /// the maximum representable number, then return
    /// [`Err(Overflow)`]. The [`NumberFixedWidthNumber`] overflows, but
    /// [`DynamicWidthNumber`] does not.
    ///
    /// The [`DynamicWidthNumber`] follows a non-standard incrementing
    /// procedure that is used specifically for the `split` program.
    /// See the [`DynamicWidthNumber`] documentation for more
    /// information.
    ///
    /// # Errors
    ///
    /// This method returns [`Err(Overflow)`] when attempting to
    /// increment beyond the largest representable number.
    ///
    /// # Examples
    ///
    /// Overflowing:
    ///
    /// ```rust,ignore
    ///
    /// use crate::number::FixedWidthNumber;
    /// use crate::number::Number;
    /// use crate::number::Overflow;
    ///
    /// // Radix 3, width of 1 digit.
    /// let mut number = Number::FixedWidth(FixedWidthNumber::new(3, 1));
    /// number.increment().unwrap();  // from 0 to 1
    /// number.increment().unwrap();  // from 1 to 2
    /// assert!(number.increment().is_err());
    /// ```
    pub fn number_increment(&mut self) -> Result<(), NumberOverflow> {
        match self {
            Self::FixedWidth(number) => number.number_increment(),
            Self::DynamicWidth(number) => number.increment(),
        }
    }
}

impl Display for Number {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::FixedWidth(number) => number.fmt(f),
            Self::DynamicWidth(number) => number.fmt(f),
        }
    }
}

/// A positional notation representation of a fixed-width number.
///
/// The digits are represented as a [`Vec<u8>`] with the most
/// significant digit on the left and the least significant digit on
/// the right. Each digit is a nonnegative integer less than the
/// radix.
///
/// # Incrementing
///
/// This number starts at `vec![0; width]`, representing the number 0
/// width the specified number of digits. Incrementing this number
/// with [`Number::number_increment`] causes it to increase its value by 1 in
/// the usual sense. If the digits are `vec![radix - 1; width]`, then
/// an overflow would occur and the [`Number::number_increment`] method
/// returns an error.
///
/// # Displaying
///
/// This number is only displayable if `radix` is 10, 16, or 26. If
/// `radix` is 10 or 16, then the digits are concatenated and
/// displayed as a fixed-width decimal or hexadecimal number,
/// respectively. If `radix` is 26, then each digit is translated to
/// the corresponding lowercase ASCII alphabetic character (that is,
/// 'a', 'b', 'c', etc.) and concatenated.
#[derive(Clone)]
pub struct NumberFixedWidthNumber {
    radix: u8,
    digits: Vec<u8>,
}

impl NumberFixedWidthNumber {
    /// Instantiate a number of the given radix and width.
    pub fn new(radix: u8, width: usize, mut suffix_start: usize) -> Result<Self, NumberOverflow> {
        let mut digits = vec![0_u8; width];

        for size in (0..digits.len()).rev() {
            let remainder = (suffix_start % (radix as usize)) as u8;
            suffix_start /= radix as usize;
            digits[size] = remainder;
            if suffix_start == 0 {
                break;
            }
        }
        if suffix_start == 0 {
            Ok(Self { radix, digits })
        } else {
            Err(NumberOverflow)
        }
    }

    /// Increment this number.
    ///
    /// This method adds one to this number. If incrementing this
    /// number would require more digits than are available with the
    /// specified width, then this method returns [`Err(Overflow)`].
    fn number_increment(&mut self) -> Result<(), NumberOverflow> {
        for size in (0..self.digits.len()).rev() {
            // Increment the current digit.
            self.digits[size] += 1;

            // 若当前位发生溢出，则将其置零并继续下一次循环，以递增下一个更高权重的位。
            // 否则，终止循环，因为后续不会再对任何高位产生变化。
            if self.digits[size] == self.radix {
                self.digits[size] = 0;
            } else {
                break;
            }
        }

        // 当发生溢出（表现为所有位均为0）时，返回错误。
        if self.digits == vec![0; self.digits.len()] {
            Err(NumberOverflow)
        } else {
            Ok(())
        }
    }
}

impl Display for NumberFixedWidthNumber {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let digits: String = self
            .digits
            .iter()
            .map(|d| map_digit(self.radix, *d))
            .collect();
        write!(f, "{digits}")
    }
}

/// A positional notation representation of a number of dynamically growing width.
///
/// The digits are represented as a [`Vec<u8>`] with the most
/// significant digit on the left and the least significant digit on
/// the right. Each digit is a nonnegative integer less than the
/// radix.
///
/// # Incrementing
///
/// This number starts at `vec![0, 0]`, representing the number 0 with
/// a width of 2 digits. Incrementing this number with
/// [`Number::number_increment`] causes it to increase its value by 1. When
/// incrementing the number would have caused it to change from
/// `vec![radix - 2, radix - 1]` to `vec![radix - 1, 0]`, it instead
/// increases its width by one and resets its value to 0. For example,
/// if the radix were 3, the digits were `vec![1, 2]`, and we called
/// [`Number::number_increment`], then the digits would become `vec![0, 0,
/// 0]`. In this way, the width grows by one each time the most
/// significant digit would have achieved its maximum value.
///
/// This notion of "incrementing" here does not match the notion of
/// incrementing the *value* of the number, it is just an abstract way
/// of updating the representation of the number in a way that is only
/// useful for the purposes of the `split` program.
///
/// # Displaying
///
/// This number is only displayable if `radix` is 10, 16, or 26. If
/// `radix` is 10 or 16, then the digits are concatenated and
/// displayed as a fixed-width decimal or hexadecimal number,
/// respectively, with a prefix of `n - 2` instances of the character
/// '9' of 'f', respectively, where `n` is the number of digits.  If
/// `radix` is 26, then each digit is translated to the corresponding
/// lowercase ASCII alphabetic character (that is, 'a', 'b', 'c',
/// etc.) and concatenated with a prefix of `n - 2` instances of the
/// character 'z'.
///
/// This notion of displaying the number is specific to the `split`
/// program.
#[derive(Clone)]
pub struct DynamicWidthNumber {
    radix: u8,
    current: usize,
}

impl DynamicWidthNumber {
    pub fn new(radix: u8, suffix_start: usize) -> Self {
        Self {
            radix,
            current: suffix_start,
        }
    }

    fn increment(&mut self) -> Result<(), NumberOverflow> {
        self.current += 1;
        Ok(())
    }

    fn digits(&self) -> Vec<u8> {
        let radix_szie = self.radix as usize;
        let mut remaining_size = self.current;
        let mut sub_size = (radix_szie - 1) * radix_szie;
        let mut num_fill_chars = 2;

        // Convert the number into "num_fill_chars" and "remaining"
        while remaining_size >= sub_size {
            remaining_size -= sub_size;
            sub_size *= radix_szie;
            num_fill_chars += 1;
        }

        // Convert the "remainder" to digits
        let mut digits = Vec::new();
        while remaining_size > 0 {
            digits.push((remaining_size % radix_szie) as u8);
            remaining_size /= radix_szie;
        }
        // Left pad the vec
        digits.resize(num_fill_chars, 0);
        digits.reverse();
        digits
    }
}

fn map_digit(radix: u8, d: u8) -> char {
    (match radix {
        10 => b'0' + d,
        16 => {
            if d < 10 {
                b'0' + d
            } else {
                b'a' + (d - 10)
            }
        }
        26 => b'a' + d,
        _ => 0,
    }) as char
}

impl Display for DynamicWidthNumber {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let digits: String = self
            .digits()
            .iter()
            .map(|d| map_digit(self.radix, *d))
            .collect();
        let fill: String = (0..digits.len() - 2)
            .map(|_| map_digit(self.radix, self.radix - 1))
            .collect();
        write!(f, "{fill}{digits}")
    }
}

