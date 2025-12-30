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
//! A table ([`Table`]) comprises a header row ([`TableHeader`]) and a
//! collection of data rows ([`TableRow`]), one per filesystem.
use unicode_width::UnicodeWidthStr;

use crate::blocks::blocks_to_magnitude_and_suffix;
use crate::blocks::BlocksSuffixType;
use crate::columns::Alignment;
use crate::columns::Column;
use crate::filesystem::Filesystem;
use crate::BlockSize;
use crate::DfOptions;
use ctcore::ct_fsext::CtMountInfo;
use ctcore::ct_fsext::FsUsage;

use std::fmt;
use std::ops::AddAssign;

/// A row in the filesystem usage data table.
///
/// A row comprises several pieces of information, including the
/// filesystem device, the mountpoint, the number of bytes used, etc.
pub(crate) struct TableRow {
    /// The filename given on the command-line, if given.
    file: Option<String>,

    /// Name of the device on which the filesystem lives.
    fs_device: String,

    /// Type of filesystem (for example, `"ext4"`, `"tmpfs"`, etc.).
    fs_type: String,

    /// Path at which the filesystem is mounted.
    fs_mount: String,

    /// Total number of bytes in the filesystem regardless of whether they are used.
    bytes: u64,

    /// Number of used bytes.
    bytes_used: u64,

    /// Number of available bytes.
    bytes_avail: u64,

    /// Percentage of bytes that are used, given as a float between 0 and 1.
    ///
    /// If the filesystem has zero bytes, then this is `None`.
    bytes_usage: Option<f64>,

    /// Percentage of bytes that are available, given as a float between 0 and 1.
    ///
    /// These are the bytes that are available to non-privileged processes.
    ///
    /// If the filesystem has zero bytes, then this is `None`.
    #[cfg(target_os = "macos")]
    bytes_capacity: Option<f64>,

    /// Total number of inodes in the filesystem.
    inodes: u128,

    /// Number of used inodes.
    inodes_used: u128,

    /// Number of free inodes.
    inodes_free: u128,

    /// Percentage of inodes that are used, given as a float between 0 and 1.
    ///
    /// If the filesystem has zero bytes, then this is `None`.
    inodes_usage: Option<f64>,
}

impl TableRow {
    pub(crate) fn new(source: &str) -> Self {
        Self {
            file: None,
            fs_device: source.into(),
            fs_type: "-".into(),
            fs_mount: "-".into(),
            bytes: 0,
            bytes_used: 0,
            bytes_avail: 0,
            bytes_usage: None,
            #[cfg(target_os = "macos")]
            bytes_capacity: None,
            inodes: 0,
            inodes_used: 0,
            inodes_free: 0,
            inodes_usage: None,
        }
    }
}

impl AddAssign for TableRow {
    /// 将两个行数据相加并赋值给当前行。
    ///
    /// 其中 `Row::fs_device` 字段被设置为 `"total"`，其余 `String` 类型的字段被设置为 `"-"`。
    fn add_assign(&mut self, rhs: Self) {
        // 计算各个数值字段的和。
        let bytes = self.bytes + rhs.bytes;
        let bytes_used = self.bytes_used + rhs.bytes_used;
        let bytes_avail = self.bytes_avail + rhs.bytes_avail;
        let inodes = self.inodes + rhs.inodes;
        let inodes_used = self.inodes_used + rhs.inodes_used;

        // 使用计算后的值重置当前对象。
        *self = Self {
            file: None,
            fs_device: "total".into(),
            fs_type: "-".into(),
            fs_mount: "-".into(),
            bytes,
            bytes_used,
            bytes_avail,
            // 计算使用率，若总字节为0，则不计算。
            bytes_usage: if bytes == 0 {
                None
            } else {
                // 由于某些文件系统（如ext4）的“bytes”值还包括了我们计算使用率时忽略的保留块，
                // 因此这里使用“(bytes_used + bytes_avail)”来计算使用率。
                Some(bytes_used as f64 / (bytes_used + bytes_avail) as f64)
            },
            // macOS平台的特定字段，当前未计算。
            #[cfg(target_os = "macos")]
            bytes_capacity: None,
            inodes,
            inodes_used,
            inodes_free: self.inodes_free + rhs.inodes_free,
            // 计算inode使用率，若总inode数为0，则不计算。
            inodes_usage: if inodes == 0 {
                None
            } else {
                Some(inodes_used as f64 / inodes as f64)
            },
        }
    }
}

/**
 * 实现将 `Filesystem` 转换为 `TableRow` 的方法。
 *
 * 这个转换函数提取 `Filesystem` 结构体中的信息，并将其转换为 `TableRow` 结构体的实例。
 * 它处理了文件系统统计信息，包括设备名、文件系统类型、挂载点、磁盘使用情况等，并将这些信息填充到 `TableRow` 中。
 *
 * @param fs `Filesystem` 结构体，包含文件系统的详细信息。
 * @return 返回一个填充好的 `TableRow` 实例。
 */
impl From<Filesystem> for TableRow {
    fn from(fs: Filesystem) -> Self {
        // 解构 `Filesystem` 中的 `CtMountInfo` 以获取所需字段。
        let CtMountInfo {
            dev_name,
            fs_type,
            mount_dir,
            ..
        } = fs.mount_info;
        // 解构 `Filesystem` 中的 `FsUsage` 以获取磁盘使用统计信息。
        let FsUsage {
            blocksize,
            blocks,
            bfree,
            bavail,
            files,
            ffree,
            ..
        } = fs.usage;

        // 计算已使用的字节和inode，使用 `saturating_sub` 防止溢出。
        let bused = blocks.saturating_sub(bfree);
        let fused = files.saturating_sub(ffree);

        // 构建 `TableRow` 实例。
        Self {
            file: fs.file,
            fs_device: dev_name,
            fs_type,
            fs_mount: mount_dir,
            bytes: blocksize * blocks,       // 总字节数
            bytes_used: blocksize * bused,   // 已用字节数
            bytes_avail: blocksize * bavail, // 可用字节数
            bytes_usage: if blocks == 0 {
                None
            } else {
                // 计算字节使用率，考虑了某些文件系统中的保留块。
                Some(bused as f64 / (bused + bavail) as f64)
            },
            #[cfg(target_os = "macos")]
            bytes_capacity: if bavail == 0 {
                None
            } else {
                // 计算 macOS 上的字节可用率。
                Some(bavail as f64 / ((bused + bavail) as f64))
            },
            inodes: files as u128,
            inodes_used: fused as u128,
            inodes_free: ffree as u128,
            inodes_usage: if files == 0 {
                None
            } else {
                // 计算 inode 使用率。
                Some(fused as f64 / files as f64)
            },
        }
    }
}

/// A formatter for [`TableRow`].
///
/// The `options` control how the information in the row gets formatted.
// 用于格式化表格中单行数据的结构体。
//
// 此结构体依据指定的选项及是否为汇总行，设计用来对表格中的每一行进行格式化。
// 它旨在表格格式化操作中使用，每行数据根据需要单独格式化。
pub(crate) struct TableRowFormatter<'a> {
    /// 存储待格式化的表格行的引用。
    row: &'a TableRow,

    /// 提供控制格式化行为的一系列选项，包括但不限于对齐方式、填充以及分隔符的使用。
    options: &'a DfOptions,

    /// 标记当前待格式化的行是否为汇总行。汇总行通常需要不同的格式化规则，
    /// 如加粗文本或特殊的背景颜色，以便从普通行中视觉区分。
    is_total_row: bool,
}

impl<'a> TableRowFormatter<'a> {
    /// 创建RowFormatter实例。
    ///
    /// # 参数
    /// * `row`: 表示一行数据的TableRow引用。
    /// * `options`: 表示格式化选项的Options引用。
    /// * `is_total_row`: 表示当前行是否为总计行的布尔值。
    ///
    /// # 返回值
    /// 返回一个初始化好的RowFormatter实例。
    pub(crate) fn new(row: &'a TableRow, options: &'a DfOptions, is_total_row: bool) -> Self {
        Self {
            row,
            options,
            is_total_row,
        }
    }

    /// 根据选项中的缩放因子，将输入的字节大小转换为格式化的字符串。
    ///
    /// # 参数
    /// * `size`: 输入的字节大小，类型为u64。
    ///
    /// # 返回值
    /// 返回一个表示缩放后大小的字符串。
    fn scaled_bytes(&self, size: u64) -> String {
        if let Some(h) = self.options.human_readable {
            blocks_to_magnitude_and_suffix(size.into(), BlocksSuffixType::HumanReadable(h))
        } else {
            let BlockSize::Bytes(d) = self.options.block_size;
            (size as f64 / d as f64).ceil().to_string()
        }
    }

    /// 根据选项中的缩放因子，将输入的inode数量转换为格式化的字符串。
    ///
    /// # 参数
    /// * `size`: 输入的inode数量，类型为u128。
    ///
    /// # 返回值
    /// 返回一个表示缩放后inode数量的字符串。
    fn scaled_inodes(&self, size: u128) -> String {
        if let Some(h) = self.options.human_readable {
            blocks_to_magnitude_and_suffix(size, BlocksSuffixType::HumanReadable(h))
        } else {
            size.to_string()
        }
    }

    /// 将0到1之间的浮点数转换为百分比字符串。
    ///
    /// # 参数
    /// * `fraction`: 一个表示比例的f64可选值。
    ///
    /// # 返回值
    /// 如果`fraction`为`None`，则返回字符串"-"，否则返回格式化的百分比字符串。
    fn percentage(fraction: Option<f64>) -> String {
        match fraction {
            None => "-".to_string(),
            Some(x) => format!("{:.0}%", (100.0 * x).ceil()),
        }
    }

    /// 格式化行数据，并返回一个字符串向量。
    ///
    /// # 返回值
    /// 返回一个包含格式化后数据的字符串向量。
    fn get_values(&self) -> Vec<String> {
        let mut strings = Vec::new();

        for column in &self.options.columns {
            let string = match column {
                Column::Source => {
                    if self.is_total_row {
                        "total".to_string()
                    } else {
                        self.row.fs_device.to_string()
                    }
                }
                Column::Size => self.scaled_bytes(self.row.bytes),
                Column::Used => self.scaled_bytes(self.row.bytes_used),
                Column::Avail => self.scaled_bytes(self.row.bytes_avail),
                Column::Pcent => Self::percentage(self.row.bytes_usage),

                Column::Target => {
                    if self.is_total_row && !self.options.columns.contains(&Column::Source) {
                        "total".to_string()
                    } else {
                        self.row.fs_mount.to_string()
                    }
                }
                Column::Itotal => self.scaled_inodes(self.row.inodes),
                Column::Iused => self.scaled_inodes(self.row.inodes_used),
                Column::Iavail => self.scaled_inodes(self.row.inodes_free),
                Column::Ipcent => Self::percentage(self.row.inodes_usage),
                Column::File => self.row.file.as_ref().unwrap_or(&"-".into()).to_string(),

                Column::Fstype => self.row.fs_type.to_string(),
                #[cfg(target_os = "macos")]
                Column::Capacity => Self::percentage(self.row.bytes_capacity),
            };

            strings.push(string);
        }

        strings
    }
}

/// A HeaderMode defines what header labels should be shown.
pub(crate) enum TableHeaderMode {
    Default,
    // the user used -h or -H
    HumanReadable,
    // the user used -P
    PosixPortability,
    // the user used --output
    Output,
}

impl Default for TableHeaderMode {
    fn default() -> Self {
        Self::Default
    }
}

/// The data of the header row.
// 定义头部行的数据结构。
struct TableHeader {}

impl TableHeader {
    /// 根据指定的选项返回列头。
    ///
    /// `options` 控制返回哪些列头。
    fn get_headers(options: &DfOptions) -> Vec<String> {
        let mut headers = Vec::new(); // 初始化一个空的字符串向量来存放头部信息。

        // 遍历选项中的列，为每列生成相应的头部信息。
        for column in &options.columns {
            let header = match column {
                // 根据列类型生成相应的头部字符串。
                Column::Source => String::from("Filesystem"),
                Column::Size => match options.header_mode {
                    TableHeaderMode::HumanReadable => String::from("Size"),
                    TableHeaderMode::PosixPortability => {
                        // 为了POSIX兼容性，使用块大小格式化"Size"头部。
                        format!("{}-blocks", options.block_size.as_u64())
                    }
                    _ => format!("{}-blocks", options.block_size),
                },
                Column::Used => String::from("Used"),
                Column::Avail => match options.header_mode {
                    TableHeaderMode::HumanReadable | TableHeaderMode::Output => {
                        String::from("Avail")
                    }
                    _ => String::from("Available"),
                },
                Column::Pcent => match options.header_mode {
                    TableHeaderMode::PosixPortability => String::from("Capacity"),
                    _ => String::from("Use%"),
                },
                Column::Target => String::from("Mounted on"),
                Column::Itotal => String::from("Inodes"),
                Column::Iused => String::from("IUsed"),
                Column::Iavail => String::from("IFree"),
                Column::Ipcent => String::from("IUse%"),
                Column::File => String::from("File"),
                Column::Fstype => String::from("Type"),
                #[cfg(target_os = "macos")]
                Column::Capacity => String::from("Capacity"),
            };

            headers.push(header); // 将生成的头部信息添加到向量中。
        }

        headers // 返回构建完成的头部信息向量。
    }
}

/// The output table.
// 表格结构体，用于表示一个格式化的表格，包含列对齐方式、实际数据行以及各列宽度。
pub(crate) struct Table {
    alignments: Vec<Alignment>, // 列对齐方式集合
    rows: Vec<Vec<String>>,     // 表格数据行
    widths: Vec<usize>,         // 各列宽度集合
}

impl Table {
    // Table 结构体的实现块

    // 新建一个 Table 实例，根据提供的选项和文件系统信息填充数据。
    // 计算列宽以适应表头名称和文件系统数据，并在需要时添加总计行。
    pub(crate) fn new(options: &DfOptions, filesystems: Vec<Filesystem>) -> Self {
        // 初始化列宽，基于表头和选项设置。
        // 根据每列数据调整宽度以容纳最长字符串。
        let headers = TableHeader::get_headers(options);
        let mut widths: Vec<_> = options
            .columns
            .iter()
            .enumerate()
            .map(|(i, col)| Column::min_width(col).max(headers[i].len()))
            .collect();

        // 以表头作为首行开始构建表格数据。
        let mut rows = vec![headers];

        // 初始化一个累加器，用于计算所有文件系统的总大小和使用情况。
        let mut total = TableRow::new("total");

        // 遍历每个文件系统，根据条件向表格中添加数据行。
        for filesystem in filesystems {
            // 根据选项决定是否显示当前文件系统的数据行。
            if options.show_all_fs || filesystem.usage.blocks > 0 {
                let row = TableRow::from(filesystem);
                let fmt = TableRowFormatter::new(&row, options, false);
                let values = fmt.get_values();
                total += row;

                rows.push(values);
            }
        }

        // 如选项所指定，添加总计行到表格末尾。
        if options.show_total {
            let total_row = TableRowFormatter::new(&total, options, true);
            rows.push(total_row.get_values());
        }

        // 根据数据行中的长值调整列宽。
        // 在添加总计行后执行此操作。
        for row in &rows {
            for (i, value) in row.iter().enumerate() {
                if UnicodeWidthStr::width(value.as_str()) > widths[i] {
                    widths[i] = UnicodeWidthStr::width(value.as_str());
                }
            }
        }

        // 构建并返回新的 Table 实例。
        Self {
            rows,
            widths,
            alignments: Self::get_alignments(&options.columns),
        }
    }

    // 获取列对齐方式的辅助函数，根据列类型确定。
    fn get_alignments(columns: &Vec<Column>) -> Vec<Alignment> {
        let mut alignments = Vec::new();

        // 遍历列，获取并存储每一列的对齐方式。
        for column in columns {
            alignments.push(Column::alignment(column));
        }

        alignments
    }
}

// 为 Table 结构体实现 fmt::Display trait，用于格式化输出表格。

// fmt 方法将表格格式化输出至指定的格式化器，遵循定义好的格式规则。
// 它处理列的对齐，并确保表格视觉上的对齐。
impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // 遍历所有行，格式化每一行的列，依据其对齐方式和宽度。
        let mut row_iter = self.rows.iter().peekable();
        while let Some(row) = row_iter.next() {
            let mut col_iter = row.iter().enumerate().peekable();
            while let Some((i, elem)) = col_iter.next() {
                let is_last_col = col_iter.peek().is_none();

                // 应用列的对齐规则至数据。
                match self.alignments[i] {
                    Alignment::Left => {
                        if is_last_col {
                            // 最后一列不添加尾随空格。
                            write!(f, "{}", elem)?;
                        } else {
                            write!(f, "{:<width$}", elem, width = self.widths[i])?;
                        }
                    }
                    Alignment::Right => write!(f, "{:>width$}", elem, width = self.widths[i])?,
                }

                // 列之间添加分隔符，除非是最后一列。
                if !is_last_col {
                    write!(f, " ")?;
                }
            }

            // 每行结束后添加换行，除非是最后一行。
            if row_iter.peek().is_some() {
                writeln!(f)?;
            }
        }

        Ok(()) // 成功完成格式化输出。
    }
}

#[cfg(test)]
mod tests {

    use std::vec;

    use crate::blocks::BlocksHumanReadable;
    use crate::columns::Column;
    use crate::table::{Table, TableHeader, TableHeaderMode, TableRow, TableRowFormatter};
    use crate::{BlockSize, DfOptions};

    const COLUMNS_WITH_FS_TYPE: [Column; 7] = [
        Column::Source,
        Column::Fstype,
        Column::Size,
        Column::Used,
        Column::Avail,
        Column::Pcent,
        Column::Target,
    ];
    const COLUMNS_WITH_INODES: [Column; 6] = [
        Column::Source,
        Column::Itotal,
        Column::Iused,
        Column::Iavail,
        Column::Ipcent,
        Column::Target,
    ];

    impl Default for TableRow {
        fn default() -> Self {
            Self {
                file: Some("/path/to/file".to_string()),
                fs_device: "my_device".to_string(),
                fs_type: "my_type".to_string(),
                fs_mount: "my_mount".to_string(),

                bytes: 100,
                bytes_used: 25,
                bytes_avail: 75,
                bytes_usage: Some(0.25),

                #[cfg(target_os = "macos")]
                bytes_capacity: Some(0.5),

                inodes: 10,
                inodes_used: 2,
                inodes_free: 8,
                inodes_usage: Some(0.2),
            }
        }
    }

    #[test]
    fn test_default_header() {
        let options = DfOptions::default();
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!(
                "Filesystem",
                "1K-blocks",
                "Used",
                "Available",
                "Use%",
                "Mounted on"
            )
        );
    }

    #[test]
    fn test_header_with_fs_type() {
        let options = DfOptions {
            columns: COLUMNS_WITH_FS_TYPE.to_vec(),
            ..Default::default()
        };
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!(
                "Filesystem",
                "Type",
                "1K-blocks",
                "Used",
                "Available",
                "Use%",
                "Mounted on"
            )
        );
    }

    #[test]
    fn test_header_with_inodes() {
        let options = DfOptions {
            columns: COLUMNS_WITH_INODES.to_vec(),
            ..Default::default()
        };
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!(
                "Filesystem",
                "Inodes",
                "IUsed",
                "IFree",
                "IUse%",
                "Mounted on"
            )
        );
    }

    #[test]
    fn test_header_with_block_size_1024() {
        let options = DfOptions {
            block_size: BlockSize::Bytes(3 * 1024),
            ..Default::default()
        };
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!(
                "Filesystem",
                "3K-blocks",
                "Used",
                "Available",
                "Use%",
                "Mounted on"
            )
        );
    }

    #[test]
    fn test_human_readable_header() {
        let options = DfOptions {
            header_mode: TableHeaderMode::HumanReadable,
            ..Default::default()
        };
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!("Filesystem", "Size", "Used", "Avail", "Use%", "Mounted on")
        );
    }

    #[test]
    fn test_posix_portability_header() {
        let options = DfOptions {
            header_mode: TableHeaderMode::PosixPortability,
            ..Default::default()
        };
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!(
                "Filesystem",
                "1024-blocks",
                "Used",
                "Available",
                "Capacity",
                "Mounted on"
            )
        );
    }

    #[test]
    fn test_output_header() {
        let options = DfOptions {
            header_mode: TableHeaderMode::Output,
            ..Default::default()
        };
        assert_eq!(
            TableHeader::get_headers(&options),
            vec!(
                "Filesystem",
                "1K-blocks",
                "Used",
                "Avail",
                "Use%",
                "Mounted on"
            )
        );
    }

    #[test]
    fn test_row_formatter() {
        let options = DfOptions {
            block_size: BlockSize::Bytes(1),
            ..Default::default()
        };
        let row = TableRow {
            fs_device: "my_device".to_string(),
            fs_mount: "my_mount".to_string(),

            bytes: 100,
            bytes_used: 25,
            bytes_avail: 75,
            bytes_usage: Some(0.25),

            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(
            fmt.get_values(),
            vec!("my_device", "100", "25", "75", "25%", "my_mount")
        );
    }

    #[test]
    fn test_row_formatter_with_fs_type() {
        let options = DfOptions {
            columns: COLUMNS_WITH_FS_TYPE.to_vec(),
            block_size: BlockSize::Bytes(1),
            ..Default::default()
        };
        let row = TableRow {
            fs_device: "my_device".to_string(),
            fs_type: "my_type".to_string(),
            fs_mount: "my_mount".to_string(),

            bytes: 100,
            bytes_used: 25,
            bytes_avail: 75,
            bytes_usage: Some(0.25),

            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(
            fmt.get_values(),
            vec!("my_device", "my_type", "100", "25", "75", "25%", "my_mount")
        );
    }

    #[test]
    fn test_row_formatter_with_inodes() {
        let options = DfOptions {
            columns: COLUMNS_WITH_INODES.to_vec(),
            block_size: BlockSize::Bytes(1),
            ..Default::default()
        };
        let row = TableRow {
            fs_device: "my_device".to_string(),
            fs_mount: "my_mount".to_string(),

            inodes: 10,
            inodes_used: 2,
            inodes_free: 8,
            inodes_usage: Some(0.2),

            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(
            fmt.get_values(),
            vec!("my_device", "10", "2", "8", "20%", "my_mount")
        );
    }

    #[test]
    fn test_row_formatter_with_bytes_and_inodes() {
        let options = DfOptions {
            columns: vec![Column::Size, Column::Itotal],
            block_size: BlockSize::Bytes(100),
            ..Default::default()
        };
        let row = TableRow {
            bytes: 100,
            inodes: 10,
            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(fmt.get_values(), vec!("1", "10"));
    }

    #[test]
    fn test_row_formatter_with_human_readable_si() {
        let options = DfOptions {
            human_readable: Some(BlocksHumanReadable::Decimal),
            columns: COLUMNS_WITH_FS_TYPE.to_vec(),
            ..Default::default()
        };
        let row = TableRow {
            fs_device: "my_device".to_string(),
            fs_type: "my_type".to_string(),
            fs_mount: "my_mount".to_string(),

            bytes: 4000,
            bytes_used: 1000,
            bytes_avail: 3000,
            bytes_usage: Some(0.25),

            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(
            fmt.get_values(),
            vec!("my_device", "my_type", "4k", "1k", "3k", "25%", "my_mount")
        );
    }

    #[test]
    fn test_row_formatter_with_human_readable_binary() {
        let options = DfOptions {
            human_readable: Some(BlocksHumanReadable::Binary),
            columns: COLUMNS_WITH_FS_TYPE.to_vec(),
            ..Default::default()
        };
        let row = TableRow {
            fs_device: "my_device".to_string(),
            fs_type: "my_type".to_string(),
            fs_mount: "my_mount".to_string(),

            bytes: 4096,
            bytes_used: 1024,
            bytes_avail: 3072,
            bytes_usage: Some(0.25),

            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(
            fmt.get_values(),
            vec!("my_device", "my_type", "4K", "1K", "3K", "25%", "my_mount")
        );
    }

    #[test]
    fn test_row_formatter_with_round_up_usage() {
        let options = DfOptions {
            columns: vec![Column::Pcent],
            ..Default::default()
        };
        let row = TableRow {
            bytes_usage: Some(0.251),
            ..Default::default()
        };
        let fmt = TableRowFormatter::new(&row, &options, false);
        assert_eq!(fmt.get_values(), vec!("26%"));
    }

    #[test]
    fn test_row_formatter_with_round_up_byte_values() {
        fn get_formatted_values(bytes: u64, bytes_used: u64, bytes_avail: u64) -> Vec<String> {
            let options = DfOptions {
                block_size: BlockSize::Bytes(1000),
                columns: vec![Column::Size, Column::Used, Column::Avail],
                ..Default::default()
            };

            let row = TableRow {
                bytes,
                bytes_used,
                bytes_avail,
                ..Default::default()
            };
            TableRowFormatter::new(&row, &options, false).get_values()
        }

        assert_eq!(get_formatted_values(100, 100, 0), vec!("1", "1", "0"));
        assert_eq!(get_formatted_values(100, 99, 1), vec!("1", "1", "1"));
        assert_eq!(get_formatted_values(1000, 1000, 0), vec!("1", "1", "0"));
        assert_eq!(get_formatted_values(1001, 1000, 1), vec!("2", "1", "1"));
    }

    #[test]
    fn test_row_converter_with_invalid_numbers() {
        // copy from wsl linux
        let d = crate::Filesystem {
            file: None,
            mount_info: crate::CtMountInfo {
                dev_id: "28".to_string(),
                dev_name: "none".to_string(),
                fs_type: "9p".to_string(),
                mount_dir: "/usr/lib/wsl/drivers".to_string(),
                mount_option: "ro,nosuid,nodev,noatime".to_string(),
                mount_root: "/".to_string(),
                remote: false,
                dummy: false,
            },
            usage: crate::table::FsUsage {
                blocksize: 4096,
                blocks: 244029695,
                bfree: 125085030,
                bavail: 125085030,
                bavail_top_bit_set: false,
                files: 999,
                ffree: 1000000,
            },
        };

        let row = TableRow::from(d);

        assert_eq!(row.inodes_used, 0);
    }

    #[test]
    fn test_table_column_width_computation_include_total_row() {
        let d1 = crate::Filesystem {
            file: None,
            mount_info: crate::CtMountInfo {
                dev_id: "28".to_string(),
                dev_name: "none".to_string(),
                fs_type: "9p".to_string(),
                mount_dir: "/usr/lib/wsl/drivers".to_string(),
                mount_option: "ro,nosuid,nodev,noatime".to_string(),
                mount_root: "/".to_string(),
                remote: false,
                dummy: false,
            },
            usage: crate::table::FsUsage {
                blocksize: 4096,
                blocks: 244029695,
                bfree: 125085030,
                bavail: 125085030,
                bavail_top_bit_set: false,
                files: 99999999999,
                ffree: 999999,
            },
        };

        let filesystems = vec![d1.clone(), d1];

        let mut options = DfOptions {
            show_total: true,
            columns: vec![
                Column::Source,
                Column::Itotal,
                Column::Iused,
                Column::Iavail,
            ],
            ..Default::default()
        };

        let table_w_total = Table::new(&options, filesystems.clone());
        assert_eq!(
            table_w_total.to_string(),
            "Filesystem           Inodes        IUsed   IFree\n\
             none            99999999999  99999000000  999999\n\
             none            99999999999  99999000000  999999\n\
             total          199999999998 199998000000 1999998"
        );

        options.show_total = false;

        let table_w_o_total = Table::new(&options, filesystems);
        assert_eq!(
            table_w_o_total.to_string(),
            "Filesystem          Inodes       IUsed  IFree\n\
             none           99999999999 99999000000 999999\n\
             none           99999999999 99999000000 999999"
        );
    }

    #[test]
    fn test_row_accumulation_u64_overflow() {
        let total = u64::MAX as u128;
        let used1 = 3000u128;
        let used2 = 50000u128;

        let mut row1 = TableRow {
            inodes: total,
            inodes_used: used1,
            inodes_free: total - used1,
            ..Default::default()
        };

        let row2 = TableRow {
            inodes: total,
            inodes_used: used2,
            inodes_free: total - used2,
            ..Default::default()
        };

        row1 += row2;

        assert_eq!(row1.inodes, total * 2);
        assert_eq!(row1.inodes_used, used1 + used2);
        assert_eq!(row1.inodes_free, total * 2 - used1 - used2);
    }
}