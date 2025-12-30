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

// 该模块包含了处理不同操作系统之间OsStr和OsString无损处理差异的类和函数。
// 与具有相似目标的现有库不同，这个模块不使用任何unsafe特性或函数。
// 由于Windows系统上OsStr/OsString的设计不够理想，我们需要在Windows操作系统上对它们进行宽字符编码/解码。
// 这阻止了在Windows上直接从OsStr借用。然而，如果使用得当，这种转换只需要在开始和结束时各做一次。

use std::ffi::OsString;
#[cfg(not(target_os = "windows"))]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(target_os = "windows")]
use std::os::windows::prelude::*;
use std::{borrow::Cow, ffi::OsStr};

#[cfg(target_os = "windows")]
use u16 as NativeIntCharU;
#[cfg(not(target_os = "windows"))]
use u8 as NativeIntCharU;

pub type NativeCharInt = NativeIntCharU;
pub type NativeIntStr = [NativeCharInt];
pub type NativeIntString = Vec<NativeCharInt>;

// 定义一个空结构体 NCvt，仅用于实现各种类型的转换 trait。
pub struct NCvt;

// 定义一个 trait EnvConvert，用于表示从一个类型到另一个类型的转换。
pub trait EnvConvert<From, To> {
    fn convert(f: From) -> To;
}

// 以下是各种从 &str, String, &OsStr, &OsString 到 Cow<'a, NativeIntStr> 的转换实现。
// 这些实现根据目标操作系统的不同（Windows 或非 Windows），
// 分别使用 encode_utf16 或 encode_wide 来编码字符串或 OsString 为平台依赖的字节序列。
impl<'a> EnvConvert<&'a str, Cow<'a, NativeIntStr>> for NCvt {
    fn convert(f: &'a str) -> Cow<'a, NativeIntStr> {
        #[cfg(target_os = "windows")]
        {
            Cow::Owned(f.encode_utf16().collect())
        }

        #[cfg(not(target_os = "windows"))]
        {
            Cow::Borrowed(f.as_bytes())
        }
    }
}

impl<'a> EnvConvert<&'a String, Cow<'a, NativeIntStr>> for NCvt {
    fn convert(f: &'a String) -> Cow<'a, NativeIntStr> {
        #[cfg(target_os = "windows")]
        {
            Cow::Owned(f.encode_utf16().collect())
        }

        #[cfg(not(target_os = "windows"))]
        {
            Cow::Borrowed(f.as_bytes())
        }
    }
}

impl<'a> EnvConvert<String, Cow<'a, NativeIntStr>> for NCvt {
    fn convert(f: String) -> Cow<'a, NativeIntStr> {
        #[cfg(target_os = "windows")]
        {
            Cow::Owned(f.encode_utf16().collect())
        }

        #[cfg(not(target_os = "windows"))]
        {
            Cow::Owned(f.into_bytes())
        }
    }
}

// ================ OsStr/OsString =================
// 定义了 `EnvConvert` 从 `&OsStr` 到 `Cow<NativeIntStr>` 的实现。
// 这个实现将给定的 `OsStr` 引用转换为 native 整数表示形式的 `Cow` 对象。
impl<'a> EnvConvert<&'a OsStr, Cow<'a, NativeIntStr>> for NCvt {
    fn convert(f: &'a OsStr) -> Cow<'a, NativeIntStr> {
        to_native_int_representation(f)
    }
}

// 定义了 `EnvConvert` 从 `&OsString` 到 `Cow<NativeIntStr>` 的实现。
// 这个实现将给定的 `OsString` 引用转换为 native 整数表示形式的 `Cow` 对象。
impl<'a> EnvConvert<&'a OsString, Cow<'a, NativeIntStr>> for NCvt {
    fn convert(f: &'a OsString) -> Cow<'a, NativeIntStr> {
        to_native_int_representation(f)
    }
}

// 定义了 `EnvConvert` 从 `OsString` 到 `Cow<NativeIntStr>` 的实现。
// 这个实现将给定的 `OsString` 值转换为 native 整数表示形式的 `Cow` 对象。
// 根据目标操作系统，转换的方式有所不同：
// - 在 Windows 上，它会将字符串编码为宽字符 Vec。
// - 在非 Windows 系统上，它会直接将字符串转换为字节 Vec。
impl<'a> EnvConvert<OsString, Cow<'a, NativeIntStr>> for NCvt {
    fn convert(f: OsString) -> Cow<'a, NativeIntStr> {
        #[cfg(target_os = "windows")]
        {
            // 在 Windows 平台，将 OsString 转换为宽字符 Vec。
            Cow::Owned(f.encode_wide().collect())
        }

        #[cfg(not(target_os = "windows"))]
        {
            // 在非 Windows 平台，直接将 OsString 转换为字节 Vec。
            Cow::Owned(f.into_vec())
        }
    }
}

// ================ Vec<Str/String> =================

// 定义了 `EnvConvert` trait 的实现，用于将不同类型的字符串向量转换为 `Vec<Cow<'a, NativeIntStr>>` 类型。
// 这些实现允许 `NCvt` 类型在环境变量处理过程中灵活地将原生字符串、引用的字符串向量、
// 字符串向量的引用以及字符串向量，转换为统一的格式，便于处理。
impl<'a> EnvConvert<&'a Vec<&'a str>, Vec<Cow<'a, NativeIntStr>>> for NCvt {
    // 将一个引用的字符串向量 (`&Vec<&str>`) 转换为 `Vec<Cow<'a, NativeIntStr>>`。
    // 其中，每个字符串都被转换为 `Cow` 类型，以支持共享或 owned 的字符串形式。
    fn convert(f: &'a Vec<&'a str>) -> Vec<Cow<'a, NativeIntStr>> {
        f.iter().map(|x| Self::convert(*x)).collect()
    }
}

impl<'a> EnvConvert<Vec<&'a str>, Vec<Cow<'a, NativeIntStr>>> for NCvt {
    // 将一个字符串向量 (`Vec<&str>`) 转换为 `Vec<Cow<'a, NativeIntStr>>`。
    // 该实现遍历输入向量，并对每个字符串应用 `convert` 方法，收集结果。
    fn convert(f: Vec<&'a str>) -> Vec<Cow<'a, NativeIntStr>> {
        f.iter().map(|x| Self::convert(*x)).collect()
    }
}

impl<'a> EnvConvert<&'a Vec<String>, Vec<Cow<'a, NativeIntStr>>> for NCvt {
    // 将一个引用的字符串向量 (`&Vec<String>`) 转换为 `Vec<Cow<'a, NativeIntStr>>`。
    // 这个实现不需要对字符串进行复制，而是直接转换为 `Cow`，从而优化了内存使用。
    fn convert(f: &'a Vec<String>) -> Vec<Cow<'a, NativeIntStr>> {
        f.iter().map(Self::convert).collect()
    }
}

impl<'a> EnvConvert<Vec<String>, Vec<Cow<'a, NativeIntStr>>> for NCvt {
    // 将一个字符串向量 (`Vec<String>`) 转换为 `Vec<Cow<'a, NativeIntStr>>`。
    // 此实现利用 `into_iter` 来消费 `Vec`，并将每个字符串转换为所需格式。
    fn convert(f: Vec<String>) -> Vec<Cow<'a, NativeIntStr>> {
        f.into_iter().map(Self::convert).collect()
    }
}

// 将 `OsStr` 的引用转换为本地整型表示的 `Cow`。
//
// - `input`：一个操作系统的字符串的不可变引用。
//
// 返回值是一个 `Cow`，它要么是 `OsStr` 的借来的内容，要么是其拥有内容的副本，
// 具体取决于操作系统的类型和输入的内容。
#[cfg(target_os = "windows")]
fn to_native_int_representation(input: &OsStr) -> Cow<'_, NativeIntStr> {
    Cow::Owned(input.encode_wide().collect())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn to_native_int_representation(input: &OsStr) -> Cow<'_, NativeIntStr> {
    Cow::Borrowed(input.as_bytes())
}

// 从本地整型表示转换回 `OsStr` 的 `Cow`。
//
// - `input`：一个本地整型字符串的 `Cow` 引用。
//
// 返回一个 `Cow`，它代表了操作系统的字符串，可能是借来的也可能拥有其内容。
#[cfg(target_os = "windows")]
fn from_native_int_representation(input: Cow<'_, NativeIntStr>) -> Cow<'_, OsStr> {
    Cow::Owned(OsString::from_wide(&input))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn from_native_int_representation(input: Cow<'_, NativeIntStr>) -> Cow<'_, OsStr> {
    match input {
        Cow::Borrowed(borrow) => Cow::Borrowed(OsStr::from_bytes(borrow)),
        Cow::Owned(own) => Cow::Owned(OsString::from_vec(own)),
    }
}

// 从本地整型表示拥有权转换为 `OsString`。
//
// - `input`：一个本地整型字符串。
//
// 返回一个 `OsString`，它代表了操作系统的字符串。
#[allow(clippy::needless_pass_by_value)] // needed on windows
pub fn from_native_int_representation_owned(input: NativeIntString) -> OsString {
    #[cfg(target_os = "windows")]
    {
        OsString::from_wide(&input)
    }

    #[cfg(not(target_os = "windows"))]
    {
        OsString::from_vec(input)
    }
}

// 获取单个字符的本地整型表示。
//
// - `c`：一个字符的引用。
//
// 如果字符可以被转换为本地整型表示，则返回该表示的 `Option`，
// 否则返回 `None`，这取决于操作系统。
#[cfg(target_os = "windows")]
fn get_single_native_int_value(c: &char) -> Option<NativeCharInt> {
    let mut buf = [0u16, 0];
    let s = c.encode_utf16(&mut buf);
    if s.len() == 1 {
        Some(buf[0])
    } else {
        None
    }
}

#[cfg(not(target_os = "windows"))]
pub fn get_single_native_int_value(c: &char) -> Option<NativeCharInt> {
    let mut buf = [0u8, 0, 0, 0];
    let s = c.encode_utf8(&mut buf);
    if s.len() == 1 {
        Some(buf[0])
    } else {
        None
    }
}

// 从本地整型字符值转换为字符及其本地整型表示的元组。
//
// - `ni`：一个本地整型字符表示。
//
// 如果给定的本地整型值可以被解析为一个字符，则返回这个字符及其本地整型表示的元组，
// 否则返回 `None`。
pub fn get_char_from_native_int(ni: NativeCharInt) -> Option<(char, NativeCharInt)> {
    let c_opt;
    #[cfg(target_os = "windows")]
    {
        c_opt = char::decode_utf16([ni; 1]).next().unwrap().ok();
    };

    #[cfg(not(target_os = "windows"))]
    {
        c_opt = std::str::from_utf8(&[ni; 1])
            .ok()
            .map(|x| x.chars().next().unwrap());
    };

    if let Some(c) = c_opt {
        return Some((c, ni));
    }

    None
}

/// `NativeStr`是一个包装了原生字符串表示的结构体。
/// 它使用`Cow`来优化内存使用，既可以共享不可变数据，也可以在需要时拥有自己的可变副本。
pub struct NativeStr<'a> {
    native: Cow<'a, NativeIntStr>,
}

impl<'a> NativeStr<'a> {
    /// 创建一个新的`NativeStr`实例。
    ///
    /// # 参数
    /// `str` - 一个操作系统字符串的引用。
    ///
    /// # 返回值
    /// 返回一个新的`NativeStr`实例，其中包含了给定字符串的原生整数表示。
    pub fn new(str: &'a OsStr) -> Self {
        Self {
            native: to_native_int_representation(str),
        }
    }

    /// 获取这个`NativeStr`的原生字符串表示的共享引用。
    ///
    /// # 返回值
    /// 返回一个原生字符串表示的`Cow`引用。
    pub fn native(&self) -> Cow<'a, NativeIntStr> {
        self.native.clone()
    }

    /// 将这个`NativeStr`转换为其原生字符串表示的所有权。
    ///
    /// # 返回值
    /// 返回一个原生字符串表示的所有权`Cow`。
    pub fn into_native(self) -> Cow<'a, NativeIntStr> {
        self.native
    }

    /// 检查字符串是否包含指定字符。
    ///
    /// # 参数
    /// `x` - 需要检查是否包含的字符的引用。
    ///
    /// # 返回值
    /// 如果字符串包含该字符，则返回`Some(true)`；如果不包含或字符不存在，则返回`Some(false)`；如果无法进行检查，则返回`None`。
    pub fn contains(&self, x: &char) -> Option<bool> {
        let n_c = get_single_native_int_value(x)?;
        Some(self.native.contains(&n_c))
    }

    /// 获取字符串的一个子串。
    ///
    /// # 参数
    /// `from` - 子串的起始位置。
    /// `to` - 子串的结束位置（不包含）。
    ///
    /// # 返回值
    /// 返回一个从指定位置开始到结束位置的子串的`Cow`引用。
    pub fn slice(&self, from: usize, to: usize) -> Cow<'a, OsStr> {
        let result = self.match_cow(|b| Ok::<_, ()>(&b[from..to]), |o| Ok(o[from..to].to_vec()));
        result.unwrap()
    }

    /// 根据给定的条件分割字符串一次，并返回两部分。
    ///
    /// # 参数
    /// `pred` - 用于分割字符串的条件，即要查找的字符。
    ///
    /// # 返回值
    /// 如果找到分割条件，返回一个包含分割后的两部分字符串的元组；如果未找到或无法分割，则返回`None`。
    pub fn split_once(&self, pred: &char) -> Option<(Cow<'a, OsStr>, Cow<'a, OsStr>)> {
        let n_c = get_single_native_int_value(pred)?;
        let p = self.native.iter().position(|&x| x == n_c)?;
        let before = self.slice(0, p);
        let after = self.slice(p + 1, self.native.len());
        Some((before, after))
    }

    /// 根据指定的位置分割字符串，并返回两部分。
    ///
    /// # 参数
    /// `pos` - 分割位置。
    ///
    /// # 返回值
    /// 返回一个包含分割后的两部分字符串的元组。
    pub fn split_at(&self, pos: usize) -> (Cow<'a, OsStr>, Cow<'a, OsStr>) {
        let before = self.slice(0, pos);
        let after = self.slice(pos, self.native.len());
        (before, after)
    }

    /// 从字符串中去除前缀。
    ///
    /// # 参数
    /// `prefix` - 需要去除的前缀字符串。
    ///
    /// # 返回值
    /// 如果成功去除前缀，则返回去除前缀后的字符串的`Cow`引用；如果原字符串不以该前缀开头，则返回`None`。
    pub fn strip_prefix(&self, prefix: &OsStr) -> Option<Cow<'a, OsStr>> {
        let n_prefix = to_native_int_representation(prefix);
        let result = self.match_cow(
            |b| b.strip_prefix(&*n_prefix).ok_or(()),
            |o| o.strip_prefix(&*n_prefix).map(|x| x.to_vec()).ok_or(()),
        );
        result.ok()
    }

    /// 从字符串中去除原生整数表示的前缀。
    ///
    /// # 参数
    /// `prefix` - 需要去除的原生整数表示的前缀。
    ///
    /// # 返回值
    /// 如果成功去除前缀，则返回去除前缀后的原生字符串表示的`Cow`；如果原字符串不以该前缀开头，则返回`None`。
    pub fn strip_prefix_native(&self, prefix: &OsStr) -> Option<Cow<'a, NativeIntStr>> {
        let n_prefix = to_native_int_representation(prefix);
        let result = self.match_cow_native(
            |b| b.strip_prefix(&*n_prefix).ok_or(()),
            |o| o.strip_prefix(&*n_prefix).map(|x| x.to_vec()).ok_or(()),
        );
        result.ok()
    }

    /// 根据字符串的存储方式（借用或拥有），应用给定的函数来处理字符串。
    ///
    /// # 参数
    /// `f_borrow` - 用于处理借用（不可变）字符串的函数。
    /// `f_owned` - 用于处理拥有（可变）字符串的函数。
    ///
    /// # 返回值
    /// 返回处理结果，具体类型依赖于函数`f_borrow`和`f_owned`的实现。
    fn match_cow<FnBorrow, FnOwned, Err>(
        &self,
        f_borrow: FnBorrow,
        f_owned: FnOwned,
    ) -> Result<Cow<'a, OsStr>, Err>
    where
        FnBorrow: FnOnce(&'a [NativeCharInt]) -> Result<&'a [NativeCharInt], Err>,
        FnOwned: FnOnce(&Vec<NativeCharInt>) -> Result<Vec<NativeCharInt>, Err>,
    {
        match &self.native {
            Cow::Borrowed(b) => {
                let slice = f_borrow(b);
                let os_str = slice.map(|x| from_native_int_representation(Cow::Borrowed(x)));
                os_str
            }
            Cow::Owned(o) => {
                let slice = f_owned(o);
                let os_str = slice.map(from_native_int_representation_owned);
                os_str.map(Cow::Owned)
            }
        }
    }

    /// 根据字符串的存储方式（借用或拥有），应用给定的函数来处理原生整数表示的字符串。
    ///
    /// # 参数
    /// `f_borrow` - 用于处理借用（不可变）原生整数字符串的函数。
    /// `f_owned` - 用于处理拥有（可变）原生整数字符串的函数。
    ///
    /// # 返回值
    /// 返回处理结果，具体类型依赖于函数`f_borrow`和`f_owned`的实现。
    fn match_cow_native<FnBorrow, FnOwned, Err>(
        &self,
        f_borrow: FnBorrow,
        f_owned: FnOwned,
    ) -> Result<Cow<'a, NativeIntStr>, Err>
    where
        FnBorrow: FnOnce(&'a [NativeCharInt]) -> Result<&'a [NativeCharInt], Err>,
        FnOwned: FnOnce(&Vec<NativeCharInt>) -> Result<Vec<NativeCharInt>, Err>,
    {
        match &self.native {
            Cow::Borrowed(b) => {
                let slice = f_borrow(b);
                slice.map(Cow::Borrowed)
            }
            Cow::Owned(o) => {
                let slice = f_owned(o);
                slice.map(Cow::Owned)
            }
        }
    }
}

