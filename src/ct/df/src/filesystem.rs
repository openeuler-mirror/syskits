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
//! A [`Filesystem`] struct represents a device containing a
//! filesystem mounted at a particular directory. It also includes
//! information on amount of space available and amount of space used.

use std::path::Path;

#[cfg(unix)]
use ctcore::ct_fsext::statfs;
use ctcore::ct_fsext::CtMountInfo;
use ctcore::ct_fsext::FsUsage;

/// 文件系统的基本表示。
///
/// `Filesystem` 结构体代表一个挂载在特定目录上的设备文件系统。
/// 通过字段 [`Filesystem::mount_info`] 可以获取挂载信息，而字段
/// [`Filesystem::usage`] 提供了关于文件系统空间使用情况的统计信息。
#[derive(Debug, Clone)]
pub(crate) struct Filesystem {
    /// 命令行中指定的文件（如果有）。
    ///
    /// 当使用 `df` 命令并附带位置参数时，它会显示包含该指定文件的
    /// 文件系统的使用情况信息。如果指定了文件，此字段将包含该文件名。
    pub file: Option<String>,

    /// 挂载设备、挂载目录及相关选项的信息。
    pub mount_info: CtMountInfo,

    /// 文件系统空间使用情况的详细信息。
    pub usage: FsUsage,
}

/// 根据给定的文件系统路径查找最匹配的挂载信息。
///
/// 此函数从 `mounts` 中返回与 `path` 所在的挂载点相匹配的项。如果没有匹配项，
/// 则返回 [`None`]。如果有两个或更多匹配项，则返回其设备名称与输入路径对应的单个
/// [`CtMountInfo`] 实例。
///
/// 如果 `canonicalize` 参数为 `true`，则在检查是否与任何挂载目录匹配前，
/// 先对 `path` 进行规范化处理。
///
/// # 参见
///
/// * [`Path::canonicalize`]
/// * [`CtMountInfo::mount_dir`]
fn filesystem_mount_info_from_path<P>(
    mounts: &[CtMountInfo],
    path: P,
    // 主要用于测试目的。
    canonicalize: bool,
) -> Option<&CtMountInfo>
/*
在给出的 Rust 函数定义中，where 关键字用于指定泛型参数 P 的约束条件。
具体来说，这里声明了 P 必须实现 AsRef<Path> trait。这意味着传入的类型 P 需要能够提供一个对 std::path::Path 的引用，
这是为了能够在函数内部对路径进行操作，比如调用 as_ref() 转换为 Path 或执行路径规范化等。
where 子句在这里的作用是定义泛型参数的限制，确保传入的类型能够满足函数内部操作的需求，即能够转换为并操作 Path 类型对象。
 */
where
    P: AsRef<Path>,
{
    // 根据需要规范化路径
    let path = if canonicalize {
        path.as_ref().canonicalize().ok()?
    } else {
        path.as_ref().to_path_buf()
    };

    // 查找与输入路径匹配的潜在挂载点
    let current_mount_point = mounts
        .iter()
        // 构造挂载信息及其规范化的设备名称对
        // （注意：这一步骤直接访问了实际文件系统，影响测试性）
        .map(|m| (m, std::fs::canonicalize(&m.dev_name)))
        // 过滤掉不存在的路径
        .filter(|m| m.1.is_ok())
        .map(|m| (m.0, m.1.unwrap()))
        // 寻找与输入路径对应的规范化的设备名称
        .find(|m| m.1 == path)
        .map(|m| m.0);

    // 若未找到直接匹配项，则按挂载目录长度选取最长匹配
    current_mount_point.or_else(|| {
        mounts
            .iter()
            .filter(|mi| path.starts_with(&mi.mount_dir))
            .max_by_key(|mi| mi.mount_dir.len())
    })
}

impl Filesystem {
    // 构造一个新的Filesystem实例。
    //
    // 根据提供的挂载信息（mount_info）和可选的文件路径（file）创建一个Filesystem对象。
    // 根据不同的操作系统，`mount_info`中的不同字段将被用来构造文件系统的路径。
    // 在Unix系统上，使用`dev_name`；而在Windows上，则使用`dev_id`。
    // 如果`mount_info`中提供了挂载目录（mount_dir），则使用该目录。
    // 此外，会根据操作系统类型收集文件系统的使用情况信息。
    //
    // 参数:
    // - mount_info: 提供了关于文件系统挂载点的详细信息。
    // - file: 可选的文件路径字符串，如果提供，将被包含在Filesystem对象中。
    // 返回值:
    // - 根据提供的信息构造的Filesystem对象的Option包装体，如果无法构造，则返回None。
    pub(crate) fn new(mount_info: CtMountInfo, file: Option<String>) -> Option<Self> {
        // 根据操作系统选择正确的路径字段
        let _stat_path = if mount_info.mount_dir.is_empty() {
            #[cfg(unix)]
            {
                mount_info.dev_name.clone()
            }
            #[cfg(windows)]
            {
                mount_info.dev_id.clone()
            }
        } else {
            mount_info.mount_dir.clone()
        };

        // 收集文件系统使用情况信息
        #[cfg(unix)]
        let usage = FsUsage::new(statfs(_stat_path).ok()?);
        #[cfg(windows)]
        let usage = FsUsage::new(Path::new(&_stat_path));

        Some(Self {
            mount_info,
            usage,
            file,
        })
    }

    /// 根据给定路径查找并创建最匹配的文件系统。
    ///
    /// 该函数会从`mounts`数组中找出`path`所挂载的文件系统元素，并返回一个新的`Filesystem`对象。
    /// 如果没有匹配项，则返回`None`。如果有两个或更多匹配项，则返回挂载目录最长的那个`Filesystem`对象。
    /// 在检查挂载目录是否匹配之前，`path`会被规范化。
    ///
    /// 参数:
    /// - mounts: 一个包含多个挂载信息的数组。
    /// - path: 要查找的文件或目录的路径。
    ///
    /// 返回值:
    /// - 找到的最匹配的`Filesystem`对象的Option包装体，如果没有找到，则返回None。
    ///
    /// # 参考
    ///
    /// - [`Path::canonicalize`]
    /// - [`CtMountInfo::mount_dir`]
    ///
    pub(crate) fn from_path<P>(mounts: &[CtMountInfo], path: P) -> Option<Self>
    where
        P: AsRef<Path>,
    {
        let file = path.as_ref().display().to_string();
        let canonicalize = true;
        // 根据路径查找最匹配的挂载信息
        let mount_info = filesystem_mount_info_from_path(mounts, path, canonicalize)?;
        // TODO: 优化以避免克隆`mount_info`。
        let mount_info = (*mount_info).clone();
        Self::new(mount_info, Some(file))
    }
}

