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

/// The first ten powers of 1024.
const IEC_BASES: [u128; 10] = [
    1,
    1_024,
    1_048_576,
    1_073_741_824,
    1_099_511_627_776,
    1_125_899_906_842_624,
    1_152_921_504_606_846_976,
    1_180_591_620_717_411_303_424,
    1_208_925_819_614_629_174_706_176,
    1_237_940_039_285_380_274_899_124_224,
];

const IEC_SUFFIXES: [&str; 9] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB", "ZiB", "YiB"];

/// The first ten powers of 1000.
const SI_BASES: [u128; 10] = [
    1,
    1_000,
    1_000_000,
    1_000_000_000,
    1_000_000_000_000,
    1_000_000_000_000_000,
    1_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000_000,
    1_000_000_000_000_000_000_000_000_000,
];

const SI_SUFFIXES: [&str; 9] = ["B", "kB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];

/// A SuffixType determines whether the suffixes are 1000 or 1024 based.
#[derive(Clone, Copy)]
pub(crate) enum SuffixType {
    Iec,
    Si,
}

impl SuffixType {
    fn properties(&self) -> (u128, &[u128], &[&'static str]) {
        match self {
            Self::Iec => (1_024, &IEC_BASES, &IEC_SUFFIXES),
            Self::Si => (1_000, &SI_BASES, &SI_SUFFIXES),
        }
    }

    fn base_and_suffix(&self, n: u128) -> (u128, &'static str, usize) {
        let (_, bases, suffixes) = self.properties();
        let mut i = 0;
        while i + 1 < suffixes.len() && bases[i + 1] <= n {
            i += 1;
        }
        (bases[i], suffixes[i], i)
    }
}

fn round_half_even(quotient: u128, remainder: u128, divisor: u128) -> u128 {
    match remainder.cmp(&(divisor - remainder)) {
        std::cmp::Ordering::Less => quotient,
        std::cmp::Ordering::Greater => quotient + 1,
        std::cmp::Ordering::Equal => quotient + (quotient & 1),
    }
}

fn format_ratio(numerator: u128, denominator: u128, suffix_type: SuffixType) -> String {
    let (unit_base, bases, suffixes) = suffix_type.properties();
    let mut exponent = if denominator == 1 {
        suffix_type.base_and_suffix(numerator).2
    } else {
        0
    };
    if denominator != 1 {
        while exponent + 1 < suffixes.len() {
            let Some(threshold) = denominator.checked_mul(bases[exponent + 1]) else {
                break;
            };
            if numerator < threshold {
                break;
            }
            exponent += 1;
        }
    }

    let divisor = denominator * bases[exponent];
    let quotient = numerator / divisor;
    let remainder = numerator % divisor;

    if exponent > 0 && quotient < 10 {
        let remainder_tenths = remainder * 10;
        let tenths = round_half_even(
            quotient * 10 + remainder_tenths / divisor,
            remainder_tenths % divisor,
            divisor,
        );
        if tenths < 100 {
            return format!("{}.{:01} {}", tenths / 10, tenths % 10, suffixes[exponent]);
        }
        return format!("{} {}", tenths / 10, suffixes[exponent]);
    }

    let mut amount = round_half_even(quotient, remainder, divisor);
    if exponent + 1 < suffixes.len() && amount == unit_base {
        exponent += 1;
        amount = 1;
        return format!("{amount}.0 {}", suffixes[exponent]);
    }
    format!("{amount} {}", suffixes[exponent])
}

/// Convert a number into a magnitude and a multi-byte unit suffix.
///
/// The returned string has a maximum length of 5 chars, for example: "1.1kB", "999kB", "1MB".
pub(crate) fn to_magnitude_and_suffix(n: u128, suffix_type: SuffixType) -> String {
    format_ratio(n, 1, suffix_type)
}

pub(crate) fn to_rate_and_suffix(n: u128, duration_nanos: u128) -> String {
    const NANOS_PER_SECOND: u128 = 1_000_000_000;

    if duration_nanos == 0 {
        return "Infinity B".to_string();
    }

    if duration_nanos <= NANOS_PER_SECOND
        && NANOS_PER_SECOND % duration_nanos == 0
        && let Some(amount) = n.checked_mul(NANOS_PER_SECOND / duration_nanos)
    {
        return format_ratio(amount, 1, SuffixType::Si);
    }
    if duration_nanos % NANOS_PER_SECOND == 0 {
        return format_ratio(n, duration_nanos / NANOS_PER_SECOND, SuffixType::Si);
    }

    // GNU falls back to floating point for a non-integral block-size ratio.
    // Its autoscale loop selects at least the kilo prefix in that branch.
    let (base, _, suffixes) = SuffixType::Si.properties();
    let rate = n as f64 * (NANOS_PER_SECOND as f64 / duration_nanos as f64);
    let mut scale = base as f64;
    let mut exponent = 1;
    while exponent + 1 < suffixes.len() && scale * base as f64 <= rate {
        scale *= base as f64;
        exponent += 1;
    }
    let magnitude = rate / scale;
    let decimal = format!("{magnitude:.1}");
    let amount = if decimal.split('.').next().unwrap_or_default().len() >= 3 {
        format!("{magnitude:.0}")
    } else {
        decimal
    };
    format!("{amount} {}", suffixes[exponent])
}

#[cfg(test)]
mod tests {
    use crate::numbers::{SuffixType, to_magnitude_and_suffix, to_rate_and_suffix};

    #[test]
    fn test_to_magnitude_and_suffix_powers_of_1024() {
        assert_eq!(to_magnitude_and_suffix(1024, SuffixType::Iec), "1.0 KiB");
        assert_eq!(to_magnitude_and_suffix(2048, SuffixType::Iec), "2.0 KiB");
        assert_eq!(to_magnitude_and_suffix(4096, SuffixType::Iec), "4.0 KiB");
        assert_eq!(
            to_magnitude_and_suffix(1024 * 1024, SuffixType::Iec),
            "1.0 MiB"
        );
        assert_eq!(
            to_magnitude_and_suffix(2 * 1024 * 1024, SuffixType::Iec),
            "2.0 MiB"
        );
        assert_eq!(
            to_magnitude_and_suffix(1024 * 1024 * 1024, SuffixType::Iec),
            "1.0 GiB"
        );
        assert_eq!(
            to_magnitude_and_suffix(34 * 1024 * 1024 * 1024, SuffixType::Iec),
            "34 GiB"
        );
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_to_magnitude_and_suffix_not_powers_of_1024() {
        assert_eq!(to_magnitude_and_suffix(1, SuffixType::Si), "1 B");
        assert_eq!(to_magnitude_and_suffix(999, SuffixType::Si), "999 B");

        assert_eq!(to_magnitude_and_suffix(1000, SuffixType::Si), "1.0 kB");
        assert_eq!(to_magnitude_and_suffix(1001, SuffixType::Si), "1.0 kB");
        assert_eq!(to_magnitude_and_suffix(1023, SuffixType::Si), "1.0 kB");
        assert_eq!(to_magnitude_and_suffix(1025, SuffixType::Si), "1.0 kB");
        assert_eq!(to_magnitude_and_suffix(1050, SuffixType::Si), "1.0 kB");
        assert_eq!(to_magnitude_and_suffix(1051, SuffixType::Si), "1.1 kB");
        assert_eq!(to_magnitude_and_suffix(1150, SuffixType::Si), "1.2 kB");
        assert_eq!(to_magnitude_and_suffix(10_500, SuffixType::Si), "10 kB");
        assert_eq!(to_magnitude_and_suffix(10_501, SuffixType::Si), "11 kB");
        assert_eq!(to_magnitude_and_suffix(10_001, SuffixType::Si), "10 kB");
        assert_eq!(to_magnitude_and_suffix(999_000, SuffixType::Si), "999 kB");

        assert_eq!(to_magnitude_and_suffix(999_001, SuffixType::Si), "999 kB");
        assert_eq!(to_magnitude_and_suffix(999_499, SuffixType::Si), "999 kB");
        assert_eq!(to_magnitude_and_suffix(999_500, SuffixType::Si), "1.0 MB");
        assert_eq!(to_magnitude_and_suffix(999_999, SuffixType::Si), "1.0 MB");
        assert_eq!(to_magnitude_and_suffix(1_000_000, SuffixType::Si), "1.0 MB");
        assert_eq!(to_magnitude_and_suffix(1_000_001, SuffixType::Si), "1.0 MB");
        assert_eq!(to_magnitude_and_suffix(1_100_000, SuffixType::Si), "1.1 MB");
        assert_eq!(to_magnitude_and_suffix(1_100_001, SuffixType::Si), "1.1 MB");
        assert_eq!(to_magnitude_and_suffix(1_900_000, SuffixType::Si), "1.9 MB");
        assert_eq!(to_magnitude_and_suffix(1_900_001, SuffixType::Si), "1.9 MB");
        assert_eq!(to_magnitude_and_suffix(9_900_000, SuffixType::Si), "9.9 MB");
        assert_eq!(to_magnitude_and_suffix(9_900_001, SuffixType::Si), "9.9 MB");
        assert_eq!(
            to_magnitude_and_suffix(999_000_000, SuffixType::Si),
            "999 MB"
        );

        assert_eq!(
            to_magnitude_and_suffix(999_000_001, SuffixType::Si),
            "999 MB"
        );
        assert_eq!(
            to_magnitude_and_suffix(999_500_000, SuffixType::Si),
            "1.0 GB"
        );
        assert_eq!(
            to_magnitude_and_suffix(1_000_000_000, SuffixType::Si),
            "1.0 GB"
        );
        assert_eq!(
            to_magnitude_and_suffix(1_000_000_001, SuffixType::Si),
            "1.0 GB"
        );
    }

    #[test]
    fn test_rate_suffix_matches_gnu_ratio_paths() {
        assert_eq!(to_rate_and_suffix(1, 1_000_000_000), "1 B");
        assert_eq!(to_rate_and_suffix(1, 2_000_000_000), "0 B");
        assert_eq!(to_rate_and_suffix(1, 1_200_000_000), "0.0 kB");
        assert_eq!(to_rate_and_suffix(10_500, 1_000_000_000), "10 kB");
    }
}
