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

//! `DataSignature` — DataCommand 的参数签名描述。
//!
//! 设计原则：
//! - 签名在 `DataCommand::signature()` 中静态返回，不持有运行时状态
//! - `CtPositionalArg`/`CtFlag` 不使用 clap，由引擎自行解析
//! - M1a 不实现 parser，签名类型结构已定型供 M1b 直接使用

use ctpipeline::CtType;

/// 位置参数描述
#[derive(Debug, Clone)]
pub struct CtPositionalArg {
    /// 参数名（用于帮助显示）
    pub name: &'static str,
    /// 参数说明
    pub desc: &'static str,
    /// 期望的值类型
    pub value_type: CtType,
    /// 是否可选（false = 必填）
    pub optional: bool,
}

impl CtPositionalArg {
    /// 创建必填位置参数
    pub fn required(name: &'static str, desc: &'static str, value_type: CtType) -> Self {
        Self {
            name,
            desc,
            value_type,
            optional: false,
        }
    }

    /// 创建可选位置参数
    pub fn optional(name: &'static str, desc: &'static str, value_type: CtType) -> Self {
        Self {
            name,
            desc,
            value_type,
            optional: true,
        }
    }
}

/// 标志参数描述（`--flag` 或 `-f`）
#[derive(Debug, Clone)]
pub struct CtFlag {
    /// 长名称（不含 `--`）
    pub long: &'static str,
    /// 短名称（不含 `-`，`None` 表示无短名称）
    pub short: Option<char>,
    /// 说明
    pub desc: &'static str,
    /// 如果非 None，则标志接受一个该类型的值（`--flag <value>`）
    pub value_type: Option<CtType>,
}

impl CtFlag {
    /// 创建无参数的开关标志（`--verbose`）
    pub fn switch(long: &'static str, short: Option<char>, desc: &'static str) -> Self {
        Self {
            long,
            short,
            desc,
            value_type: None,
        }
    }

    /// 创建带值的标志（`--output <path>`）
    pub fn with_value(
        long: &'static str,
        short: Option<char>,
        desc: &'static str,
        value_type: CtType,
    ) -> Self {
        Self {
            long,
            short,
            desc,
            value_type: Some(value_type),
        }
    }
}

/// DataCommand 的完整参数签名
#[derive(Debug, Clone)]
pub struct DataSignature {
    /// 命令名称（必须唯一）
    pub name: &'static str,
    /// 命令简短说明（用于帮助文本）
    pub desc: &'static str,
    /// 必填位置参数
    pub required_positional: Vec<CtPositionalArg>,
    /// 可选位置参数
    pub optional_positional: Vec<CtPositionalArg>,
    /// 可变参数
    pub rest_positional: Option<CtPositionalArg>,
    /// 命名参数
    pub named: Vec<CtFlag>,
    /// 输入输出类型对
    pub input_output_types: Vec<(CtType, CtType)>,
    /// 是否允许未知参数
    pub allows_unknown_args: bool,

    // Backward-compatible fields used by existing engine/tests.
    /// 位置参数列表（顺序即解析顺序）
    pub positionals: Vec<CtPositionalArg>,
    /// 标志参数列表
    pub flags: Vec<CtFlag>,
    /// 命令期望接收的输入类型（None 表示不消费管线输入）
    pub input_type: Option<CtType>,
    /// 命令产生的输出类型（None 表示 Nothing）
    pub output_type: Option<CtType>,
}

impl DataSignature {
    /// 创建最简签名（无参数，无管线 IO）
    pub fn new(name: &'static str, desc: &'static str) -> Self {
        Self {
            name,
            desc,
            required_positional: Vec::new(),
            optional_positional: Vec::new(),
            rest_positional: None,
            named: Vec::new(),
            input_output_types: Vec::new(),
            allows_unknown_args: false,
            positionals: Vec::new(),
            flags: Vec::new(),
            input_type: None,
            output_type: None,
        }
    }

    /// 追加一个位置参数
    pub fn positional(mut self, arg: CtPositionalArg) -> Self {
        if arg.optional {
            self.optional_positional.push(arg.clone());
        } else {
            self.required_positional.push(arg.clone());
        }
        self.positionals.push(arg);
        self
    }

    /// 追加一个标志参数
    pub fn flag(mut self, flag: CtFlag) -> Self {
        self.named.push(flag.clone());
        self.flags.push(flag);
        self
    }

    /// 设置管线输入类型
    pub fn input(mut self, t: CtType) -> Self {
        self.input_type = Some(t);
        self.sync_input_output_types();
        self
    }

    /// 设置管线输出类型
    pub fn output(mut self, t: CtType) -> Self {
        self.output_type = Some(t);
        self.sync_input_output_types();
        self
    }

    /// 设置可变参数描述
    pub fn rest(mut self, arg: CtPositionalArg) -> Self {
        self.rest_positional = Some(arg);
        self
    }

    /// 设置是否允许未知参数
    pub fn allow_unknown_args(mut self, allow: bool) -> Self {
        self.allows_unknown_args = allow;
        self
    }

    fn sync_input_output_types(&mut self) {
        self.input_output_types.clear();
        if let (Some(input), Some(output)) = (self.input_type, self.output_type) {
            self.input_output_types.push((input, output));
        }
    }

    /// 获取输入输出类型对（若仅声明单边则返回空）
    pub fn io_types(&self) -> &[(CtType, CtType)] {
        &self.input_output_types
    }

    /// 按 LLD 语义返回命名参数
    pub fn named_flags(&self) -> &[CtFlag] {
        &self.named
    }

    /// 按 LLD 语义返回必填位置参数
    pub fn required_positionals(&self) -> &[CtPositionalArg] {
        &self.required_positional
    }

    /// 按 LLD 语义返回可选位置参数
    pub fn optional_positionals(&self) -> &[CtPositionalArg] {
        &self.optional_positional
    }

    /// 按 LLD 语义返回 rest 参数
    pub fn rest_positional_arg(&self) -> Option<&CtPositionalArg> {
        self.rest_positional.as_ref()
    }

    /// 是否允许未知参数
    pub fn allows_unknown(&self) -> bool {
        self.allows_unknown_args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_signature_build() {
        let sig = DataSignature::new("select", "Select columns from a record or table")
            .positional(CtPositionalArg::required(
                "columns",
                "columns to select",
                CtType::String,
            ))
            .flag(CtFlag::switch("verbose", Some('v'), "verbose output"))
            .input(CtType::List)
            .output(CtType::List);

        assert_eq!(sig.name, "select");
        assert_eq!(sig.positionals.len(), 1);
        assert_eq!(sig.flags.len(), 1);
        assert!(!sig.positionals[0].optional);
        assert_eq!(sig.flags[0].short, Some('v'));
        assert_eq!(sig.input_type, Some(CtType::List));
        assert_eq!(sig.output_type, Some(CtType::List));
        assert_eq!(sig.required_positionals().len(), 1);
        assert_eq!(sig.named_flags().len(), 1);
        assert_eq!(sig.io_types(), &[(CtType::List, CtType::List)]);
    }

    #[test]
    fn test_positional_optional() {
        let arg = CtPositionalArg::optional("path", "output path", CtType::String);
        assert!(arg.optional);
    }

    #[test]
    fn test_flag_with_value() {
        let flag = CtFlag::with_value("output", Some('o'), "output file", CtType::String);
        assert_eq!(flag.value_type, Some(CtType::String));
        assert_eq!(flag.long, "output");
    }
}
