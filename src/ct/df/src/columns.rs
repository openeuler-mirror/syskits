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
use crate::{DF_OPT_INODES, DF_OPT_OUTPUT, DF_OPT_PRINT_TYPE};
use clap::{parser::ValueSource, ArgMatches};

/// 定义了输出表格中可能出现的列。
///
/// 输出表格中的列由 [`Column`] 枚举中的不同变体表示。
/// 每个变体都对应着输出表格中的一列。
#[derive(PartialEq, Copy, Clone, Debug)]
pub(crate) enum Column {
    /// 挂载点的来源，通常是设备名。
    Source,

    /// 总块数。
    Size,

    /// 已使用的块数。
    Used,

    /// 可用的块数。
    Avail,

    /// 已使用块数占总块数的百分比。
    Pcent,

    /// 挂载点。
    Target,

    /// 总 inode 数。
    Itotal,

    /// 已使用的 inode 数。
    Iused,

    /// 可用的 inode 数。
    Iavail,

    /// 已使用 inode 数占总 inode 数的百分比。
    Ipcent,

    /// 作为命令行参数给出的文件名。
    File,

    /// 文件系统的类型，如 "ext4" 或 "squashfs"。
    Fstype,

    /// 非特权进程可用字节的百分比。
    #[cfg(target_os = "macos")]
    Capacity,
}

/// 在定义输出表格中的列时可能出现的错误。
#[derive(Debug)]
pub(crate) enum ColumnError {
    /// 如果命令行参数中某个列出现了多次。
    MultipleColumns(String),
}

impl Column {
    /// 根据命令行参数转换为列的序列。
    ///
    /// 输出表格中显示的列集可以通过命令行参数进行指定。此函数将这些参数转换为
    /// 一个包含 [`Column`] 枚举变体的 [`Vec`]。
    ///
    /// # 错误
    ///
    /// 如果命令行参数中某个列被指定了多次，此函数将返回错误。
    pub(crate) fn from_matches(matches: &ArgMatches) -> Result<Vec<Self>, ColumnError> {
        // 根据命令行提供的选项和参数，决定输出哪些列
        match (
            matches.get_flag(DF_OPT_PRINT_TYPE),
            matches.get_flag(DF_OPT_INODES),
            matches.value_source(DF_OPT_OUTPUT) == Some(ValueSource::CommandLine),
        ) {
            (false, false, false) => Ok(vec![
                Self::Source,
                Self::Size,
                Self::Used,
                Self::Avail,
                #[cfg(target_os = "macos")]
                Self::Capacity,
                Self::Pcent,
                Self::Target,
            ]),
            (false, false, true) => {
                // 从命令行参数中解析用户指定的列
                let names = matches
                    .get_many::<String>(DF_OPT_OUTPUT)
                    .unwrap()
                    .map(|s| s.as_str());
                let mut seen: Vec<&str> = vec![];
                let mut columns = vec![];
                for name in names {
                    if seen.contains(&name) {
                        return Err(ColumnError::MultipleColumns(name.to_string()));
                    }
                    seen.push(name);
                    let column = Self::parse(name).unwrap();
                    columns.push(column);
                }
                Ok(columns)
            }
            (false, true, false) => Ok(vec![
                Self::Source,
                Self::Itotal,
                Self::Iused,
                Self::Iavail,
                Self::Ipcent,
                Self::Target,
            ]),
            (true, false, false) => Ok(vec![
                Self::Source,
                Self::Fstype,
                Self::Size,
                Self::Used,
                Self::Avail,
                #[cfg(target_os = "macos")]
                Self::Capacity,
                Self::Pcent,
                Self::Target,
            ]),
            (true, true, false) => Ok(vec![
                Self::Source,
                Self::Fstype,
                Self::Itotal,
                Self::Iused,
                Self::Iavail,
                Self::Ipcent,
                Self::Target,
            ]),
            // 命令行参数 -T 和 -i 与 --output 互斥，因此如果出现这些组合，命令行参数解析器应该先拒绝
            _ => unreachable!(),
        }
    }

    /// 将列名转换为相应的枚举变体。
    ///
    /// 有十二个有效的列名，每个变体对应一个：
    ///
    /// - "source"
    /// - "fstype"
    /// - "itotal"
    /// - "iused"
    /// - "iavail"
    /// - "ipcent"
    /// - "size"
    /// - "used"
    /// - "avail"
    /// - "pcent"
    /// - "file"
    /// - "target"
    ///
    /// # 错误
    ///
    /// 如果字符串 `s` 不是有效的列名。
    fn parse(s: &str) -> Result<Self, ()> {
        // 根据列名字符串匹配相应的枚举变体
        match s {
            "source" => Ok(Self::Source),
            "fstype" => Ok(Self::Fstype),
            "itotal" => Ok(Self::Itotal),
            "iused" => Ok(Self::Iused),
            "iavail" => Ok(Self::Iavail),
            "ipcent" => Ok(Self::Ipcent),
            "size" => Ok(Self::Size),
            "used" => Ok(Self::Used),
            "avail" => Ok(Self::Avail),
            "pcent" => Ok(Self::Pcent),
            "file" => Ok(Self::File),
            "target" => Ok(Self::Target),
            _ => Err(()),
        }
    }

    /// 返回指定列的对齐方式。
    pub(crate) fn alignment(column: &Self) -> Alignment {
        // 根据列的类型决定其在输出中的对齐方式
        match column {
            Self::Source | Self::Target | Self::File | Self::Fstype => Alignment::Left,
            _ => Alignment::Right,
        }
    }

    /// 返回指定列的最小宽度。
    pub(crate) fn min_width(column: &Self) -> usize {
        // 根据列的类型决定其最小宽度
        match column {
            // 14 = "Filesystem" 的长度加 4 个空格
            Self::Source => 14,
            Self::Used => 5,
            // 最短的表头长度为 4 个字符，因此我们将其作为最小宽度
            _ => 4,
        }
    }
}

/// 列的对齐方式。
///
/// 我们定义自己的 `Alignment` 枚举而不是使用 `std::fmt::Alignment`，因为 `df` 没有居中对齐的列，所以不需要 `Center` 变体。
pub(crate) enum Alignment {
    Left,
    Right,
}

