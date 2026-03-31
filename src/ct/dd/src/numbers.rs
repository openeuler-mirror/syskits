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
    fn base_and_suffix(&self, n: u128) -> (u128, &'static str) {
        let (bases, suffixes) = match self {
            Self::Iec => (IEC_BASES, IEC_SUFFIXES),
            Self::Si => (SI_BASES, SI_SUFFIXES),
        };
        let mut i = 0;
        while bases[i + 1] - bases[i] < n && i < suffixes.len() {
            i += 1;
        }
        (bases[i], suffixes[i])
    }
}

/// Convert a number into a magnitude and a multi-byte unit suffix.
///
/// The returned string has a maximum length of 5 chars, for example: "1.1kB", "999kB", "1MB".
pub(crate) fn to_magnitude_and_suffix(n: u128, suffix_type: SuffixType) -> String {
    let (base, suffix) = suffix_type.base_and_suffix(n);
    // TODO To match dd on my machine, we would need to round like
    // this:
    //
    // 1049 => 1.0 kB
    // 1050 => 1.0 kB  # why is this different?
    // 1051 => 1.1 kB
    // ...
    // 1149 => 1.1 kB
    // 1150 => 1.2 kB
    // ...
    // 1250 => 1.2 kB
    // 1251 => 1.3 kB
    // ..
    // 10500 => 10 kB
    // 10501 => 11 kB
    //
    let quotient = (n as f64) / (base as f64);
    if quotient < 10.0 {
        format!("{quotient:.1} {suffix}")
    } else {
        format!("{} {}", quotient.round(), suffix)
    }
}

