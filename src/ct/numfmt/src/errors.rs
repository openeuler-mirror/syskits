/*
 *    Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *     syskits is licensed under Mulan PSL v2.
 *    You can use this software according to the terms and conditions of the Mulan PSL V2
 *    You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 *    THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 *    KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 *    NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 *    See the Mulan PSL v2 for more details.
 *
 */

use ctcore::ct_error::CTError;
use std::{
    error::Error,
    fmt::{Debug, Display},
};

#[derive(Debug)]
pub enum NumfmtError {
    NumfmtIoError(String),
    NumfmtIllegalArgument(String),
    NumfmtFormattingError(String),
}

impl CTError for NumfmtError {
    fn code(&self) -> i32 {
        match self {
            NumfmtError::NumfmtIoError(_) => 1,
            NumfmtError::NumfmtIllegalArgument(_) => 1,
            NumfmtError::NumfmtFormattingError(_) => 2,
        }
    }
}

impl Error for NumfmtError {}

impl Display for NumfmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumfmtError::NumfmtIoError(err)
            | NumfmtError::NumfmtIllegalArgument(err)
            | NumfmtError::NumfmtFormattingError(err) => {
                write!(f, "{err}")
            }
        }
    }
}

