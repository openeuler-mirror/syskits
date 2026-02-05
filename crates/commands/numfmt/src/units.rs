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
use std::fmt;

pub const NUMFMT_SI_BASES: [f64; 10] = [1., 1e3, 1e6, 1e9, 1e12, 1e15, 1e18, 1e21, 1e24, 1e27];

pub const NUMFMT_IEC_BASES: [f64; 10] = [
    1.,
    1_024.,
    1_048_576.,
    1_073_741_824.,
    1_099_511_627_776.,
    1_125_899_906_842_624.,
    1_152_921_504_606_846_976.,
    1_180_591_620_717_411_303_424.,
    1_208_925_819_614_629_174_706_176.,
    1_237_940_039_285_380_274_899_124_224.,
];

pub type NumfmtWithI = bool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumfmtUnit {
    Auto,
    Si,
    Iec(NumfmtWithI),
    None,
}

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumfmtRawSuffix {
    K,
    M,
    G,
    T,
    P,
    E,
    Z,
    Y,
}

pub type NumfmtSuffix = (NumfmtRawSuffix, NumfmtWithI);

pub struct DisplayableSuffix(pub NumfmtSuffix);

impl fmt::Display for DisplayableSuffix {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Self((ref raw_suffix, ref with_i)) = *self;
        match raw_suffix {
            NumfmtRawSuffix::K => write!(f, "K"),
            NumfmtRawSuffix::M => write!(f, "M"),
            NumfmtRawSuffix::G => write!(f, "G"),
            NumfmtRawSuffix::T => write!(f, "T"),
            NumfmtRawSuffix::P => write!(f, "P"),
            NumfmtRawSuffix::E => write!(f, "E"),
            NumfmtRawSuffix::Z => write!(f, "Z"),
            NumfmtRawSuffix::Y => write!(f, "Y"),
        }
        .and_then(|()| if *with_i { write!(f, "i") } else { Ok(()) })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_display_suffix_no_i() {
        let suffixes = [
            (NumfmtRawSuffix::K, false, "K"),
            (NumfmtRawSuffix::M, false, "M"),
            (NumfmtRawSuffix::G, false, "G"),
            (NumfmtRawSuffix::T, false, "T"),
            (NumfmtRawSuffix::P, false, "P"),
            (NumfmtRawSuffix::E, false, "E"),
            (NumfmtRawSuffix::Z, false, "Z"),
            (NumfmtRawSuffix::Y, false, "Y"),
        ];

        for (raw_suffix, with_i, expected) in suffixes {
            let suffix = DisplayableSuffix((raw_suffix, with_i));
            assert_eq!(format!("{suffix}"), expected);
        }
    }

    #[test]
    fn test_display_suffix_with_i() {
        let suffixes = [
            (NumfmtRawSuffix::K, true, "Ki"),
            (NumfmtRawSuffix::M, true, "Mi"),
            (NumfmtRawSuffix::G, true, "Gi"),
            (NumfmtRawSuffix::T, true, "Ti"),
            (NumfmtRawSuffix::P, true, "Pi"),
            (NumfmtRawSuffix::E, true, "Ei"),
            (NumfmtRawSuffix::Z, true, "Zi"),
            (NumfmtRawSuffix::Y, true, "Yi"),
        ];

        for (raw_suffix, with_i, expected) in suffixes {
            let suffix = DisplayableSuffix((raw_suffix, with_i));
            assert_eq!(format!("{suffix}"), expected);
        }
    }
}
