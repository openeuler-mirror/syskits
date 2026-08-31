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

//! `CtValue` — syskits 数据管线的核心值类型。
//!
//! 设计原则：
//! - 变体与 Nushell 的 `Value` 类型完全独立，自主设计
//! - M1a 仅实现基础标量与容器类型，流类型由 `pipeline_data.rs` 持有
//! - `CtValueError` 统一表示类型转换失败

use thiserror::Error;

/// 值类型标签（不持有数据）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CtType {
    /// 任意类型（用于签名兜底）
    Any,
    /// 无值（空管线）
    Nothing,
    /// 布尔
    Bool,
    /// 64 位有符号整数
    Int,
    /// 64 位浮点数
    Float,
    /// UTF-8 字符串
    String,
    /// 二进制字节序列
    Binary,
    /// Unix 时间戳（纳秒 i128）
    DateTime,
    /// 纳秒时长 i64
    Duration,
    /// 字节大小（u64，最大 16 EB）
    Size,
    /// 结构体（有序键值对）
    Record,
    /// 同质列表（惰性或具象）
    List,
    /// 惰性列表流（来自管线）
    ListStream,
    /// 字节流（来自外部命令或文件）
    ByteStream,
    /// 运行时错误值（可在管线中传递）
    Error,
}

/// 管线中流动的核心值类型
///
/// 不可 `Clone` 的变体（`ListStream`/`ByteStream`）托管于 `CtPipelineData`，
/// 此处仅保留可廉价克隆的标量及容器变体。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum CtValue {
    // NOTE:
    // CtValue 使用 untagged 反序列化时，数值/数组等变体存在匹配歧义。
    // 对外部 JSON 输入请优先走显式转换流程（如 ctengine::external::json_to_ctvalue）。
    /// 空值（表示命令无输出）
    Nothing,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// 具象列表（所有元素已求值）
    List(Vec<CtValue>),
    Binary(Vec<u8>),
    /// Unix 纳秒时间戳
    DateTime(i128),
    /// 纳秒时长
    Duration(i64),
    /// 字节大小
    Size(u64),
    /// 有序键值对记录
    Record(Vec<(String, CtValue)>),
    /// 运行时错误（包装为可传递值）
    #[serde(skip)]
    Error(Box<CtValueError>),
}

impl CtValue {
    /// 将值转换为适合管线文本流展示的简单字符串表示
    pub fn to_text(&self) -> String {
        match self {
            CtValue::Nothing => String::new(),
            CtValue::Bool(b) => b.to_string(),
            CtValue::Int(i) => i.to_string(),
            CtValue::Float(f) => f.to_string(),
            CtValue::String(s) => s.clone(),
            CtValue::Binary(_) => "<binary>".to_string(),
            CtValue::DateTime(nanos) => format_datetime_nanos(*nanos),
            CtValue::Duration(nanos) => format_duration_nanos(*nanos),
            CtValue::Size(bytes) => format_size_bytes(*bytes),
            CtValue::Record(_) => "<record>".to_string(),
            CtValue::List(_) => "<list>".to_string(),
            CtValue::Error(e) => format!("Error: {e}"),
        }
    }

    /// 返回此值的类型标签
    pub fn value_type(&self) -> CtType {
        match self {
            CtValue::Nothing => CtType::Nothing,
            CtValue::Bool(_) => CtType::Bool,
            CtValue::Int(_) => CtType::Int,
            CtValue::Float(_) => CtType::Float,
            CtValue::String(_) => CtType::String,
            CtValue::Binary(_) => CtType::Binary,
            CtValue::DateTime(_) => CtType::DateTime,
            CtValue::Duration(_) => CtType::Duration,
            CtValue::Size(_) => CtType::Size,
            CtValue::Record(_) => CtType::Record,
            CtValue::List(_) => CtType::List,
            CtValue::Error(_) => CtType::Error,
        }
    }

    /// 尝试转换为 `bool`，失败时返回 `CtValueError`
    pub fn as_bool(&self) -> Result<bool, CtValueError> {
        match self {
            CtValue::Bool(b) => Ok(*b),
            other => Err(CtValueError::type_mismatch(
                CtType::Bool,
                other.value_type(),
            )),
        }
    }

    /// 尝试转换为 `i64`
    pub fn as_int(&self) -> Result<i64, CtValueError> {
        match self {
            CtValue::Int(i) => Ok(*i),
            other => Err(CtValueError::type_mismatch(CtType::Int, other.value_type())),
        }
    }

    /// 尝试转换为 `f64`
    pub fn as_float(&self) -> Result<f64, CtValueError> {
        match self {
            CtValue::Float(f) => Ok(*f),
            CtValue::Int(i) => Ok(*i as f64),
            other => Err(CtValueError::type_mismatch(
                CtType::Float,
                other.value_type(),
            )),
        }
    }

    /// 尝试转换为 `&str`
    pub fn as_str(&self) -> Result<&str, CtValueError> {
        match self {
            CtValue::String(s) => Ok(s.as_str()),
            other => Err(CtValueError::type_mismatch(
                CtType::String,
                other.value_type(),
            )),
        }
    }

    /// 尝试转换为 `u64`（字节大小）
    pub fn as_size(&self) -> Result<u64, CtValueError> {
        match self {
            CtValue::Size(s) => Ok(*s),
            CtValue::Int(i) => {
                if *i >= 0 {
                    Ok(*i as u64)
                } else {
                    Err(CtValueError::custom(
                        "negative integer cannot be converted to Size",
                    ))
                }
            }
            other => Err(CtValueError::type_mismatch(
                CtType::Size,
                other.value_type(),
            )),
        }
    }

    /// 尝试转换为 `i64`（纳秒时长）
    pub fn as_duration(&self) -> Result<i64, CtValueError> {
        match self {
            CtValue::Duration(d) => Ok(*d),
            other => Err(CtValueError::type_mismatch(
                CtType::Duration,
                other.value_type(),
            )),
        }
    }

    /// 尝试转换为 `i128`（Unix 纳秒时间戳）
    pub fn as_datetime(&self) -> Result<i128, CtValueError> {
        match self {
            CtValue::DateTime(dt) => Ok(*dt),
            other => Err(CtValueError::type_mismatch(
                CtType::DateTime,
                other.value_type(),
            )),
        }
    }
}

/// 值类型转换或访问错误
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CtValueError {
    #[error("type mismatch: expected {expected:?}, got {got:?}")]
    TypeMismatch { expected: CtType, got: CtType },
    #[error("{0}")]
    Custom(String),
}

impl CtValueError {
    pub fn type_mismatch(expected: CtType, got: CtType) -> Self {
        Self::TypeMismatch { expected, got }
    }

    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }
}

// ── 格式化辅助函数 ──────────────────────────────────────

/// 将 Unix 纳秒时间戳格式化为 ISO-8601 字符串
fn format_datetime_nanos(nanos: i128) -> String {
    const NANOS_PER_SEC: i128 = 1_000_000_000;
    const SECS_PER_DAY: i64 = 86_400;

    let raw_secs = nanos.div_euclid(NANOS_PER_SEC);
    let secs = match i64::try_from(raw_secs) {
        Ok(secs) => secs,
        Err(_) if raw_secs < 0 => i64::MIN,
        Err(_) => i64::MAX,
    };
    let subsec_nanos = nanos.rem_euclid(NANOS_PER_SEC) as u32;
    // 手动计算 UTC 日期时间（避免引入外部时间库）
    // 使用 Unix epoch (1970-01-01) 为基准
    let days = secs.div_euclid(SECS_PER_DAY);
    let day_secs = secs.rem_euclid(SECS_PER_DAY);
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    let (y, mo, d) = days_to_ymd(days);
    if subsec_nanos == 0 {
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
    } else {
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{subsec_nanos:09}Z")
    }
}

/// 将天数偏移（从 1970-01-01 起）转换为 (year, month, day)
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    // Civil from days algorithm (Howard Hinnant)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// 将纳秒时长格式化为人类可读字符串
fn format_duration_nanos(nanos: i64) -> String {
    let negative = nanos < 0;
    let abs = nanos.unsigned_abs();
    let prefix = if negative { "-" } else { "" };

    if abs < 1_000 {
        return format!("{prefix}{abs}ns");
    }
    if abs < 1_000_000 {
        return format!("{prefix}{}us", abs / 1_000);
    }
    if abs < 1_000_000_000 {
        return format!("{prefix}{}ms", abs / 1_000_000);
    }

    let total_secs = abs / 1_000_000_000;
    if total_secs < 60 {
        return format!("{prefix}{total_secs}sec");
    }
    if total_secs < 3600 {
        let m = total_secs / 60;
        let s = total_secs % 60;
        return if s == 0 {
            format!("{prefix}{m}min")
        } else {
            format!("{prefix}{m}min {s}sec")
        };
    }
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    if m == 0 {
        format!("{prefix}{h}hr")
    } else {
        format!("{prefix}{h}hr {m}min")
    }
}

/// 将字节大小格式化为人类可读字符串
fn format_size_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    const TB: u64 = 1024 * 1024 * 1024 * 1024;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes < TB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ctvalue_conversions() {
        let v = CtValue::Int(42);
        assert_eq!(v.value_type(), CtType::Int);
        assert_eq!(v.as_int().unwrap(), 42);
        assert!(v.as_bool().is_err());
    }

    #[test]
    fn test_ctvalue_float_coerce_from_int() {
        let v = CtValue::Int(3);
        assert!((v.as_float().unwrap() - 3.0f64).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ctvalue_nothing() {
        let v = CtValue::Nothing;
        assert_eq!(v.value_type(), CtType::Nothing);
    }

    #[test]
    fn test_ctvalue_string() {
        let v = CtValue::String("hello".to_string());
        assert_eq!(v.as_str().unwrap(), "hello");
    }

    #[test]
    fn test_ctvalue_error_type_mismatch() {
        let err = CtValueError::type_mismatch(CtType::Bool, CtType::Int);
        let msg = err.to_string();
        assert!(msg.contains("Bool"));
        assert!(msg.contains("Int"));
    }

    #[test]
    fn test_ctvalue_list_clone() {
        let v = CtValue::List(vec![CtValue::Int(1), CtValue::Int(2)]);
        let v2 = v.clone();
        if let CtValue::List(items) = v2 {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn test_ctvalue_json_numeric_array_deserializes_as_list() {
        let v: CtValue = serde_json::from_str("[1,2,3]").unwrap();
        let CtValue::List(items) = v else {
            panic!("expected List");
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(items[0], CtValue::Int(1)));
        assert!(matches!(items[1], CtValue::Int(2)));
        assert!(matches!(items[2], CtValue::Int(3)));
    }

    #[test]
    fn test_ctvalue_size() {
        let v = CtValue::Size(1024 * 1024 * 10); // 10 MB
        assert_eq!(v.value_type(), CtType::Size);
        assert_eq!(v.as_size().unwrap(), 10_485_760);
        assert_eq!(v.to_text(), "10.0 MB");
    }

    #[test]
    fn test_ctvalue_size_from_int() {
        let v = CtValue::Int(1024);
        assert_eq!(v.as_size().unwrap(), 1024);
    }

    #[test]
    fn test_ctvalue_size_negative_int_fails() {
        let v = CtValue::Int(-1);
        assert!(v.as_size().is_err());
    }

    #[test]
    fn test_ctvalue_duration() {
        let v = CtValue::Duration(120_000_000_000); // 2 min
        assert_eq!(v.as_duration().unwrap(), 120_000_000_000);
        assert_eq!(v.to_text(), "2min");
    }

    #[test]
    fn test_ctvalue_datetime() {
        // 2025-01-01T00:00:00Z = 1735689600 seconds from epoch
        let nanos: i128 = 1_735_689_600 * 1_000_000_000;
        let v = CtValue::DateTime(nanos);
        assert_eq!(v.as_datetime().unwrap(), nanos);
        assert_eq!(v.to_text(), "2025-01-01T00:00:00Z");
    }

    #[test]
    fn test_ctvalue_datetime_negative_subsecond() {
        let v = CtValue::DateTime(-1);
        assert_eq!(v.to_text(), "1969-12-31T23:59:59.999999999Z");
    }

    #[test]
    fn test_ctvalue_datetime_i128_max_saturates_seconds() {
        let v = CtValue::DateTime(i128::MAX);
        assert_eq!(v.to_text(), "292277026596-12-04T15:30:07.884105727Z");
    }

    #[test]
    fn test_ctvalue_datetime_i128_min_saturates_seconds() {
        let v = CtValue::DateTime(i128::MIN);
        assert_eq!(v.to_text(), "-292277022657-01-27T08:29:52.115894272Z");
    }

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size_bytes(0), "0 B");
        assert_eq!(format_size_bytes(512), "512 B");
        assert_eq!(format_size_bytes(1024), "1.0 KB");
        assert_eq!(format_size_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_size_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn test_format_duration_nanos() {
        assert_eq!(format_duration_nanos(500), "500ns");
        assert_eq!(format_duration_nanos(5_000), "5us");
        assert_eq!(format_duration_nanos(5_000_000), "5ms");
        assert_eq!(format_duration_nanos(5_000_000_000), "5sec");
        assert_eq!(format_duration_nanos(90_000_000_000), "1min 30sec");
        assert_eq!(format_duration_nanos(7200_000_000_000), "2hr");
        assert_eq!(format_duration_nanos(-5_000_000_000), "-5sec");
    }
}
