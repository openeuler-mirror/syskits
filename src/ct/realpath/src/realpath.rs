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

//! 打印经过解析的绝对路径文件名；
//!除最后一部分外，文件名的所有组成部分必须存在

use clap::{
    Arg, ArgAction, ArgMatches, Command, builder::NonEmptyStringValueParser, crate_version,
};
use ctcore::ct_fs::make_path_relative_to;
use ctcore::{
    Tool,
    ct_display::Quotable,
    ct_error::{CTResult, FromIo, UClapError},
    ct_format_usage,
    ct_fs::{MissingHandling, ResolveMode, canonicalize},
    ct_help_about, ct_help_usage,
    ct_line_ending::CtLineEnding,
    ct_show_if_err,
};
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};

const REALPATH_ABOUT: &str = ct_help_about!("realpath.md");
const REALPATH_USAGE: &str = ct_help_usage!("realpath.md");

mod realpath_flags {
    // 在输出的路径后面添加一个空字符（null 字符），而不是换行符。
    pub const REALPATH_QUIET: &str = "quiet";

    // 不解析符号链接，直接返回路径。
    pub const REALPATH_STRIP: &str = "strip";

    // 在输出的路径后面添加一个空字符（null 字符），而不是换行符。
    pub const REALPATH_ZERO: &str = "zero";

    // 使用物理路径解析符号链接，不解析符号链接。
    pub const REALPATH_PHYSICAL: &str = "physical";

    // 使用逻辑路径解析符号链接（默认行为）。
    pub const REALPATH_LOGICAL: &str = "logical";

    // 返回绝对路径，即使路径中的某些部分不存在。
    pub const REALPATH_CANONICALIZE_MISSING: &str = "canonicalize-missing";

    // 只返回存在的文件的绝对路径。如果路径中的任何部分不存在，则返回错误。
    pub const REALPATH_CANONICALIZE_EXISTING: &str = "canonicalize-existing";

    // 将输出的路径相对于指定的目录 DIR。也就是说，输出的路径将是相对于 DIR 的相对路径，而不是绝对路径。
    pub const REALPATH_RELATIVE_TO: &str = "relative-to";

    // 当与 --relative-to 一起使用时，如果路径不在 DIR 目录下，则输出绝对路径。也就是说，如果路径在 DIR 目录下，则输出相对于 DIR 的相对路径；否则，输出绝对路径。
    pub const REALPATH_RELATIVE_BASE: &str = "relative-base";

    pub const REALPATH_ARG_FILES: &str = "files";
}

struct RealpathFlags {
    is_quiet: bool,
    relative_to: Option<PathBuf>,
    relative_base: Option<PathBuf>,
    files: Vec<PathBuf>,
    can_mode: MissingHandling,
    resolve_mode: ResolveMode,
    line_ending: CtLineEnding,
}

impl RealpathFlags {
    // 创建 RealpathFlags 实例的构造函数
    // 该函数从 ArgMatches 对象中提取参数，并根据这些参数构建 RealpathFlags 实例
    // 参数:
    // - matches: ArgMatches 类型，包含命令行参数的匹配结果
    // 返回值:
    // - CTResult<Self> 类型，表示构造 RealpathFlags 实例的结果，可能包含错误
    fn new(matches: ArgMatches) -> CTResult<Self> {
        // 提取文件路径参数并转换为 PathBuf 类型的向量
        let files: Vec<PathBuf> = matches
            .get_many::<String>(realpath_flags::REALPATH_ARG_FILES)
            .unwrap()
            .map(PathBuf::from)
            .collect();

        // 提取是否使用零字符结尾的标志，并据此确定行尾符类型
        let is_zero = matches.get_flag(realpath_flags::REALPATH_ZERO);
        let line_ending = CtLineEnding::from_zero_flag(is_zero);

        // 提取是否进行现有路径规范化的标志
        let is_canonicalize_existing =
            matches.get_flag(realpath_flags::REALPATH_CANONICALIZE_EXISTING);
        // 提取是否进行缺失路径规范化的标志
        let is_canonicalize_missing =
            matches.get_flag(realpath_flags::REALPATH_CANONICALIZE_MISSING);
        // 根据上述标志确定路径处理模式
        let can_mode = if is_canonicalize_existing {
            MissingHandling::Existing
        } else if is_canonicalize_missing {
            MissingHandling::Missing
        } else {
            MissingHandling::Normal
        };

        // 提取是否进行符号链接剥离的标志
        let is_strip = matches.get_flag(realpath_flags::REALPATH_STRIP);
        // 提取是否进行逻辑解析的标志
        let is_logical = matches.get_flag(realpath_flags::REALPATH_LOGICAL);
        // 根据上述标志确定路径解析模式
        let resolve_mode = if is_strip {
            ResolveMode::None
        } else if is_logical {
            ResolveMode::Logical
        } else {
            ResolveMode::Physical
        };

        // 提取相对路径基准的参数
        let relative_to = matches
            .get_one::<String>(realpath_flags::REALPATH_RELATIVE_TO)
            .cloned()
            .map(PathBuf::from);
        // 提取相对路径基础的参数
        let relative_base = matches
            .get_one::<String>(realpath_flags::REALPATH_RELATIVE_BASE)
            .cloned()
            .map(PathBuf::from);
        // 根据相对路径参数和处理模式，准备相对路径选项
        let (relative_to, relative_base) = RealpathFlags::realpath_prepare_relative_options(
            &relative_to,
            &relative_base,
            can_mode,
            resolve_mode,
        )?;

        // 提取是否安静模式的标志
        let is_quiet = matches.get_flag(realpath_flags::REALPATH_QUIET);
        // 构造并返回 RealpathFlags 实例
        Ok(RealpathFlags {
            is_quiet,
            relative_to,
            relative_base,
            files,
            can_mode,
            resolve_mode,
            line_ending,
        })
    }

    /// 准备 `--relative-to` 和 `--relative-base` 选项。
    /// 将这些选项转换为绝对路径。
    /// 检查 `--relative-to` 是否是 `--relative-base` 的子路径，
    /// 如果不是，则将它们的值置为 `None`。
    ///
    /// # 参数
    /// - `relative_to`: 可选的 `PathBuf`，表示 `--relative-to` 选项。
    /// - `relative_base`: 可选的 `PathBuf`，表示 `--relative-base` 选项。
    /// - `can_mode`: `MissingHandling` 枚举，用于指定处理缺失路径的方式。
    /// - `resolve_mode`: `ResolveMode` 枚举，用于指定解析路径的方式。
    ///
    /// # 返回值
    /// 返回一个包含两个 `Option<PathBuf>` 的元组，分别表示处理后的 `relative_to` 和 `relative_base`。
    /// 如果 `relative_to` 不是 `relative_base` 的子路径，则返回 `(None, None)`。
    fn realpath_prepare_relative_options(
        relative_to: &Option<PathBuf>,
        relative_base: &Option<PathBuf>,
        can_mode: MissingHandling,
        resolve_mode: ResolveMode,
    ) -> CTResult<(Option<PathBuf>, Option<PathBuf>)> {
        // 定义一个闭包，用于将相对路径转换为绝对路径，并处理可能的错误。
        let canonicalize_relative_option =
            |relative: &Option<PathBuf>| -> CTResult<Option<PathBuf>> {
                Ok(match relative {
                    None => None,
                    Some(p) => {
                        // 将路径转换为绝对路径，并捕获可能的错误信息。
                        let abs = canonicalize(p, can_mode, resolve_mode)
                            .map_err_context(|| p.maybe_quote().to_string())?;

                        // 如果 `can_mode` 是 `Existing`，则确保路径是一个目录。
                        if can_mode == MissingHandling::Existing && !abs.is_dir() {
                            abs.read_dir()?; // 如果路径不是目录，则抛出错误。
                        }
                        Some(abs)
                    }
                })
            };

        // 对 `relative_to` 和 `relative_base` 进行绝对路径转换。
        let relative_to = canonicalize_relative_option(relative_to)?;
        let relative_base = canonicalize_relative_option(relative_base)?;

        // 检查 `relative_to` 是否是 `relative_base` 的子路径。
        if let (Some(base), Some(to)) = (relative_base.as_deref(), relative_to.as_deref()) {
            if !to.starts_with(base) {
                return Ok((None, None)); // 如果不是子路径，则返回 `(None, None)`。
            }
        }

        // 返回处理后的 `relative_to` 和 `relative_base`。
        Ok((relative_to, relative_base))
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    let mut stdout = std::io::stdout();
    realpath_main(&mut stdout, args)
}

/// 主函数，用于处理实时路径解析
///
/// # Parameters
/// * `args`: 实现了 `ctcore::Args` 特性的类型，通常用于命令行参数的输入
///
/// # Returns
/// * `CTResult<()>`: 一个结果类型，用于处理可能的错误
///
/// # Description
/// 该函数是实时路径解析功能的入口点它接受命令行参数，解析这些参数，并根据参数执行相应的路径解析操作
/// 函数首先尝试从提供的参数中获取匹配信息，然后根据这些匹配信息创建 RealpathFlags 对象，最后调用 realpath_exec 函数执行实际的路径解析操作
pub fn realpath_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 尝试从提供的参数中获取匹配信息，如果失败，则以退出码 1 终止程序
    let matches = ct_app().try_get_matches_from(args).with_exit_code(1)?;

    // 根据匹配信息创建 RealpathFlags 对象，用于指导后续的路径解析操作
    let flags = RealpathFlags::new(matches)?;

    // 执行实时路径解析操作
    realpath_exec(writer, &flags)?;
    Ok(())
}

/// 根据RealpathFlags中的配置解析文件路径
/// 此函数遍历RealpathFlags中指定的文件列表，对每个文件路径进行解析
/// 如果设置了quiet标志，解析过程中不会显示错误信息
///
/// # Parameters
/// - `flags`: &RealpathFlags - 包含要解析的文件路径和解析选项的引用
///
/// # Returns
/// - `CTResult<()>` - 表示操作结果的类型，如果所有路径都成功解析或根据配置不显示错误信息，则返回Ok(())
fn realpath_exec<W: Write>(writer: &mut W, flags: &RealpathFlags) -> CTResult<()> {
    // 遍历需要解析的文件路径列表
    for path in &flags.files {
        // 将 stdout 作为 writer 传入
        let result = realpath_resolve_path(writer, path, flags);

        // 如果未设置quiet标志，则显示解析过程中的错误信息
        if !flags.is_quiet {
            ct_show_if_err!(result.map_err_context(|| path.maybe_quote().to_string()));
        }
    }
    // 所有路径解析操作完成，返回Ok
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = REALPATH_ABOUT;
    let usage_description = ct_format_usage(REALPATH_USAGE);
    let args = vec![
        Arg::new(realpath_flags::REALPATH_QUIET)
            .short('q')
            .long(realpath_flags::REALPATH_QUIET)
            .help("Do not print warnings for invalid paths")
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_STRIP)
            .short('s')
            .long(realpath_flags::REALPATH_STRIP)
            .visible_alias("no-symlinks")
            .help("Only strip '.' and '..' components, but don't resolve symbolic links")
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_ZERO)
            .short('z')
            .long(realpath_flags::REALPATH_ZERO)
            .help("Separate output filenames with \\0 rather than newline")
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_LOGICAL)
            .short('L')
            .long(realpath_flags::REALPATH_LOGICAL)
            .help("resolve '..' components before symlinks")
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_PHYSICAL)
            .short('P')
            .long(realpath_flags::REALPATH_PHYSICAL)
            .overrides_with_all([
                realpath_flags::REALPATH_STRIP,
                realpath_flags::REALPATH_LOGICAL,
            ])
            .help("resolve symlinks as encountered (default)")
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_CANONICALIZE_EXISTING)
            .short('e')
            .long(realpath_flags::REALPATH_CANONICALIZE_EXISTING)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, all components must exist",
            )
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_CANONICALIZE_MISSING)
            .short('m')
            .long(realpath_flags::REALPATH_CANONICALIZE_MISSING)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, without requirements on components existence",
            )
            .action(ArgAction::SetTrue),
        Arg::new(realpath_flags::REALPATH_RELATIVE_TO)
            .long(realpath_flags::REALPATH_RELATIVE_TO)
            .value_name("DIR")
            .value_parser(NonEmptyStringValueParser::new())
            .help("print the resolved path relative to DIR"),
        Arg::new(realpath_flags::REALPATH_RELATIVE_BASE)
            .long(realpath_flags::REALPATH_RELATIVE_BASE)
            .value_name("DIR")
            .value_parser(NonEmptyStringValueParser::new())
            .help("print absolute paths unless paths below DIR"),
        Arg::new(realpath_flags::REALPATH_ARG_FILES)
            .action(ArgAction::Append)
            .required(true)
            .value_parser(NonEmptyStringValueParser::new())
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

/// 将路径解析为绝对形式并打印。
///
/// 如果提供了 `relative_to` 和/或 `relative_base`，则路径将以相对形式打印，
/// 如果 `zero` 为 `true`，则该函数会在路径后打印空字符 (`'\0'`) 而不是换行符 (`'\n'`)。
///
/// # 错误
///
/// 如果在解析符号链接时出现问题，此函数将返回错误。
///
/// # 参数
/// - `p`: 需要解析的路径。
/// - `flags`: 包含解析路径选项的标志。
///
/// # 返回值
/// 返回一个 `Result`，如果路径成功解析并打印，则返回 `Ok`；如果发生错误，则返回 `Err`。
fn realpath_resolve_path<W: Write>(
    writer: &mut W,
    p: &Path,
    flags: &RealpathFlags,
) -> std::io::Result<()> {
    // 将给定路径转换为绝对路径，并解析任何符号链接。
    let abs = canonicalize(p, flags.can_mode, flags.resolve_mode)?;

    // 根据给定的相对选项处理绝对路径。
    let abs = realpath_process_relative(
        abs,
        flags.relative_base.as_deref(),
        flags.relative_to.as_deref(),
    );

    // 打印处理后的路径。
    writer.write_all(abs.as_path().to_string_lossy().as_bytes())?;
    // 根据给定的标志打印行结束字符。
    writer.write_all(&[flags.line_ending.into()])?;
    Ok(())
}

/// 根据以下规则有条件地将绝对路径转换为相对路径：
/// 1. 如果仅提供了 `relative_to`，则结果相对于 `relative_to`
/// 2. 如果仅提供了 `relative_base`，则检查给定的 `path` 是否是 `relative_base` 的后代，
///    如果是，则结果相对于 `relative_base`，否则结果是给定的 `path`
/// 3. 如果同时提供了 `relative_to` 和 `relative_base`，则当 `path` 是 `relative_base` 的后代时，
///    结果相对于 `relative_to`，否则结果是 `path`
fn realpath_process_relative(
    path: PathBuf,                // 输入的路径
    relative_base: Option<&Path>, // 可选的相对基准路径
    relative_to: Option<&Path>,   // 可选的相对目标路径
) -> PathBuf {
    // 根据 `relative_base` 和 `relative_to` 的不同情况处理路径
    match (relative_base, relative_to) {
        // 当 `relative_base` 存在且 `path` 以 `relative_base` 开头时，
        // 尝试将 `path` 相对于 `relative_to` 或 `relative_base`（如果 `relative_to` 不存在）
        (Some(base), _) if path.starts_with(base) => {
            make_path_relative_to(path, relative_to.unwrap_or(base))
        }
        // 当 `relative_to` 存在时，将 `path` 相对于 `relative_to`
        (_, Some(to)) => make_path_relative_to(path, to),
        // 如果上述条件都不满足，返回原始的 `path`
        _ => path,
    }
}

#[derive(Default)]
pub struct Realpath;
impl Tool for Realpath {
    fn name(&self) -> &'static str {
        "realpath"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        // 直接调用原有的 realpath_main 函数
        let mut stdout = std::io::stdout();
        realpath_main(&mut stdout, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::fs::File;
    use tempfile::Builder;

    #[test]
    fn test_tool_implementation() {
        let tool = Realpath::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "realpath");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("realpath"));

        // 测试 execute 方法
        let args = vec![OsString::from("realpath"), OsString::from("--help")];
        let result = tool.execute(&args);
        assert!(result.is_err()); // realpath命令需要参数，所以不带参数应该返回错误
    }

    mod realpath_flags_tests {
        use super::*;

        fn create_test_matches(args: &[&str]) -> ArgMatches {
            ct_app().try_get_matches_from(args).unwrap()
        }

        #[test]
        fn test_flags_new_basic() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();

            assert!(!flags.is_quiet);
            assert_eq!(flags.line_ending, CtLineEnding::Newline);
            assert_eq!(flags.can_mode, MissingHandling::Normal);
            assert_eq!(flags.resolve_mode, ResolveMode::Physical);
            assert!(flags.relative_to.is_none());
            assert!(flags.relative_base.is_none());
            assert_eq!(flags.files.len(), 1);
        }

        #[test]
        fn test_flags_with_zero_option() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "-z", "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.line_ending, CtLineEnding::Nul);
        }

        #[test]
        fn test_flags_with_quiet_option() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "-q", "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert!(flags.is_quiet);
        }

        #[test]
        fn test_flags_with_canonicalize_existing() {
            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "--canonicalize-existing",
                "test.txt",
            ]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.can_mode, MissingHandling::Existing);
        }

        #[test]
        fn test_flags_with_relative_options() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let base_dir = temp_dir.path().join("base");
            let dir = temp_dir.path().join("dir");

            // 创建目录
            std::fs::create_dir(&base_dir).unwrap();
            std::fs::create_dir(&dir).unwrap();

            // 创建一个测试文件在 base_dir 下
            let test_file = base_dir.join("test.txt");
            File::create(&test_file).unwrap();

            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                &format!("--relative-to={}", base_dir.display()),
                &format!("--relative-base={}", base_dir.display()), // 使用相同的 base_dir
                test_file.to_str().unwrap(),
            ]);

            let flags = RealpathFlags::new(matches).unwrap();

            // 验证 relative_to 和 relative_base 都被正确设置
            assert!(flags.relative_to.is_some());
            assert!(flags.relative_base.is_some());

            // 额外验证路径是否正确
            if let Some(relative_to) = &flags.relative_to {
                assert_eq!(
                    relative_to.canonicalize().unwrap(),
                    base_dir.canonicalize().unwrap()
                );
            }
            if let Some(relative_base) = &flags.relative_base {
                assert_eq!(
                    relative_base.canonicalize().unwrap(),
                    base_dir.canonicalize().unwrap()
                );
            }
        }

        #[test]
        fn test_flags_with_strip_option() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "--strip", "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.resolve_mode, ResolveMode::None);
        }

        #[test]
        fn test_flags_with_logical_option() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "--logical", "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.resolve_mode, ResolveMode::Logical);
        }

        #[test]
        fn test_flags_with_physical_option() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "--physical", "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.resolve_mode, ResolveMode::Physical);
        }

        #[test]
        fn test_flags_with_multiple_files() {
            let matches = create_test_matches(&[
                ctcore::ct_util_name(),
                "file1.txt",
                "file2.txt",
                "file3.txt",
            ]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.files.len(), 3);
        }
    }

    mod realpath_prepare_relative_options_tests {
        use super::*;

        fn setup_test_dir() -> (tempfile::TempDir, PathBuf) {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let test_dir = temp_dir.path().join("test_dir");
            std::fs::create_dir(&test_dir).unwrap();
            (temp_dir, test_dir)
        }

        #[test]
        fn test_prepare_relative_options_none() {
            let result = RealpathFlags::realpath_prepare_relative_options(
                &None,
                &None,
                MissingHandling::Normal,
                ResolveMode::Physical,
            )
            .unwrap();
            assert_eq!(result, (None, None));
        }

        #[test]
        fn test_prepare_relative_options_with_existing_dir() {
            let (_temp_dir, test_dir) = setup_test_dir();
            let result = RealpathFlags::realpath_prepare_relative_options(
                &Some(test_dir.clone()),
                &None,
                MissingHandling::Existing,
                ResolveMode::Physical,
            )
            .unwrap();
            assert!(result.0.is_some());
        }

        #[test]
        fn test_prepare_relative_options_with_missing_dir() {
            let result = RealpathFlags::realpath_prepare_relative_options(
                &Some(PathBuf::from("/nonexistent")),
                &None,
                MissingHandling::Missing,
                ResolveMode::Physical,
            )
            .unwrap();
            assert!(result.0.is_some());
        }

        #[test]
        fn test_prepare_relative_options_with_invalid_dir() {
            let result = RealpathFlags::realpath_prepare_relative_options(
                &Some(PathBuf::from("/nonexistent")),
                &None,
                MissingHandling::Existing,
                ResolveMode::Physical,
            );
            assert!(result.is_err());
        }

        #[test]
        fn test_prepare_relative_options_with_both_dirs() {
            let (_temp_dir, test_dir) = setup_test_dir();
            let sub_dir = test_dir.join("subdir");
            std::fs::create_dir(&sub_dir).unwrap();

            let result = RealpathFlags::realpath_prepare_relative_options(
                &Some(sub_dir.clone()),
                &Some(test_dir.clone()),
                MissingHandling::Existing,
                ResolveMode::Physical,
            )
            .unwrap();
            assert!(result.0.is_some());
            assert!(result.1.is_some());
        }

        #[test]
        fn test_prepare_relative_options_with_non_subpath() {
            let (_temp_dir1, dir1) = setup_test_dir();
            let (_temp_dir2, dir2) = setup_test_dir();

            let result = RealpathFlags::realpath_prepare_relative_options(
                &Some(dir1),
                &Some(dir2),
                MissingHandling::Existing,
                ResolveMode::Physical,
            )
            .unwrap();
            assert_eq!(result, (None, None));
        }
    }

    mod realpath_exec_tests {
        use super::*;

        fn setup_test_file() -> (tempfile::TempDir, PathBuf) {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let test_file = temp_dir.path().join("test.txt");
            File::create(&test_file).unwrap();
            (temp_dir, test_file)
        }

        #[test]
        fn test_exec_basic() {
            let (_temp_dir, test_file) = setup_test_file();
            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![test_file],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
        }

        #[test]
        fn test_exec_quiet_mode() {
            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: true,
                relative_to: None,
                relative_base: None,
                files: vec![PathBuf::from("/nonexistent")],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
        }

        #[test]
        fn test_exec_multiple_files() {
            let (temp_dir, test_file1) = setup_test_file();
            let test_file2 = temp_dir.path().join("test2.txt");
            File::create(&test_file2).unwrap();

            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![test_file1, test_file2],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
        }

        #[test]
        fn test_exec_with_relative_paths() {
            let (_temp_dir, test_file) = setup_test_file();
            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: Some(test_file.parent().unwrap().to_path_buf()),
                relative_base: None,
                files: vec![test_file],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
            assert!(String::from_utf8_lossy(&output).ends_with('\n'));
        }

        #[test]
        fn test_exec_with_zero_terminator() {
            let (_temp_dir, test_file) = setup_test_file();
            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![test_file],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Nul,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
            assert_eq!(output.last(), Some(&0));
        }

        #[test]
        fn test_exec_with_missing_handling() {
            let mut output = Vec::new();
            let nonexistent = PathBuf::from("/nonexistent/path");
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![nonexistent],
                can_mode: MissingHandling::Missing,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
        }
    }

    mod realpath_main_tests {
        use super::*;

        #[test]
        fn test_main_basic() {
            let (_temp_dir, test_file) = setup_test_file();
            let mut output = Vec::new();
            let args = vec![ctcore::ct_util_name(), test_file.to_str().unwrap()];
            assert!(realpath_main(&mut output, args.iter().map(|s| OsString::from(s))).is_ok());
        }

        #[test]
        fn test_main_invalid_args() {
            let mut output = Vec::new();
            let args = vec![ctcore::ct_util_name(), "--invalid-flag"];
            assert!(realpath_main(&mut output, args.iter().map(|s| OsString::from(s))).is_err());
        }

        #[test]
        fn test_main_help() {
            let mut output = Vec::new();
            let args = vec![ctcore::ct_util_name(), "--help"];
            assert!(realpath_main(&mut output, args.iter().map(|s| OsString::from(s))).is_err());
        }

        fn setup_test_file() -> (tempfile::TempDir, PathBuf) {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let test_file = temp_dir.path().join("test.txt");
            File::create(&test_file).unwrap();
            (temp_dir, test_file)
        }
    }

    mod ct_app_tests {
        use super::*;

        #[test]
        fn test_app_version() {
            let args = vec![ctcore::ct_util_name(), "--version"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayVersion
            );
        }

        #[test]
        fn test_app_help() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                clap::error::ErrorKind::DisplayHelp
            );
        }

        #[test]
        fn test_app_missing_required_args() {
            let args = vec![ctcore::ct_util_name()];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
        }

        #[test]
        fn test_app_valid_args() {
            let args = vec![ctcore::ct_util_name(), "test.txt"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
