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

//! `DataCall` — 命令调用时的已绑定参数集合。
//!
//! `DataCall` 由引擎（M1b 实现）在解析完命令行后构建，交由 `DataCommand::run()` 使用。
//! M1a 阶段仅定义结构体及参数提取 API，不实现解析逻辑。

use ctpipeline::{CtSpan, CtValue, CtValueError};
use std::collections::HashMap;
use thiserror::Error;

/// 参数提取错误
#[derive(Debug, Clone, Error)]
pub enum CallError {
    #[error("missing required argument at position {pos}")]
    MissingRequired { pos: usize },
    #[error("argument at position {pos}: {source}")]
    TypeConversion { pos: usize, source: CtValueError },
    #[error("flag '--{name}': {source}")]
    FlagConversion { name: String, source: CtValueError },
}

/// 已绑定的单条参数
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoundArg {
    pub value: CtValue,
    pub span: Option<CtSpan>,
}

impl BoundArg {
    pub fn new(value: CtValue, span: Option<CtSpan>) -> Self {
        Self { value, span }
    }
}

/// 命令调用时的已绑定参数集合
///
/// 由引擎在参数绑定阶段构建，`DataCommand::run()` 通过此结构获取参数值。
/// 字段与 LLD §5.3 对齐：head / command_name / positionals / flags / rest。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DataCall {
    /// 命令调用头部 span（用于参数错误定位）
    pub head: Option<CtSpan>,
    /// 被调用的命令名
    pub command_name: String,
    /// 位置参数列表（按声明顺序，不含 rest）
    pub positionals: Vec<BoundArg>,
    /// 标志参数映射（key 为长名称）
    pub flags: HashMap<String, Option<BoundArg>>,
    /// 剩余位置参数（rest positional）
    pub rest: Vec<BoundArg>,
}

impl DataCall {
    /// 创建空调用（用于测试和存根）
    pub fn empty() -> Self {
        Self {
            head: None,
            command_name: String::new(),
            positionals: Vec::new(),
            flags: HashMap::new(),
            rest: Vec::new(),
        }
    }

    /// 创建带命令名的空调用
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            head: None,
            command_name: name.into(),
            positionals: Vec::new(),
            flags: HashMap::new(),
            rest: Vec::new(),
        }
    }

    /// 检查开关标志是否被设置（`--verbose`）
    pub fn has_flag(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    /// 获取带值标志的值，若标志未设置则返回 `None`
    pub fn get_flag<T>(&self, name: &str) -> Result<Option<T>, CallError>
    where
        T: TryFromCtValue,
    {
        match self.flags.get(name) {
            None => Ok(None),
            Some(None) => Ok(None),
            Some(Some(arg)) => {
                T::try_from_ct_value(&arg.value)
                    .map(Some)
                    .map_err(|e| CallError::FlagConversion {
                        name: name.to_string(),
                        source: e,
                    })
            }
        }
    }

    /// 获取必填位置参数（缺失或类型不符时返回 `Err`）
    pub fn req<T>(&self, pos: usize) -> Result<T, CallError>
    where
        T: TryFromCtValue,
    {
        let arg = self
            .positionals
            .get(pos)
            .ok_or(CallError::MissingRequired { pos })?;
        T::try_from_ct_value(&arg.value).map_err(|e| CallError::TypeConversion { pos, source: e })
    }

    /// 获取可选位置参数（未提供时返回 `Ok(None)`）
    pub fn opt<T>(&self, pos: usize) -> Result<Option<T>, CallError>
    where
        T: TryFromCtValue,
    {
        match self.positionals.get(pos) {
            None => Ok(None),
            Some(arg) => T::try_from_ct_value(&arg.value)
                .map(Some)
                .map_err(|e| CallError::TypeConversion { pos, source: e }),
        }
    }

    /// 获取剩余位置参数（从 `start` 位置起的所有参数）
    pub fn rest<T>(&self, start: usize) -> Result<Vec<T>, CallError>
    where
        T: TryFromCtValue,
    {
        self.positionals[start.min(self.positionals.len())..]
            .iter()
            .enumerate()
            .map(|(i, arg)| {
                T::try_from_ct_value(&arg.value).map_err(|e| CallError::TypeConversion {
                    pos: start + i,
                    source: e,
                })
            })
            .collect()
    }
}

/// 从 `CtValue` 转换为目标类型的 trait（限定于基本类型）
pub trait TryFromCtValue: Sized {
    fn try_from_ct_value(value: &CtValue) -> Result<Self, CtValueError>;
}

impl TryFromCtValue for i64 {
    fn try_from_ct_value(value: &CtValue) -> Result<Self, CtValueError> {
        value.as_int()
    }
}

impl TryFromCtValue for f64 {
    fn try_from_ct_value(value: &CtValue) -> Result<Self, CtValueError> {
        value.as_float()
    }
}

impl TryFromCtValue for bool {
    fn try_from_ct_value(value: &CtValue) -> Result<Self, CtValueError> {
        value.as_bool()
    }
}

impl TryFromCtValue for String {
    fn try_from_ct_value(value: &CtValue) -> Result<Self, CtValueError> {
        value.as_str().map(|s| s.to_owned())
    }
}

impl TryFromCtValue for CtValue {
    fn try_from_ct_value(value: &CtValue) -> Result<Self, CtValueError> {
        Ok(value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctpipeline::CtValue;

    fn make_call_with_positionals(values: Vec<CtValue>) -> DataCall {
        DataCall {
            head: None,
            command_name: "test".to_string(),
            positionals: values.into_iter().map(|v| BoundArg::new(v, None)).collect(),
            flags: HashMap::new(),
            rest: Vec::new(),
        }
    }

    #[test]
    fn test_data_call_has_flag() {
        let mut call = DataCall::empty();
        call.flags.insert("verbose".to_string(), None);
        assert!(call.has_flag("verbose"));
        assert!(!call.has_flag("quiet"));
    }

    #[test]
    fn test_data_call_req_ok() {
        let call = make_call_with_positionals(vec![CtValue::Int(42)]);
        let v: i64 = call.req(0).unwrap();
        assert_eq!(v, 42);
    }

    #[test]
    fn test_data_call_req_missing() {
        let call = DataCall::empty();
        let result = call.req::<i64>(0);
        assert!(matches!(result, Err(CallError::MissingRequired { pos: 0 })));
    }

    #[test]
    fn test_data_call_opt_none() {
        let call = DataCall::empty();
        let v: Option<String> = call.opt(0).unwrap();
        assert_eq!(v, None);
    }

    #[test]
    fn test_data_call_opt_some() {
        let call = make_call_with_positionals(vec![CtValue::String("world".to_string())]);
        let v: Option<String> = call.opt(0).unwrap();
        assert_eq!(v.as_deref(), Some("world"));
    }

    #[test]
    fn test_data_call_rest() {
        let call = make_call_with_positionals(vec![
            CtValue::String("a".to_string()),
            CtValue::String("b".to_string()),
            CtValue::String("c".to_string()),
        ]);
        let rest: Vec<String> = call.rest(1).unwrap();
        assert_eq!(rest, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn test_data_call_get_flag_with_value() {
        let mut call = DataCall::empty();
        call.flags.insert(
            "output".to_string(),
            Some(BoundArg::new(CtValue::String("out.txt".to_string()), None)),
        );
        let v: Option<String> = call.get_flag("output").unwrap();
        assert_eq!(v.as_deref(), Some("out.txt"));
    }
}
