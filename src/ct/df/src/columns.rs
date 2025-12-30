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

#[cfg(test)]
mod tests_from_matches {
    use crate::columns::Column;
    use crate::ct_app;
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use tempfile::Builder;

    #[test]
    fn test_from_matches_a() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-a"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_all() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--all"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_k() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BK"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_k() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bk"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_k() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=K"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_k() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=k"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_m() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BM"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_m() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bm"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_m() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=M"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_m() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=m"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }
    #[test]
    fn test_from_matches_uppercase_b_g() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BG"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_g() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bg"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_g() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=G"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_g() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=g"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_t() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BT"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_t() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bt"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_t() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=T"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_t() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=t"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }
    #[test]
    fn test_from_matches_uppercase_b_p() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BP"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_p() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bp"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_p() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=P"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_p() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=p"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_e() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BE"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_e() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Be"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_e() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=E"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_e() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=e"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_z() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BZ"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_z() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bz"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_z() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Z"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_z() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=z"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_y() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BY"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_y() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-By"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_y() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Y"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_y() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=y"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_block_size_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_k_total_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BK", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_k_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bk", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_k_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=K", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_k_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=k", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_m_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BM", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_m_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bm", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_m_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=M", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_m_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=m", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }
    #[test]
    fn test_from_matches_uppercase_b_g_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BG", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_g_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bg", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_g_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=G", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_g_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=g", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_t_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BT", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_t_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bt", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_t_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=T", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_t_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=t", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }
    #[test]
    fn test_from_matches_uppercase_b_p_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BP", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_p_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bp", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_p_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=P", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_p_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=p", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_e_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BE", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_e_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Be", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_e_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=E", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_e_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=e", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_z_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BZ", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_z_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-Bz", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_z_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Z", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_z_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=z", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_y_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BY", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_b_y_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-By", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_block_size_y_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=Y", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_lowercase_block_size_y_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--block-size=y", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_uppercase_b_k_total() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-BK", "--total"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_human_readable_binary() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--human-readable"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_human_readable_decimal() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--human-readable"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_h_binary() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-h"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_h_decimal() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-h"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_si() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-H"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_si_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--si"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_inodes() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-i"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Itotal,
                Column::Iused,
                Column::Iavail,
                Column::Ipcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_inodes_whole() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--inodes"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Itotal,
                Column::Iused,
                Column::Iavail,
                Column::Ipcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_k() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-k"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_l() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-l"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_local() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--local"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_l_local() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-l", "--local"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_no_sync() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--no-sync"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_l_no_sync() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-l", "--no-sync"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }
    #[test]
    fn test_from_matches_local_no_sync() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--local", "--no-sync"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_local_no_sync_output_source() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=source",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Source,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_fstype() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=fstype",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Fstype,]);
    }
    #[test]
    fn test_from_matches_local_no_sync_output_itotal() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=itotal",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Itotal,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_iused() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=iused",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Iused,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_iavail() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=iavail",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Iavail,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_ipcent() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=ipcent",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Ipcent,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_size() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=size",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Size,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_used() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=used",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Used,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_avail() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=avail",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Avail,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_pcent() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=pcent",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Pcent,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_file() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=file",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::File,]);
    }

    #[test]
    fn test_from_matches_local_no_sync_output_target() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![
            ctcore::ct_util_name(),
            df_dir,
            "--local",
            "--no-sync",
            "--output=target",
        ];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Target,]);
    }
    #[test]
    fn test_from_matches_output_source() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=source"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Source,]);
    }

    #[test]
    fn test_from_matches_output_fstype() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=fstype"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Fstype,]);
    }

    #[test]
    fn test_from_matches_output_itotal() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=itotal"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Itotal,]);
    }

    #[test]
    fn test_from_matches_output_iused() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=iused"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Iused,]);
    }

    #[test]
    fn test_from_matches_output_iavail() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=iavail"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Iavail,]);
    }

    #[test]
    fn test_from_matches_output_ipcent() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=ipcent"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Ipcent,]);
    }

    #[test]
    fn test_from_matches_output_size() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=size"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Size,]);
    }

    #[test]
    fn test_from_matches_output_used() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=used"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Used,]);
    }

    #[test]
    fn test_from_matches_output_avail() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=avail"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Avail,]);
    }

    #[test]
    fn test_from_matches_output_pcent() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=pcent"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Pcent,]);
    }

    #[test]
    fn test_from_matches_output_file() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=file"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::File,]);
    }

    #[test]
    fn test_from_matches_output_target() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--output=target"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(columns, vec![Column::Target,]);
    }

    #[test]
    fn test_from_matches_p() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-P"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_portability() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "--portability"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

    #[test]
    fn test_from_matches_p_portability() {
        let temp_dir = Builder::new()
            .prefix("tests_ct_app_file1")
            .tempdir()
            .unwrap();
        let sub_dir_path = temp_dir.path().join("sub_dir");
        fs::create_dir(&sub_dir_path).unwrap();
        let test_file_1 = sub_dir_path.join("test_file_1.txt");
        File::create(&test_file_1).unwrap();
        let mut file = File::create(&test_file_1).unwrap();
        let _ = test_file_1.to_str().unwrap();

        let content = "aaaa.\n\
                   bbbb.\n\
                   cccc.\n\
                   dddd.\n";
        file.write_all(content.as_bytes()).unwrap();

        let df_dir = sub_dir_path.to_str().unwrap();

        let command = ct_app();
        let args = vec![ctcore::ct_util_name(), df_dir, "-P", "--portability"];
        let result = command.try_get_matches_from(args);
        let binding = result.unwrap();

        let columns = Column::from_matches(&binding).unwrap();
        assert_eq!(
            columns,
            vec![
                Column::Source,
                Column::Size,
                Column::Used,
                Column::Avail,
                Column::Pcent,
                Column::Target,
            ]
        );
    }

}