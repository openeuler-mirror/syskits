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

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, builder::OsStringValueParser, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_fs::make_path_relative_to;
use ctcore::ct_posix::GnuGetoptCommandExt;
use ctcore::{
    Tool,
    ct_display::Quotable,
    ct_error::{CTError, CTResult, CTsageError, FromIo, UClapError, set_ct_exit_code},
    ct_fs::{MissingHandling, ResolveMode, canonicalize},
    ct_line_ending::CtLineEnding,
    ct_show,
};
use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::{Path, PathBuf},
};
use sys_locale::get_locale;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealpathResolutionMode {
    None,
    Physical,
    Logical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealpathMissingHandling {
    Normal,
    Existing,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealpathSemanticRow {
    pub input: String,
    pub resolved_path: String,
    pub output_path: String,
    pub resolution_mode: RealpathResolutionMode,
    pub missing_handling: RealpathMissingHandling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealpathSemantic {
    pub rows: Vec<RealpathSemanticRow>,
    pub classic_text: String,
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
            .get_many::<OsString>(realpath_flags::REALPATH_ARG_FILES)
            .unwrap_or_default()
            .map(PathBuf::from)
            .collect();
        if files.is_empty() {
            return Err(CTsageError::new(1, "missing operand"));
        }

        // 提取是否使用零字符结尾的标志，并据此确定行尾符类型
        let is_zero = matches.get_count(realpath_flags::REALPATH_ZERO) > 0;
        let line_ending = CtLineEnding::from_zero_flag(is_zero);
        // 提取是否进行现有路径规范化的标志
        let is_canonicalize_existing =
            matches.get_count(realpath_flags::REALPATH_CANONICALIZE_EXISTING) > 0;
        // 提取是否进行缺失路径规范化的标志
        let is_canonicalize_missing =
            matches.get_count(realpath_flags::REALPATH_CANONICALIZE_MISSING) > 0;
        // 根据上述标志确定路径处理模式
        let can_mode = if is_canonicalize_existing {
            MissingHandling::Existing
        } else if is_canonicalize_missing {
            MissingHandling::Missing
        } else {
            MissingHandling::Normal
        };

        // 提取是否进行符号链接剥离的标志
        let is_strip = matches.get_count(realpath_flags::REALPATH_STRIP) > 0;
        // 提取是否进行逻辑解析的标志
        let is_logical = matches.get_count(realpath_flags::REALPATH_LOGICAL) > 0;
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
            .get_many::<OsString>(realpath_flags::REALPATH_RELATIVE_TO)
            .and_then(|mut values| values.next_back())
            .cloned()
            .map(PathBuf::from);
        // 提取相对路径基础的参数
        let relative_base = matches
            .get_many::<OsString>(realpath_flags::REALPATH_RELATIVE_BASE)
            .and_then(|mut values| values.next_back())
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
        let is_quiet = matches.get_count(realpath_flags::REALPATH_QUIET) > 0;
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
                        let abs = realpath_canonicalize(p, can_mode, resolve_mode)
                            .map_err_context(|| p.maybe_quote().to_string())?;

                        // 如果 `can_mode` 是 `Existing`，则确保路径是一个目录。
                        if can_mode == MissingHandling::Existing && !abs.is_dir() {
                            abs.read_dir()
                                .map_err_context(|| p.maybe_quote().to_string())?;
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
    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 尝试从提供的参数中获取匹配信息，如果失败，则以退出码 1 终止程序
    let matches = ct_app().try_get_matches_from(args).with_exit_code(1)?;

    // 根据匹配信息创建 RealpathFlags 对象，用于指导后续的路径解析操作
    let flags = RealpathFlags::new(matches)?;

    // 执行实时路径解析操作
    realpath_exec(writer, &flags)?;
    Ok(())
}

pub fn realpath_native_semantic(args: impl ctcore::Args) -> CTResult<RealpathSemantic> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    let matches = ct_app().try_get_matches_from(args).with_exit_code(1)?;
    let flags = RealpathFlags::new(matches)?;

    let mut rows = Vec::with_capacity(flags.files.len());
    let mut classic_text = Vec::new();

    for path in &flags.files {
        let resolved = realpath_canonicalize(path, flags.can_mode, flags.resolve_mode)
            .map_err_context(|| path.maybe_quote().to_string())?;
        let output = realpath_process_relative(
            resolved.clone(),
            flags.relative_base.as_deref(),
            flags.relative_to.as_deref(),
        );

        classic_text.extend_from_slice(output.as_path().to_string_lossy().as_bytes());
        classic_text.push(flags.line_ending.into());

        rows.push(RealpathSemanticRow {
            input: path.display().to_string(),
            resolved_path: resolved.display().to_string(),
            output_path: output.display().to_string(),
            resolution_mode: semantic_resolution_mode(flags.resolve_mode),
            missing_handling: semantic_missing_handling(flags.can_mode),
        });
    }

    Ok(RealpathSemantic {
        rows,
        classic_text: String::from_utf8(classic_text)
            .expect("realpath classic output should be valid utf-8"),
    })
}

fn semantic_resolution_mode(mode: ResolveMode) -> RealpathResolutionMode {
    match mode {
        ResolveMode::None => RealpathResolutionMode::None,
        ResolveMode::Physical => RealpathResolutionMode::Physical,
        ResolveMode::Logical => RealpathResolutionMode::Logical,
    }
}

fn semantic_missing_handling(mode: MissingHandling) -> RealpathMissingHandling {
    match mode {
        MissingHandling::Normal => RealpathMissingHandling::Normal,
        MissingHandling::Existing => RealpathMissingHandling::Existing,
        MissingHandling::Missing => RealpathMissingHandling::Missing,
    }
}

fn realpath_canonicalize(
    path: &Path,
    missing_handling: MissingHandling,
    resolve_mode: ResolveMode,
) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No such file or directory",
        ));
    }
    // GNU first processes the input in CAN_NOLINKS mode.  This validates the
    // directory components before a later /./ or /.. can discard them.
    validate_path_components(path, missing_handling)?;

    match resolve_mode {
        ResolveMode::None => canonicalize(path, MissingHandling::Missing, ResolveMode::None),
        ResolveMode::Physical => canonicalize(path, missing_handling, ResolveMode::Physical),
        ResolveMode::Logical => {
            let lexical_path = canonicalize(path, MissingHandling::Missing, ResolveMode::None)?;
            canonicalize(lexical_path, missing_handling, ResolveMode::Physical)
        }
    }
}

#[cfg(unix)]
fn validate_path_components(path: &Path, missing_handling: MissingHandling) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    if missing_handling == MissingHandling::Missing {
        return Ok(());
    }

    if missing_handling == MissingHandling::Existing {
        return std::fs::metadata(path).map(|_| ());
    }

    let bytes = path.as_os_str().as_bytes();
    let mut prefix = if bytes.starts_with(b"/") {
        PathBuf::from("/")
    } else {
        std::env::current_dir()?
    };
    let mut component_start = 0;

    while component_start < bytes.len() {
        while component_start < bytes.len() && bytes[component_start] == b'/' {
            component_start += 1;
        }
        if component_start == bytes.len() {
            break;
        }

        let component_end = bytes[component_start..]
            .iter()
            .position(|byte| *byte == b'/')
            .map_or(bytes.len(), |offset| component_start + offset);
        let component = &bytes[component_start..component_end];
        let suffix = &bytes[component_end..];

        match component {
            b"." => {}
            b".." => {
                prefix.pop();
            }
            _ => {
                prefix.push(OsStr::from_bytes(component));
                if strip_suffix_requires_directory(suffix) {
                    match std::fs::metadata(prefix.join(".")) {
                        Ok(_) => {}
                        Err(error)
                            if error.kind() == io::ErrorKind::NotFound
                                && suffix.iter().all(|byte| *byte == b'/') => {}
                        Err(error) => return Err(error),
                    }
                } else if suffix.is_empty() {
                    if let Err(error) = std::fs::metadata(&prefix) {
                        if error.kind() != io::ErrorKind::NotFound {
                            return Err(error);
                        }
                    }
                }
            }
        }
        component_start = component_end;
    }

    Ok(())
}

#[cfg(unix)]
fn strip_suffix_requires_directory(suffix: &[u8]) -> bool {
    let mut index = 0;
    while index < suffix.len() && suffix[index] == b'/' {
        while index < suffix.len() && suffix[index] == b'/' {
            index += 1;
        }
        if index == suffix.len() {
            return true;
        }
        if suffix[index] != b'.' {
            return false;
        }
        index += 1;
        if index == suffix.len()
            || (suffix[index] == b'.' && (index + 1 == suffix.len() || suffix[index + 1] == b'/'))
        {
            return true;
        }
    }
    false
}

#[cfg(not(unix))]
fn validate_path_components(path: &Path, missing_handling: MissingHandling) -> io::Result<()> {
    match missing_handling {
        MissingHandling::Existing => std::fs::metadata(path).map(|_| ()),
        MissingHandling::Normal => match std::fs::metadata(path) {
            Ok(_)
            | Err(io::Error {
                kind: io::ErrorKind::NotFound,
                ..
            }) => Ok(()),
            Err(error) => Err(error),
        },
        MissingHandling::Missing => Ok(()),
    }
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
    for path in &flags.files {
        match realpath_resolve_output_path(path, flags) {
            Ok(output_path) => {
                write_resolved_path(writer, &output_path, flags.line_ending)
                    .map_err_context(|| String::from("write error"))?;
            }
            Err(error) => {
                let error = error.map_err_context(|| path.maybe_quote().to_string());
                if flags.is_quiet {
                    set_ct_exit_code(error.code());
                } else {
                    ct_show!(error);
                }
            }
        }
    }
    Ok(())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("realpath.about");
    let usage_description = t!("realpath.usage");
    let args = vec![
        Arg::new(realpath_flags::REALPATH_QUIET)
            .short('q')
            .long(realpath_flags::REALPATH_QUIET)
            .help(t!("realpath.clap.realpath_quiet"))
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_STRIP)
            .short('s')
            .long(realpath_flags::REALPATH_STRIP)
            .overrides_with_all([
                realpath_flags::REALPATH_PHYSICAL,
                realpath_flags::REALPATH_LOGICAL,
            ])
            .visible_alias("no-symlinks")
            .help(t!("realpath.clap.realpath_strip"))
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_ZERO)
            .short('z')
            .long(realpath_flags::REALPATH_ZERO)
            .help(t!("realpath.clap.realpath_zero"))
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_LOGICAL)
            .short('L')
            .long(realpath_flags::REALPATH_LOGICAL)
            .overrides_with_all([
                realpath_flags::REALPATH_PHYSICAL,
                realpath_flags::REALPATH_STRIP,
            ])
            .help(t!("realpath.clap.realpath_logical"))
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_PHYSICAL)
            .short('P')
            .long(realpath_flags::REALPATH_PHYSICAL)
            .overrides_with_all([
                realpath_flags::REALPATH_STRIP,
                realpath_flags::REALPATH_LOGICAL,
            ])
            .help(t!("realpath.clap.realpath_physical"))
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_CANONICALIZE_EXISTING)
            .short('e')
            .long(realpath_flags::REALPATH_CANONICALIZE_EXISTING)
            .overrides_with(realpath_flags::REALPATH_CANONICALIZE_MISSING)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, all components must exist",
            )
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_CANONICALIZE_MISSING)
            .short('m')
            .long(realpath_flags::REALPATH_CANONICALIZE_MISSING)
            .overrides_with(realpath_flags::REALPATH_CANONICALIZE_EXISTING)
            .help(
                "canonicalize by following every symlink in every component of the \
                     given name recursively, without requirements on components existence",
            )
            .action(ArgAction::Count),
        Arg::new(realpath_flags::REALPATH_RELATIVE_TO)
            .long(realpath_flags::REALPATH_RELATIVE_TO)
            .value_name("DIR")
            .value_parser(OsStringValueParser::new())
            .help("print the resolved path relative to DIR")
            .action(ArgAction::Append),
        Arg::new(realpath_flags::REALPATH_RELATIVE_BASE)
            .long(realpath_flags::REALPATH_RELATIVE_BASE)
            .value_name("DIR")
            .value_parser(OsStringValueParser::new())
            .help("print absolute paths unless paths below DIR")
            .action(ArgAction::Append),
        Arg::new(realpath_flags::REALPATH_ARG_FILES)
            .action(ArgAction::Append)
            .value_parser(OsStringValueParser::new())
            .value_hint(clap::ValueHint::AnyPath),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
        .gnu_getopt()
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
#[cfg(test)]
fn realpath_resolve_path<W: Write>(
    writer: &mut W,
    p: &Path,
    flags: &RealpathFlags,
) -> std::io::Result<()> {
    let output_path = realpath_resolve_output_path(p, flags)?;
    write_resolved_path(writer, &output_path, flags.line_ending)
}

fn realpath_resolve_output_path(p: &Path, flags: &RealpathFlags) -> std::io::Result<PathBuf> {
    let absolute_path = realpath_canonicalize(p, flags.can_mode, flags.resolve_mode)?;
    Ok(realpath_process_relative(
        absolute_path,
        flags.relative_base.as_deref(),
        flags.relative_to.as_deref(),
    ))
}

fn write_resolved_path<W: Write>(
    writer: &mut W,
    path: &Path,
    line_ending: CtLineEnding,
) -> std::io::Result<()> {
    write_path_bytes(writer, path.as_os_str())?;
    writer.write_all(&[line_ending.into()])
}

fn write_path_bytes<W: Write>(writer: &mut W, path: &OsStr) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        writer.write_all(path.as_bytes())
    }
    #[cfg(not(unix))]
    {
        writer.write_all(path.to_string_lossy().as_bytes())
    }
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
        // 提供了 relative_base 且路径在其下 → 相对于 relative_to（或 base）计算
        (Some(base), to) if path.starts_with(base) => {
            make_path_relative_to(path, to.unwrap_or(base))
        }
        // 提供了 relative_base 但路径不在其下 → 返回绝对路径（不做转换）
        (Some(_), _) => path,
        // 没有 relative_base 但有 relative_to → 相对于 relative_to 计算
        (None, Some(to)) => make_path_relative_to(path, to),
        // 两者都没有 → 返回绝对路径
        (None, None) => path,
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
    use ctcore::ct_error::{get_ct_exit_code, set_ct_exit_code};
    use std::ffi::OsString;
    use std::fs::File;
    use std::sync::Mutex;
    use tempfile::Builder;

    static POSIXLY_CORRECT_LOCK: Mutex<()> = Mutex::new(());
    static EXIT_CODE_TEST_LOCK: Mutex<()> = Mutex::new(());
    static CURRENT_DIRECTORY_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(unix)]
    struct FullWriter;

    #[cfg(unix)]
    impl Write for FullWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from_raw_os_error(ctcore::libc::ENOSPC))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_tool_implementation() {
        let tool = Realpath;

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

    #[test]
    fn test_realpath_main_reports_missing_operand() {
        let mut output = Vec::new();
        let error = realpath_main(
            &mut output,
            std::iter::once(OsString::from(ctcore::ct_util_name())),
        )
        .unwrap_err();

        assert_eq!(error.code(), 1);
        assert_eq!(error.to_string(), "missing operand");
        assert!(output.is_empty());
    }

    mod realpath_flags_tests {
        use super::*;
        #[cfg(unix)]
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

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
        fn test_repeated_boolean_options_are_accepted() {
            let cases = [
                (
                    vec!["-e", "-e", "."],
                    MissingHandling::Existing,
                    ResolveMode::Physical,
                ),
                (
                    vec!["-m", "-m", "."],
                    MissingHandling::Missing,
                    ResolveMode::Physical,
                ),
                (
                    vec!["-L", "-L", "."],
                    MissingHandling::Normal,
                    ResolveMode::Logical,
                ),
                (
                    vec!["-P", "-P", "."],
                    MissingHandling::Normal,
                    ResolveMode::Physical,
                ),
                (
                    vec!["-s", "-s", "."],
                    MissingHandling::Normal,
                    ResolveMode::None,
                ),
                (
                    vec!["--no-symlinks", "-s", "."],
                    MissingHandling::Normal,
                    ResolveMode::None,
                ),
            ];

            for (options, expected_can_mode, expected_resolve_mode) in cases {
                let mut args = vec![ctcore::ct_util_name()];
                args.extend(options);
                let matches = create_test_matches(&args);
                let flags = RealpathFlags::new(matches).unwrap();

                assert_eq!(flags.can_mode, expected_can_mode);
                assert_eq!(flags.resolve_mode, expected_resolve_mode);
            }

            let quiet_matches = create_test_matches(&[ctcore::ct_util_name(), "-q", "-q", "."]);
            assert!(RealpathFlags::new(quiet_matches).unwrap().is_quiet);

            let zero_matches = create_test_matches(&[ctcore::ct_util_name(), "-z", "-z", "."]);
            assert_eq!(
                RealpathFlags::new(zero_matches).unwrap().line_ending,
                CtLineEnding::Nul
            );
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
        fn test_existing_relative_to_file_reports_option_value() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let file = temp_dir.path().join("file");
            File::create(&file).unwrap();
            let args = [
                OsString::from(ctcore::ct_util_name()),
                OsString::from("-e"),
                OsString::from("--relative-to"),
                file.clone().into_os_string(),
                OsString::from("."),
            ];
            let matches = ct_app().try_get_matches_from(args).unwrap();

            let error = match RealpathFlags::new(matches) {
                Ok(_) => panic!("an existing relative-to value must be a directory"),
                Err(error) => error,
            };

            assert_eq!(
                error.to_string(),
                format!("{}: Not a directory", file.display())
            );
        }

        #[test]
        fn test_last_canonicalize_option_wins() {
            let missing_last =
                create_test_matches(&[ctcore::ct_util_name(), "-e", "-m", "missing"]);
            assert_eq!(
                RealpathFlags::new(missing_last).unwrap().can_mode,
                MissingHandling::Missing
            );

            let existing_last =
                create_test_matches(&[ctcore::ct_util_name(), "-m", "-e", "missing"]);
            assert_eq!(
                RealpathFlags::new(existing_last).unwrap().can_mode,
                MissingHandling::Existing
            );
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
        fn test_last_relative_options_win() {
            let relative_to = ct_app()
                .try_get_matches_from([
                    ctcore::ct_util_name(),
                    "--relative-to=/",
                    "--relative-to=/tmp",
                    "path",
                ])
                .unwrap();
            assert_eq!(
                RealpathFlags::new(relative_to).unwrap().relative_to,
                Some(PathBuf::from("/tmp"))
            );

            let relative_base = ct_app()
                .try_get_matches_from([
                    ctcore::ct_util_name(),
                    "--relative-base=/",
                    "--relative-base=/tmp",
                    "path",
                ])
                .unwrap();
            assert_eq!(
                RealpathFlags::new(relative_base).unwrap().relative_base,
                Some(PathBuf::from("/tmp"))
            );
        }

        #[test]
        fn test_posixly_correct_stops_option_parsing_at_first_file() {
            let _guard = POSIXLY_CORRECT_LOCK.lock().unwrap();
            let previous = std::env::var_os("POSIXLY_CORRECT");
            unsafe { std::env::set_var("POSIXLY_CORRECT", "1") };
            let matches = ct_app()
                .try_get_matches_from([ctcore::ct_util_name(), "a", "-m", "missing"])
                .unwrap();
            match previous {
                Some(value) => unsafe { std::env::set_var("POSIXLY_CORRECT", value) },
                None => unsafe { std::env::remove_var("POSIXLY_CORRECT") },
            }

            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.can_mode, MissingHandling::Normal);
            assert_eq!(
                flags.files,
                [
                    PathBuf::from("a"),
                    PathBuf::from("-m"),
                    PathBuf::from("missing")
                ]
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_flags_preserve_non_utf8_path_bytes() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let base = temp_dir
                .path()
                .join(OsString::from_vec(b"base-\xff".to_vec()));
            std::fs::create_dir(&base).unwrap();
            let input = base.join(OsString::from_vec(b"child-\xff".to_vec()));
            File::create(&input).unwrap();

            let mut relative_to = b"--relative-to=".to_vec();
            relative_to.extend_from_slice(base.as_os_str().as_bytes());
            let matches = ct_app()
                .try_get_matches_from([
                    OsString::from(ctcore::ct_util_name()),
                    OsString::from_vec(relative_to),
                    input.as_os_str().to_os_string(),
                ])
                .unwrap();
            let flags = RealpathFlags::new(matches).unwrap();

            assert_eq!(flags.files, vec![input]);
            assert_eq!(
                flags.relative_to.unwrap().as_os_str().as_bytes(),
                base.as_os_str().as_bytes()
            );
        }

        #[cfg(unix)]
        #[test]
        fn test_resolve_path_preserves_non_utf8_output_bytes() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let base = temp_dir
                .path()
                .join(OsString::from_vec(b"base-\xff".to_vec()));
            std::fs::create_dir(&base).unwrap();
            let input = base.join(OsString::from_vec(b"child-\xff".to_vec()));
            File::create(&input).unwrap();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: Some(base),
                relative_base: None,
                files: vec![input.clone()],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };
            let mut output = Vec::new();

            realpath_resolve_path(&mut output, &input, &flags).unwrap();

            assert_eq!(output, b"child-\xff\n");
        }

        #[test]
        fn test_flags_with_strip_option() {
            let matches = create_test_matches(&[ctcore::ct_util_name(), "--strip", "test.txt"]);
            let flags = RealpathFlags::new(matches).unwrap();
            assert_eq!(flags.resolve_mode, ResolveMode::None);
            assert_eq!(flags.can_mode, MissingHandling::Normal);
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
        fn test_last_resolution_mode_option_wins() {
            let logical_last = create_test_matches(&[ctcore::ct_util_name(), "-s", "-L", "path"]);
            assert_eq!(
                RealpathFlags::new(logical_last).unwrap().resolve_mode,
                ResolveMode::Logical
            );

            let strip_last = create_test_matches(&[ctcore::ct_util_name(), "-L", "-s", "path"]);
            assert_eq!(
                RealpathFlags::new(strip_last).unwrap().resolve_mode,
                ResolveMode::None
            );

            let physical_last = create_test_matches(&[ctcore::ct_util_name(), "-L", "-P", "path"]);
            assert_eq!(
                RealpathFlags::new(physical_last).unwrap().resolve_mode,
                ResolveMode::Physical
            );
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
        fn test_prepare_relative_options_rejects_empty_dir() {
            let result = RealpathFlags::realpath_prepare_relative_options(
                &Some(PathBuf::new()),
                &None,
                MissingHandling::Normal,
                ResolveMode::Physical,
            );
            assert!(result.is_err());
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
        #[cfg(unix)]
        use ctcore::libc;
        #[cfg(unix)]
        use std::os::unix::fs::symlink;

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
        fn test_exec_quiet_mode_sets_failure_exit_code() {
            let _guard = EXIT_CODE_TEST_LOCK.lock().unwrap();
            set_ct_exit_code(0);
            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: true,
                relative_to: None,
                relative_base: None,
                files: vec![PathBuf::from("/definitely-not-a-realpath-test-file")],
                can_mode: MissingHandling::Existing,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            assert!(realpath_exec(&mut output, &flags).is_ok());
            assert_eq!(get_ct_exit_code(), 1);
            set_ct_exit_code(0);
        }

        #[test]
        fn test_resolve_empty_path_returns_not_found() {
            let mut output = Vec::new();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![PathBuf::new()],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            let error = realpath_resolve_path(&mut output, Path::new(""), &flags).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            assert!(output.is_empty());
        }

        #[cfg(unix)]
        #[test]
        fn test_resolve_strip_existing_rejects_dangling_symlink() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let dangling = temp_dir.path().join("dangling");
            symlink("missing", &dangling).unwrap();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![dangling.clone()],
                can_mode: MissingHandling::Existing,
                resolve_mode: ResolveMode::None,
                line_ending: CtLineEnding::Newline,
            };

            let error = realpath_resolve_path(&mut Vec::new(), &dangling, &flags).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        }

        #[cfg(unix)]
        #[test]
        fn test_resolve_strip_normal_rejects_symlink_loop() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let first = temp_dir.path().join("first");
            let second = temp_dir.path().join("second");
            symlink("second", &first).unwrap();
            symlink("first", &second).unwrap();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![first.clone()],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::None,
                line_ending: CtLineEnding::Newline,
            };

            let error = realpath_resolve_path(&mut Vec::new(), &first, &flags).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
        }

        #[cfg(unix)]
        #[test]
        fn test_resolve_missing_keeps_symlink_loop_literal() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let first = temp_dir.path().join("first");
            let second = temp_dir.path().join("second");
            symlink("second", &first).unwrap();
            symlink("first", &second).unwrap();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![first.clone()],
                can_mode: MissingHandling::Missing,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };
            let mut output = Vec::new();

            realpath_resolve_path(&mut output, &first, &flags).unwrap();

            assert_eq!(output, format!("{}\n", first.display()).as_bytes());
        }

        #[cfg(unix)]
        #[test]
        fn test_resolve_missing_keeps_suffix_after_symlink_loop() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let first = temp_dir.path().join("first");
            let second = temp_dir.path().join("second");
            symlink("second", &first).unwrap();
            symlink("first", &second).unwrap();
            let input = first.join("child");
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![input.clone()],
                can_mode: MissingHandling::Missing,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };
            let mut output = Vec::new();

            realpath_resolve_path(&mut output, &input, &flags).unwrap();

            assert_eq!(output, format!("{}\n", input.display()).as_bytes());
        }

        #[cfg(unix)]
        #[test]
        fn test_resolve_strip_normal_keeps_broken_intermediate_link_literal() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let directory = temp_dir.path().join("directory");
            std::fs::create_dir(&directory).unwrap();
            let link = directory.join("link");
            symlink("missing", &link).unwrap();
            let input = link.join("file");
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![input.clone()],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::None,
                line_ending: CtLineEnding::Newline,
            };
            let mut output = Vec::new();

            realpath_resolve_path(&mut output, &input, &flags).unwrap();

            assert_eq!(output, format!("{}\n", input.display()).as_bytes());
        }

        #[test]
        fn test_resolve_strip_normal_checks_parent_before_dotdot() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let file = temp_dir.path().join("file");
            File::create(&file).unwrap();
            let input = file.join("..").join("missing");
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![input.clone()],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::None,
                line_ending: CtLineEnding::Newline,
            };

            let error = realpath_resolve_path(&mut Vec::new(), &input, &flags).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
        }

        #[test]
        fn test_resolve_checks_directory_before_dotdot() {
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let file = temp_dir.path().join("file");
            File::create(&file).unwrap();
            let input = file.join("..");

            for resolve_mode in [ResolveMode::Physical, ResolveMode::Logical] {
                for can_mode in [MissingHandling::Normal, MissingHandling::Existing] {
                    let flags = RealpathFlags {
                        is_quiet: false,
                        relative_to: None,
                        relative_base: None,
                        files: vec![input.clone()],
                        can_mode,
                        resolve_mode,
                        line_ending: CtLineEnding::Newline,
                    };

                    let error = realpath_resolve_path(&mut Vec::new(), &input, &flags)
                        .expect_err("a non-directory cannot be followed by /..");

                    assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
                }
            }
        }

        #[test]
        fn test_resolve_checks_leading_parent_directory_component() {
            let _guard = CURRENT_DIRECTORY_LOCK.lock().unwrap();
            let temp_dir = Builder::new().prefix("realpath_test").tempdir().unwrap();
            let child = temp_dir.path().join("child");
            let parent_file = temp_dir.path().join("file");
            std::fs::create_dir(&child).unwrap();
            std::fs::create_dir(child.join("file")).unwrap();
            File::create(&parent_file).unwrap();
            let original_directory = std::env::current_dir().unwrap();
            std::env::set_current_dir(&child).unwrap();
            let flags = RealpathFlags {
                is_quiet: false,
                relative_to: None,
                relative_base: None,
                files: vec![PathBuf::from("../file/..")],
                can_mode: MissingHandling::Normal,
                resolve_mode: ResolveMode::Physical,
                line_ending: CtLineEnding::Newline,
            };

            let result = realpath_resolve_path(&mut Vec::new(), Path::new("../file/.."), &flags);
            std::env::set_current_dir(original_directory).unwrap();
            let error = result.expect_err("the parent file cannot be traversed as a directory");

            assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
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

        #[cfg(unix)]
        #[test]
        fn test_main_reports_write_error_without_path_context() {
            let mut writer = FullWriter;
            let args = [OsString::from(ctcore::ct_util_name()), OsString::from("/")];

            let error = realpath_main(&mut writer, args.into_iter()).unwrap_err();

            assert_eq!(error.to_string(), "write error: No space left on device");
        }

        #[test]
        fn test_main_basic() {
            let (_temp_dir, test_file) = setup_test_file();
            let mut output = Vec::new();
            let args = [ctcore::ct_util_name(), test_file.to_str().unwrap()];
            assert!(realpath_main(&mut output, args.iter().map(OsString::from)).is_ok());
        }

        #[test]
        fn test_main_invalid_args() {
            let mut output = Vec::new();
            let args = [ctcore::ct_util_name(), "--invalid-flag"];
            assert!(realpath_main(&mut output, args.iter().map(OsString::from)).is_err());
        }

        #[test]
        fn test_main_help() {
            let mut output = Vec::new();
            let args = [ctcore::ct_util_name(), "--help"];
            assert!(realpath_main(&mut output, args.iter().map(OsString::from)).is_err());
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
        fn test_app_defers_missing_operand_diagnostic_to_flags() {
            let args = vec![ctcore::ct_util_name()];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let error = match RealpathFlags::new(matches) {
                Ok(_) => panic!("missing operands must be rejected by RealpathFlags"),
                Err(error) => error,
            };

            assert_eq!(error.code(), 1);
            assert_eq!(error.to_string(), "missing operand");
        }

        #[test]
        fn test_app_valid_args() {
            let args = vec![ctcore::ct_util_name(), "test.txt"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }
    }
}
