/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved.
 *  syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt::Display;
#[cfg(unix)]
use std::fs;
use std::io::ErrorKind;
use std::iter;
#[cfg(unix)]
use std::os::unix::prelude::PermissionsExt;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

use clap::{builder::ValueParser, crate_version, Arg, ArgAction, ArgMatches, Command};
use rand::Rng;
use tempfile::Builder;

use ctcore::ct_display::{ct_println_verbatim, Quotable};
use ctcore::ct_error::{CTError, CTResult, CTsageError, FromIo};
use ctcore::{ct_format_usage, ct_help_about, ct_help_usage};

const MKTEMP_ABOUT: &str = ct_help_about!("mktemp.md");
const MKTEMP_USAGE: &str = ct_help_usage!("mktemp.md");

const MKTEMP_DEFAULT_TEMPLATE: &str = "tmp.XXXXXXXXXX";
mod mktemp_flags {
    pub const MKTEMP_DIRECTORY: &str = "directory";
    pub const MKTEMP_DRY_RUN: &str = "dry-run";
    pub const MKTEMP_QUIET: &str = "quiet";
    pub const MKTEMP_SUFFIX: &str = "suffix";
    pub const MKTEMP_TMPDIR: &str = "tmpdir";
    pub const MKTEMP_P: &str = "p";
    pub const MKTEMP_T: &str = "t";
}
const MKTEMP_ARG_TEMPLATE: &str = "template";

#[cfg(not(windows))]
const TMPDIR_ENV_VAR: &str = "TMPDIR";
#[cfg(windows)]
const TMPDIR_ENV_VAR: &str = "TMP";

#[derive(Debug)]
enum MkTempError {
    PersistError(PathBuf),
    MustEndInX(String),
    TooFewXs(String),

    /// 模板前缀包含路径分隔符（例如 `"a/bXXX"`）。
    PrefixContainsDirSeparator(String),

    /// 模板后缀包含路径分隔符（例如 `"XXXa/b"`）。
    SuffixContainsDirSeparator(String),
    InvalidTemplate(String),
    TooManyTemplates,

    /// 指定的临时目录未找到。
    NotFound(String, String),
}

impl CTError for MkTempError {
    fn usage(&self) -> bool {
        matches!(self, Self::TooManyTemplates)
    }
}

impl Error for MkTempError {}

impl Display for MkTempError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MkTempError::PersistError(p) => write!(f, "could not persist file {}", p.quote()),
            MkTempError::MustEndInX(s) => {
                write!(f, "with --suffix, template {} must end in X", s.quote())
            }
            MkTempError::TooFewXs(s) => write!(f, "too few X's in template {}", s.quote()),
            MkTempError::PrefixContainsDirSeparator(s) => {
                write!(
                    f,
                    "invalid template, {}, contains directory separator",
                    s.quote()
                )
            }
            MkTempError::SuffixContainsDirSeparator(s) => {
                write!(
                    f,
                    "invalid suffix {}, contains directory separator",
                    s.quote()
                )
            }
            MkTempError::InvalidTemplate(s) => write!(
                f,
                "invalid template, {}; with --tmpdir, it may not be absolute",
                s.quote()
            ),
            MkTempError::TooManyTemplates => {
                write!(f, "too many templates")
            }
            MkTempError::NotFound(template, s) => write!(
                f,
                "failed to create {} via template {}: No such file or directory",
                template,
                s.quote()
            ),
        }
    }
}

/// 从命令行解析的选项。
/// 这提供了应用程序逻辑和参数解析库 `clap` 之间的间接层，使每个都可以独立变化。
#[derive(Clone)]
pub struct MkTempFlags {
    /// 是否创建临时目录而不是文件。
    pub is_directory: bool,

    /// 是否只打印将创建的文件的名称。
    pub is_dry_run: bool,

    /// 是否抑制文件创建错误消息。
    pub is_quiet: bool,

    /// 创建临时文件的目录。
    /// 如果为 `None`，文件将创建在当前目录中。
    pub tmpdir: Option<PathBuf>,

    /// 要附加到临时文件的后缀（如果有）。
    pub suffix: Option<String>,

    /// 是否将模板参数视为单个文件路径组件。
    pub is_treat_as_template: bool,

    /// 用于临时文件名称的模板。
    pub template: String,
}

impl MkTempFlags {
    fn from(matches: &ArgMatches) -> Self {
        let tmp_dir = matches
            .get_one::<PathBuf>(mktemp_flags::MKTEMP_TMPDIR)
            .or_else(|| matches.get_one::<PathBuf>(mktemp_flags::MKTEMP_P))
            .cloned();
        let (tmpdir, template) = match matches.get_one::<String>(MKTEMP_ARG_TEMPLATE) {
            // 如果没有提供模板参数，则隐含 `--tmpdir`。
            None => {
                let tmpdir = Some(tmp_dir.unwrap_or_else(env::temp_dir));
                let template = MKTEMP_DEFAULT_TEMPLATE;
                (tmpdir, template.to_string())
            }
            Some(template) => {
                let tmpdir = if env::var(TMPDIR_ENV_VAR).is_ok()
                    && matches.get_flag(mktemp_flags::MKTEMP_T)
                {
                    env::var_os(TMPDIR_ENV_VAR).map(|t| t.into())
                } else if tmp_dir.is_some() {
                    tmp_dir
                } else if matches.get_flag(mktemp_flags::MKTEMP_T)
                    || matches.contains_id(mktemp_flags::MKTEMP_TMPDIR)
                {
                    // 如果提供了 --tmpdir 而没有参数，或者提供了 -t
                    // 导出到 TMPDIR
                    Some(env::temp_dir())
                } else {
                    None
                };
                (tmpdir, template.to_string())
            }
        };
        Self {
            is_directory: matches.get_flag(mktemp_flags::MKTEMP_DIRECTORY),
            is_dry_run: matches.get_flag(mktemp_flags::MKTEMP_DRY_RUN),
            is_quiet: matches.get_flag(mktemp_flags::MKTEMP_QUIET),
            tmpdir,
            suffix: matches
                .get_one::<String>(mktemp_flags::MKTEMP_SUFFIX)
                .map(String::from),
            is_treat_as_template: matches.get_flag(mktemp_flags::MKTEMP_T),
            template,
        }
    }
}

/// 控制临时文件路径和名称的参数。
/// 临时文件将创建在
///
/// ```text
/// {directory}/{prefix}{XXX}{suffix}
/// ```
///
/// 其中 `{XXX}` 是长度为 `num_rand_chars` 的随机字符序列。
#[derive(Debug)]
struct MkTempParams {
    /// 包含临时文件的目录。
    directory: PathBuf,

    /// 临时文件的（非随机）前缀。
    prefix: String,

    /// 临时文件名称中的随机字符数。
    rand_num_chars: usize,

    /// 临时文件的（非随机）后缀。
    suffix: String,
}

/// 查找最后一个连续的 X 块的起始和结束索引。
/// 如果找不到至少三个 X 的连续块，此函数返回 `None`。
///
/// # 示例
///
/// ```rust,ignore
/// assert_eq!(mktemp_find_last_contiguous_block_of_xs("XXX_XXX"), Some((4, 7)));
/// assert_eq!(mktemp_find_last_contiguous_block_of_xs("aXbXcX"), None);
/// ```
fn mktemp_find_last_contiguous_block_of_xs(s: &str) -> Option<(usize, usize)> {
    let j = s.rfind("XXX")? + 3;
    let i = s[..j].rfind(|c| c != 'X').map_or(0, |i| i + 1);
    Some((i, j))
}

impl MkTempParams {
    fn from(flags: MkTempFlags) -> Result<Self, MkTempError> {
        // 如果提供了后缀选项，模板参数必须以 'X' 结尾。
        if flags.suffix.is_some() && !flags.template.ends_with('X') {
            return Err(MkTempError::MustEndInX(flags.template));
        }

        // 获取模板中随机部分的起始和结束索引。
        // 例如，如果模板是 "abcXXXXyz"，那么 `i` 是 3，`j` 是 7。
        let (i, j) = if let Some(indices) = mktemp_find_last_contiguous_block_of_xs(&flags.template)
        {
            indices
        } else {
            let s = match flags.suffix {
                None => flags.template,
                Some(s) => format!("{}{}", flags.template, s),
            };
            return Err(MkTempError::TooFewXs(s));
        };

        // 组合作为选项给出的目录和模板的前缀。
        // 例如，如果 `tmpdir` 是 "a/b" 且模板是 "c/dXXX"，
        // 那么 `prefix` 是 "a/b/c/d"。
        let tmpdir = flags.tmpdir;
        let prefix_from_option = tmpdir.clone().unwrap_or_default();
        let prefix_from_template = &flags.template[..i];
        let prefix = Path::new(&prefix_from_option)
            .join(prefix_from_template)
            .display()
            .to_string();
        if flags.is_treat_as_template && prefix_from_template.contains(MAIN_SEPARATOR) {
            return Err(MkTempError::PrefixContainsDirSeparator(flags.template));
        }
        if tmpdir.is_some() && Path::new(prefix_from_template).is_absolute() {
            return Err(MkTempError::InvalidTemplate(flags.template));
        }

        // 将父目录与前缀路径部分分开。
        // 例如，如果 `prefix` 是 "a/b/c/d"，那么 `directory` 是
        // "a/b/c" 是 `prefix` 被重新分配给 "d"。
        let (dir, prefix) = if prefix.ends_with(MAIN_SEPARATOR) {
            (prefix, String::new())
        } else {
            let path = Path::new(&prefix);
            let dir = if let Some(d) = path.parent() {
                d.display().to_string()
            } else {
                String::new()
            };

            let prefix = if let Some(f) = path.file_name() {
                f.to_str().unwrap().to_string()
            } else {
                String::new()
            };

            (dir, prefix)
        };

        // 将模板中的后缀与选项给出的后缀组合起来。
        // 例如，如果命令行参数的后缀是 ".txt" 且
        // 模板是 "XXXabc"，那么 `suffix` 是 "abc.txt"。
        let suffix_from_flag = flags.suffix.unwrap_or_default();
        let suffix_from_template = &flags.template[j..];
        let suffix = format!("{}{}", suffix_from_template, suffix_from_flag);
        if suffix.contains(MAIN_SEPARATOR) {
            return Err(MkTempError::SuffixContainsDirSeparator(suffix));
        }

        // 模板中的随机字符数。
        // 例如，如果模板是 "abcXXXXyz"，那么随机字符数是四个。
        let rand_num_chars = j - i;

        Ok(Self {
            directory: dir.into(),
            prefix,
            rand_num_chars,
            suffix,
        })
    }
}

#[ctcore::main]
pub fn ctmain(args: impl ctcore::Args) -> CTResult<()> {
    mktemp_main(args)
}

pub fn mktemp_main(args: impl ctcore::Args) -> CTResult<()> {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let args_vec: Vec<_> = args.collect();
    let matches = match ct_app().try_get_matches_from(&args_vec) {
        Ok(m) => m,
        Err(e) => {
            if e.kind() == ErrorKind::TooManyValues
                && e.context().any(|(k, val)| {
                    k == ContextKind::InvalidArg
                        && val == &ContextValue::String("[template]".into())
                })
            {
                return Err(CTsageError::new(1, "too many templates"));
            }
            return Err(e.into());
        }
    };

    // 将命令行选项解析为适用于应用程序逻辑的 ct_format。
    let flags = MkTempFlags::from(&matches);

    if env::var("POSIXLY_CORRECT").is_ok() {
        // 如果设置了 POSIXLY_CORRECT，模板必须是最后一个参数。
        if matches.contains_id(MKTEMP_ARG_TEMPLATE) {
            // 提供了模板参数，检查是否是最后一个。
            if args_vec.last().unwrap() != OsStr::new(&flags.template) {
                return Err(Box::new(MkTempError::TooManyTemplates));
            }
        }
    }

    let is_dry_run = flags.is_dry_run;
    let is_suppress_file_err = flags.is_quiet;
    let is_make_dir = flags.is_directory;

    // 从命令行选项解析文件路径参数。
    let MkTempParams {
        directory: tmpdir,
        prefix,
        rand_num_chars: rand,
        suffix,
    } = MkTempParams::from(flags)?;

    // 创建临时文件或目录，或模拟创建它。
    let exec_res = match is_dry_run {
        true => mktemp_dry_exec(&tmpdir, &prefix, rand, &suffix),
        false => mktemp_exec(&tmpdir, &prefix, rand, &suffix, is_make_dir),
    };

    let res = match is_suppress_file_err {
        true => {
            // 将所有 UErrors 映射到 ExitCodes 防止错误被打印
            exec_res.map_err(|e| e.code().into())
        }
        false => exec_res,
    };

    ct_println_verbatim(res?).map_err_context(|| "failed to print directory name".to_owned())
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = MKTEMP_ABOUT;
    let usage_description = ct_format_usage(MKTEMP_USAGE);
    let args = vec![
        Arg::new(mktemp_flags::MKTEMP_DIRECTORY)
            .short('d')
            .long(mktemp_flags::MKTEMP_DIRECTORY)
            .help("Make a directory instead of a file")
            .action(ArgAction::SetTrue),
        Arg::new(mktemp_flags::MKTEMP_DRY_RUN)
            .short('u')
            .long(mktemp_flags::MKTEMP_DRY_RUN)
            .help("do not create anything; merely print a name (unsafe)")
            .action(ArgAction::SetTrue),
        Arg::new(mktemp_flags::MKTEMP_QUIET)
            .short('q')
            .long("quiet")
            .help("Fail silently if an error occurs.")
            .action(ArgAction::SetTrue),
        Arg::new(mktemp_flags::MKTEMP_SUFFIX)
            .long(mktemp_flags::MKTEMP_SUFFIX)
            .help(
                "append SUFFIX to TEMPLATE; SUFFIX must not contain a path separator. \
                      This option is implied if TEMPLATE does not end with X.",
            )
            .value_name("SUFFIX"),
        Arg::new(mktemp_flags::MKTEMP_P)
            .short('p')
            .help("short form of --tmpdir")
            .value_name("DIR")
            .num_args(1)
            .value_parser(ValueParser::path_buf())
            .value_hint(clap::ValueHint::DirPath),
        Arg::new(mktemp_flags::MKTEMP_TMPDIR)
            .long(mktemp_flags::MKTEMP_TMPDIR)
            .help(
                "interpret TEMPLATE relative to DIR; if DIR is not specified, use \
                      $TMPDIR ($TMP on windows) if set, else /tmp. With this option, \
                      TEMPLATE must not be an absolute name; unlike with -t, TEMPLATE \
                      may contain slashes, but mktemp creates only the final component",
            )
            .value_name("DIR")
            // 仅通过设置 --tmpdir 允许使用默认参数。否则，
            // 使用提供的输入生成 tmpdir
            .num_args(0..=1)
            // 需要等号以避免没有提供 tmpdir 时的歧义
            .require_equals(true)
            .overrides_with(mktemp_flags::MKTEMP_P)
            .value_parser(ValueParser::path_buf())
            .value_hint(clap::ValueHint::DirPath),
        Arg::new(mktemp_flags::MKTEMP_T)
            .short('t')
            .help(
                "Generate a template (using the supplied prefix and TMPDIR \
                 (TMP on windows) if set) to create a filename template [deprecated]",
            )
            .action(ArgAction::SetTrue),
        Arg::new(MKTEMP_ARG_TEMPLATE).num_args(..=1),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .args(args)
}

fn mktemp_dry_exec(tmpdir: &Path, prefix: &str, rand: usize, suffix: &str) -> CTResult<PathBuf> {
    let len = prefix.len() + suffix.len() + rand;
    let mut buffer = Vec::with_capacity(len);
    buffer.extend(prefix.as_bytes());
    buffer.extend(iter::repeat(b'X').take(rand));
    buffer.extend(suffix.as_bytes());

    // 随机化。
    let bytes = &mut buffer[prefix.len()..prefix.len() + rand];
    rand::thread_rng().fill(bytes);
    for b in bytes {
        *b = match *b % 62 {
            v @ 0..=9 => v + b'0',
            v @ 10..=35 => v - 10 + b'a',
            v @ 36..=61 => v - 36 + b'A',
            _ => unreachable!(),
        }
    }
    // 我们保证 utf8。
    let buf = String::from_utf8(buffer).unwrap();
    let tmp_dir = Path::new(tmpdir).join(buf);
    Ok(tmp_dir)
}

/// 使用给定的参数创建临时目录。
///
/// 此函数创建一个作为 `dir` 子目录的临时目录。目录的名称是
/// `prefix`、一串 `rand` 随机字符和 `suffix` 的连接。目录的权限设置为 `u+rwx`
///
/// # 错误
///
/// 如果临时目录无法写入磁盘或给定的目录 `dir` 不存在。
fn mktemp_dir(dir: &Path, prefix: &str, rand: usize, suffix: &str) -> CTResult<PathBuf> {
    let mut builder = Builder::new();
    builder.prefix(prefix).rand_bytes(rand).suffix(suffix);
    match builder.tempdir_in(dir) {
        Ok(d) => {
            // `into_path` 消耗 TempDir 而不删除它
            let p = d.into_path();
            #[cfg(not(windows))]
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700))?;
            Ok(p)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let file_name = format!("{}{}{}", prefix, "X".repeat(rand), suffix);
            let p = Path::new(dir).join(file_name);
            let s = p.display().to_string();
            Err(MkTempError::NotFound("directory".to_string(), s).into())
        }
        Err(e) => Err(e.into()),
    }
}

/// 使用给定的参数创建临时文件。
///
/// 此函数在目录 `dir` 中创建一个临时文件。文件的名称是
/// `prefix`、一串 `rand` 随机字符和 `suffix` 的连接。文件的权限设置为 `u+rw`。
///
/// # 错误
///
/// 如果文件无法写入磁盘或目录不存在。
fn mktemp_file(dir: &Path, prefix: &str, rand: usize, suffix: &str) -> CTResult<PathBuf> {
    let mut builder = Builder::new();
    builder.prefix(prefix).rand_bytes(rand).suffix(suffix);
    match builder.tempfile_in(dir) {
        // `keep` 确保文件不被删除
        Ok(named_temp_file) => match named_temp_file.keep() {
            Ok((_, p)) => Ok(p),
            Err(e) => Err(MkTempError::PersistError(e.file.path().to_path_buf()).into()),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let file_name = format!("{}{}{}", prefix, "X".repeat(rand), suffix);
            let p = Path::new(dir).join(file_name);
            let s = p.display().to_string();
            Err(MkTempError::NotFound("file".to_string(), s).into())
        }
        Err(e) => Err(e.into()),
    }
}

fn mktemp_exec(
    dir: &Path,
    prefix: &str,
    rand: usize,
    suffix: &str,
    make_dir: bool,
) -> CTResult<PathBuf> {
    let path = if make_dir {
        mktemp_dir(dir, prefix, rand, suffix)?
    } else {
        mktemp_file(dir, prefix, rand, suffix)?
    };

    // 获取到创建的临时文件或目录路径的最后一个组件。
    let filename = path.file_name();
    let filename = filename.unwrap().to_str().unwrap();

    // 将目录与路径连接以获取要打印的路径。我们
    // 不能使用 `Builder` 返回的路径，因为它给出了
    // 绝对路径，我们需要返回一个与命令行上给定模板匹配的文件名
    // 它可能是相对路径。
    let path_buf = Path::new(dir).join(filename);

    Ok(path_buf)
}

/// 创建临时文件或目录
///
/// 行为由 `flags` 参数确定，请参阅 [`MkTempFlags`] 了解详细信息。
pub fn mktemp(flags: &MkTempFlags) -> CTResult<PathBuf> {
    // 从命令行选项解析文件路径参数。
    let MkTempParams {
        directory: tmpdir,
        prefix,
        rand_num_chars: rand,
        suffix,
    } = MkTempParams::from(flags.clone())?;

    // 创建临时文件或目录，或模拟创建它。
    if flags.is_dry_run {
        mktemp_dry_exec(&tmpdir, &prefix, rand, &suffix)
    } else {
        mktemp_exec(&tmpdir, &prefix, rand, &suffix, flags.is_directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod options_from_tests {
        use clap::ArgMatches;

        use super::*;

        fn get_matches_from(args: &[&str]) -> ArgMatches {
            ct_app().get_matches_from(args)
        }

        #[test]
        fn test_options_from_basic() {
            let matches = get_matches_from(&[ctcore::ct_util_name(), "test.XXXXXX"]);
            let options = MkTempFlags::from(&matches);
            assert_eq!(options.is_directory, false);
            assert_eq!(options.is_dry_run, false);
            assert_eq!(options.is_quiet, false);
            assert_eq!(options.tmpdir, None);
            assert_eq!(options.suffix, None);
            assert_eq!(options.is_treat_as_template, false);
            assert_eq!(options.template, "test.XXXXXX".to_string());
        }

        #[test]
        fn test_options_from_with_flags() {
            let matches =
                get_matches_from(&[ctcore::ct_util_name(), "-d", "-u", "-q", "test.XXXXXX"]);
            let options = MkTempFlags::from(&matches);
            assert_eq!(options.is_directory, true);
            assert_eq!(options.is_dry_run, true);
            assert_eq!(options.is_quiet, true);
            assert_eq!(options.tmpdir, None);
            assert_eq!(options.suffix, None);
            assert_eq!(options.is_treat_as_template, false);
            assert_eq!(options.template, "test.XXXXXX".to_string());
        }

        #[test]
        fn test_options_from_with_suffix() {
            let matches =
                get_matches_from(&[ctcore::ct_util_name(), "--suffix", ".log", "test.XXXXXX"]);
            let options = MkTempFlags::from(&matches);
            assert_eq!(options.is_directory, false);
            assert_eq!(options.is_dry_run, false);
            assert_eq!(options.is_quiet, false);
            assert_eq!(options.tmpdir, None);
            assert_eq!(options.suffix, Some(".log".to_string()));
            assert_eq!(options.is_treat_as_template, false);
            assert_eq!(options.template, "test.XXXXXX".to_string());
        }

        #[test]
        fn test_options_from_with_p() {
            let matches = get_matches_from(&[
                ctcore::ct_util_name(),
                "-p",
                "/custom/tmpdir",
                "test.XXXXXX",
            ]);
            let options = MkTempFlags::from(&matches);
            assert_eq!(options.is_directory, false);
            assert_eq!(options.is_dry_run, false);
            assert_eq!(options.is_quiet, false);
            assert_eq!(options.tmpdir, Some(PathBuf::from("/custom/tmpdir")));
            assert_eq!(options.suffix, None);
            assert_eq!(options.is_treat_as_template, false);
            assert_eq!(options.template, "test.XXXXXX".to_string());
        }

        #[test]
        fn test_options_by_env() {
            // test_options_from_with_t
            {
                let matches = get_matches_from(&[ctcore::ct_util_name(), "-t", "test.XXXXXX"]);
                let options = MkTempFlags::from(&matches);
                assert_eq!(options.is_directory, false);
                assert_eq!(options.is_dry_run, false);
                assert_eq!(options.is_quiet, false);
                assert_eq!(options.tmpdir, Some(env::temp_dir()));
                assert_eq!(options.suffix, None);
                assert_eq!(options.is_treat_as_template, true);
                assert_eq!(options.template, "test.XXXXXX".to_string());
            }

            // test_options_from_no_template
            {
                let matches = get_matches_from(&[ctcore::ct_util_name()]);
                let options = MkTempFlags::from(&matches);
                assert_eq!(options.is_directory, false);
                assert_eq!(options.is_dry_run, false);
                assert_eq!(options.is_quiet, false);
                assert_eq!(options.tmpdir, Some(env::temp_dir()));
                assert_eq!(options.suffix, None);
                assert_eq!(options.is_treat_as_template, false);
                assert_eq!(options.template, "tmp.XXXXXXXXXX".to_string());
            }
        
            // test_options_from_with_environment_tmpdir
            {
                std::env::set_var("TMPDIR", "/custom/env_tmpdir");
                let matches = get_matches_from(&[ctcore::ct_util_name(), "-t"]);
                let options = MkTempFlags::from(&matches);
                assert_eq!(options.is_directory, false);
                assert_eq!(options.is_dry_run, false);
                assert_eq!(options.is_quiet, false);
                assert_eq!(options.tmpdir, Some(PathBuf::from("/custom/env_tmpdir")));
                assert_eq!(options.suffix, None);
                assert_eq!(options.is_treat_as_template, true);
                assert_eq!(options.template, "tmp.XXXXXXXXXX".to_string());
                std::env::remove_var("TMPDIR");
            }
        }

        #[test]
        fn test_options_from_with_empty_suffix() {
            let matches = get_matches_from(&[ctcore::ct_util_name(), "--suffix=", "test.XXXXXX"]);
            let options = MkTempFlags::from(&matches);
            assert_eq!(options.is_directory, false);
            assert_eq!(options.is_dry_run, false);
            assert_eq!(options.is_quiet, false);
            assert_eq!(options.tmpdir, None);
            assert_eq!(options.suffix, Some("".to_string()));
            assert_eq!(options.is_treat_as_template, false);
            assert_eq!(options.template, "test.XXXXXX".to_string());
        }

        #[test]
        fn test_options_from_with_template_and_flags() {
            let matches =
                get_matches_from(&[ctcore::ct_util_name(), "-d", "-u", "-q", "test.XXXXXX"]);
            let options = MkTempFlags::from(&matches);
            assert_eq!(options.is_directory, true);
            assert_eq!(options.is_dry_run, true);
            assert_eq!(options.is_quiet, true);
            assert_eq!(options.tmpdir, None);
            assert_eq!(options.suffix, None);
            assert_eq!(options.is_treat_as_template, false);
            assert_eq!(options.template, "test.XXXXXX".to_string());
        }
    }

    #[cfg(test)]
    mod mk_temp_error_tests {
        use super::*;

        #[test]
        fn test_mktemp_error_fmt_persist_error() {
            let path = PathBuf::from("/invalid/path");
            let error = MkTempError::PersistError(path.clone());
            let expected_message = format!("could not persist file '{}'", path.display());
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_must_end_in_x() {
            let template = "template_without_x".to_string();
            let error = MkTempError::MustEndInX(template.clone());
            let expected_message = format!("with --suffix, template '{}' must end in X", template);
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_too_few_xs() {
            let template = "too_few_X".to_string();
            let error = MkTempError::TooFewXs(template.clone());
            let expected_message = format!("too few X's in template '{}'", template);
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_prefix_contains_dir_separator() {
            let template = "invalid/template".to_string();
            let error = MkTempError::PrefixContainsDirSeparator(template.clone());
            let expected_message = format!(
                "invalid template, '{}', contains directory separator",
                template
            );
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_suffix_contains_dir_separator() {
            let suffix = "invalid/suffix".to_string();
            let error = MkTempError::SuffixContainsDirSeparator(suffix.clone());
            let expected_message =
                format!("invalid suffix '{}', contains directory separator", suffix);
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_invalid_template() {
            let template = "/absolute/template".to_string();
            let error = MkTempError::InvalidTemplate(template.clone());
            let expected_message = format!(
                "invalid template, '{}'; with --tmpdir, it may not be absolute",
                template
            );
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_too_many_templates() {
            let error = MkTempError::TooManyTemplates;
            let expected_message = "too many templates".to_string();
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_not_found() {
            let template_type = "file".to_string();
            let template = "non_existent_template".to_string();
            let error = MkTempError::NotFound(template_type.clone(), template.clone());
            let expected_message = format!(
                "failed to create {} via template '{}': No such file or directory",
                template_type, template
            );
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_usage_too_many_templates() {
            let error = MkTempError::TooManyTemplates;
            assert!(error.usage());
        }

        #[test]
        fn test_mktemp_error_usage_other_errors() {
            let errors = vec![
                MkTempError::PersistError(PathBuf::from("/invalid/path")),
                MkTempError::MustEndInX("template_without_x".to_string()),
                MkTempError::TooFewXs("too_few_X".to_string()),
                MkTempError::PrefixContainsDirSeparator("invalid/template".to_string()),
                MkTempError::SuffixContainsDirSeparator("invalid/suffix".to_string()),
                MkTempError::InvalidTemplate("/absolute/template".to_string()),
                MkTempError::NotFound("file".to_string(), "non_existent_template".to_string()),
            ];

            for error in errors {
                assert!(!error.usage());
            }
        }

        #[test]
        fn test_mktemp_error_fmt_not_found_directory() {
            let template_type = "directory".to_string();
            let template = "non_existent_template".to_string();
            let error = MkTempError::NotFound(template_type.clone(), template.clone());
            let expected_message = format!(
                "failed to create {} via template '{}': No such file or directory",
                template_type, template
            );
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_invalid_template_with_suffix() {
            let template = "template/with/suffix.XXXXXX".to_string();
            let error = MkTempError::InvalidTemplate(template.clone());
            let expected_message = format!(
                "invalid template, '{}'; with --tmpdir, it may not be absolute",
                template
            );
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_must_end_in_x_with_suffix() {
            let template = "template_without_x".to_string();
            let error = MkTempError::MustEndInX(template.clone());
            let expected_message = format!("with --suffix, template '{}' must end in X", template);
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_suffix_contains_dir_separator_with_template() {
            let suffix = "invalid/suffix".to_string();
            let error = MkTempError::SuffixContainsDirSeparator(suffix.clone());
            let expected_message =
                format!("invalid suffix '{}', contains directory separator", suffix);
            assert_eq!(format!("{}", error), expected_message);
        }

        #[test]
        fn test_mktemp_error_fmt_prefix_contains_dir_separator_with_template() {
            let template = "invalid/template".to_string();
            let error = MkTempError::PrefixContainsDirSeparator(template.clone());
            let expected_message = format!(
                "invalid template, '{}', contains directory separator",
                template
            );
            assert_eq!(format!("{}", error), expected_message);
        }
    }

    #[cfg(test)]
    mod params_from_tests {
        use super::*;

        fn create_options(
            directory: bool,
            dry_run: bool,
            quiet: bool,
            tmpdir: Option<PathBuf>,
            suffix: Option<String>,
            treat_as_template: bool,
            template: &str,
        ) -> MkTempFlags {
            MkTempFlags {
                is_directory: directory,
                is_dry_run: dry_run,
                is_quiet: quiet,
                tmpdir: tmpdir,
                suffix,
                is_treat_as_template: treat_as_template,
                template: template.to_string(),
            }
        }

        #[test]
        fn test_params_from_basic() {
            let options = create_options(false, false, false, None, None, false, "test.XXXXXX");
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_with_suffix() {
            let options = create_options(
                false,
                false,
                false,
                None,
                Some(".log".to_string()),
                false,
                "test.XXXXXX",
            );
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, ".log");
        }

        #[test]
        fn test_params_from_with_tmpdir() {
            let tmpdir = std::env::temp_dir().join("custom_tmpdir");
            let options = create_options(
                false,
                false,
                false,
                Some(tmpdir.clone()),
                None,
                false,
                "test.XXXXXX",
            );
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, tmpdir);
            assert_eq!(params.prefix, "test.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_invalid_template_with_suffix() {
            let options = create_options(
                false,
                false,
                false,
                None,
                Some(".log".to_string()),
                false,
                "test_without_x",
            );
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "with --suffix, template 'test_without_x' must end in X"
            );
        }

        #[test]
        fn test_params_from_too_few_xs() {
            let options = create_options(false, false, false, None, None, false, "too_few_X");
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "too few X's in template 'too_few_X'"
            );
        }

        #[test]
        fn test_params_from_prefix_contains_dir_separator() {
            let options = create_options(
                false,
                false,
                false,
                None,
                None,
                true,
                "invalid/template.XXXXXX",
            );
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "invalid template, 'invalid/template.XXXXXX', contains directory separator"
            );
        }

        #[test]
        fn test_params_from_invalid_template_absolute_path() {
            let options = create_options(
                false,
                false,
                false,
                Some(std::env::temp_dir()),
                None,
                false,
                "/absolute/template.XXXXXX",
            );
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().to_string(), "invalid template, '/absolute/template.XXXXXX'; with --tmpdir, it may not be absolute");
        }

        #[test]
        fn test_params_from_suffix_contains_dir_separator() {
            let options = create_options(
                false,
                false,
                false,
                None,
                Some("/invalid_suffix".to_string()),
                false,
                "template.XXXXXX",
            );
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "invalid suffix '/invalid_suffix', contains directory separator"
            );
        }

        #[test]
        fn test_params_from_no_tmpdir_with_t_flag() {
            let options = create_options(false, false, false, None, None, true, "test.XXXXXX");
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_with_env_var_tmpdir() {
            std::env::set_var("TMPDIR", "/custom/env_tmpdir");
            let options = create_options(false, false, false, None, None, true, "test.XXXXXX");
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
            std::env::remove_var("TMPDIR");
        }

        #[test]
        fn test_params_from_no_random_chars() {
            let options = create_options(false, false, false, None, None, false, "test.");
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "too few X's in template 'test.'"
            );
        }

        #[test]
        fn test_params_from_with_multiple_templates() {
            let options = create_options(
                false,
                false,
                false,
                None,
                None,
                false,
                "template1.XXXXXX template2.XXXXXX",
            );
            let result = MkTempParams::from(options).unwrap();
            assert_eq!(result.directory, PathBuf::from(""));
            assert_eq!(result.prefix, "template1.XXXXXX template2.");
            assert_eq!(result.rand_num_chars, 6);
            assert_eq!(result.suffix, "");
        }

        #[test]
        fn test_params_from_with_spaces_in_template() {
            let options = create_options(
                false,
                false,
                false,
                None,
                None,
                false,
                "test with spaces.XXXXXX",
            );
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test with spaces.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_with_trailing_slash_in_template() {
            let options = create_options(false, false, false, None, None, false, "test/XXXXXX");
            let result = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(result.directory, PathBuf::from("test/"));
            assert_eq!(result.prefix, "");
            assert_eq!(result.rand_num_chars, 6);
            assert_eq!(result.suffix, "");
        }

        #[test]
        fn test_params_from_with_trailing_xs_in_prefix() {
            let options = create_options(false, false, false, None, None, false, "testXXX.XXXXXX");
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "testXXX.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_with_leading_xs_in_suffix() {
            let options = create_options(
                false,
                false,
                false,
                None,
                Some("XXX.log".to_string()),
                false,
                "test.XXXXXX",
            );
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test.");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "XXX.log");
        }

        #[test]
        fn test_params_from_with_empty_template() {
            let options = create_options(false, false, false, None, None, false, "");
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "too few X's in template ''"
            );
        }

        #[test]
        fn test_params_from_with_no_template() {
            let options = create_options(false, false, false, None, None, false, "");
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "too few X's in template ''"
            );
        }

        #[test]
        fn test_params_from_with_only_xs() {
            let options = create_options(false, false, false, None, None, false, "XXXXXX");
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_with_special_chars_in_template() {
            let options = create_options(false, false, false, None, None, false, "test!@#XXXXXX");
            let params = MkTempParams::from(options).expect("Failed to create Params");
            assert_eq!(params.directory, PathBuf::from(""));
            assert_eq!(params.prefix, "test!@#");
            assert_eq!(params.rand_num_chars, 6);
            assert_eq!(params.suffix, "");
        }

        #[test]
        fn test_params_from_with_no_xs_and_suffix() {
            let options = create_options(
                false,
                false,
                false,
                None,
                Some(".log".to_string()),
                false,
                "test",
            );
            let result = MkTempParams::from(options);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "with --suffix, template 'test' must end in X"
            );
        }
    }

    #[cfg(test)]
    mod find_last_contiguous_block_of_xs_tests {
        use super::*;

        #[test]
        fn test_find_last_contiguous_block_of_xs_basic() {
            let s = "abcXXXdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((3, 6)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_multiple_blocks() {
            let s = "abcXXXdefXXXghi";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((9, 12)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_single_x() {
            let s = "abcXdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_mixed_case() {
            let s = "abcXXxXdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_no_x() {
            let s = "abcdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_trailing_xs() {
            let s = "abcdefXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((6, 9)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_leading_xs() {
            let s = "XXXabcdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((0, 3)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_continuous_xs() {
            let s = "abcXXXXXXXXXdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((3, 12)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_no_continuous_block_of_three() {
            let s = "abcXXdefXXghi";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_exactly_three_xs() {
            let s = "abcXXXdefXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((9, 12)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_empty_string() {
            let s = "";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_x_only() {
            let s = "XXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((0, 4)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_spaces() {
            let s = "abc XXX def XXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((12, 15)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_newlines() {
            let s = "abc\nXXX\ndef\nXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((12, 15)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_tabs() {
            let s = "abc\tXXX\tdef\tXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((12, 15)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_special_chars() {
            let s = "abc!@#XXX$%^XXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((12, 15)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_all_x() {
            let s = "XXXXXXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((0, 8)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_digits() {
            let s = "abc123XXXdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((6, 9)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_special_x() {
            let s = "abc*XXX*def";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((4, 7)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_leading_and_trailing_xs() {
            let s = "XXXabcXXXdefXXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((12, 15)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_intermittent_xs() {
            let s = "abcXXdefXXghiXXjkl";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_single_contiguous_block_of_three() {
            let s = "abcXXdefXXXghi";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((8, 11)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_leading_non_x_chars() {
            let s = "XXabcXXXdef";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((5, 8)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_only_three_xs() {
            let s = "XXX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, Some((0, 3)));
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_only_two_xs() {
            let s = "XX";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs_with_special_characters_between_xs() {
            let s = "X@X#X$X%X^X";
            let result = mktemp_find_last_contiguous_block_of_xs(s);
            assert_eq!(result, None);
        }

        #[test]
        fn test_find_last_contiguous_block_of_xs() {
            assert_eq!(mktemp_find_last_contiguous_block_of_xs("XXX"), Some((0, 3)));
            assert_eq!(
                mktemp_find_last_contiguous_block_of_xs("XXX_XXX"),
                Some((4, 7))
            );
            assert_eq!(
                mktemp_find_last_contiguous_block_of_xs("XXX_XXX_XXX"),
                Some((8, 11))
            );
            assert_eq!(
                mktemp_find_last_contiguous_block_of_xs("aaXXXbb"),
                Some((2, 5))
            );
            assert_eq!(mktemp_find_last_contiguous_block_of_xs(""), None);
            assert_eq!(mktemp_find_last_contiguous_block_of_xs("X"), None);
            assert_eq!(mktemp_find_last_contiguous_block_of_xs("XX"), None);
            assert_eq!(mktemp_find_last_contiguous_block_of_xs("aXbXcX"), None);
            assert_eq!(mktemp_find_last_contiguous_block_of_xs("aXXbXXcXX"), None);
        }
    }
}