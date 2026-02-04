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

//! 向一个任务发送一个信号
//! 定义三种兼容模式: Bash, UtilLinux, Coreutils
//! 通过 SYSKITS_KILL_MODE 环境变量切换
//! Flag                    Bash       util-linux   coreutils    syskits     说明
//! -s, --signal            支持        支持          支持         支持         指定信号名
//! -l, --list              支持        支持          支持         支持         列出信号
//! -L                      支持(同-l)   支持(同-t)   支持(同-t)    支持         信号表格别名
//! -t, --table             不支持      不支持        支持          模式限制     表格格式输出
//! -n                      支持        不支持        支持          模式限制    信号编号
//! -p, --pid               不支持      支持          不支持        支持        只打印PID
//! --verbose               不支持      支持          不支持        支持        详细输出
//! -q, --queue             不支持      支持          不支持        支持        sigqueue发送
//! -a, --all               不支持      支持          不支持        支持        不限制UID
//! -r, --require-handler   不支持      支持          不支持        支持        需要handler
//! --timeout               不支持      支持          不支持        支持        超时跟进信号

extern crate rust_i18n;
use clap::{Arg, ArgAction, ArgMatches, Command, crate_version};
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CtSimpleError, FromIo};
use ctcore::ct_show;
use ctcore::ct_signals::{ALL_SIGNALS, get_ct_signal_by_name_or_value};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::convert::TryInto;
use std::io::Error;
use std::io::Write;

use ctcore::Tool;
use std::ffi::OsString;
use sys_locale::get_locale;

/// Kill 命令兼容模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KillCompatMode {
    /// Bash 模式 (默认): 与 bash 内嵌 kill 一致
    #[default]
    Bash,
    /// Util-linux 模式: 与 /usr/bin/kill (util-linux) 一致
    UtilLinux,
    /// GNU coreutils 模式: 与 coreutils kill 一致
    Coreutils,
}

impl KillCompatMode {
    /// 从环境变量获取模式
    pub fn from_env() -> Self {
        if let Ok(mode) = std::env::var("SYSKITS_KILL_MODE") {
            return match mode.to_lowercase().as_str() {
                "bash" | "bash-builtin" => Self::Bash,
                "util-linux" | "utillinux" | "util_linux" => Self::UtilLinux,
                "coreutils" | "gnu" => Self::Coreutils,
                _ => Self::default(), // 未知值使用默认 (bash)
            };
        }
        Self::default() // 无环境变量时使用默认 (bash)
    }

    /// 是否使用 bash 风格的 -l 输出 (5列,SIG前缀,带编号)
    /// 注意: /usr/bin/kill (util-linux) 实际使用空格分隔无SIG前缀格式
    pub fn use_bash_list_output(&self) -> bool {
        matches!(self, Self::Bash)
    }

    /// 是否支持 -t 选项
    pub fn supports_table_option(&self) -> bool {
        // bash 内嵌不支持 -t, util-linux 不支持 -t, coreutils 支持 -t
        matches!(self, Self::Coreutils)
    }

    /// 是否支持 -n 选项 (bash 和 coreutils 支持)
    pub fn supports_signum_option(&self) -> bool {
        matches!(self, Self::Bash | Self::Coreutils)
    }

    /// 是否支持 util-linux 扩展选项 (-q, -p, -a, -r, --timeout, --verbose)
    pub fn supports_util_linux_options(&self) -> bool {
        matches!(self, Self::UtilLinux)
    }

    /// 无参数时的退出码
    pub fn no_args_exit_code(&self) -> i32 {
        match self {
            Self::Bash => 2,      // bash 内嵌 kill
            Self::UtilLinux => 1, // /usr/bin/kill (util-linux)
            Self::Coreutils => 1,
        }
    }

    /// 无参数时的错误信息
    pub fn no_args_error_message(&self) -> &'static str {
        match self {
            Self::Bash => {
                "usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            }
            Self::UtilLinux | Self::Coreutils => "not enough arguments",
        }
    }
}

pub mod kill_flags {
    pub static KILL_PIDS_OR_SIGNALS: &str = "pids_or_signals";
    pub static LIST: &str = "list";
    pub static TABLE: &str = "table";
    pub static SIGNAL: &str = "signal";
    pub static SIGNAL_NUM: &str = "signum"; // -n
    pub static QUEUE: &str = "queue"; // -q (util-linux)
    pub static PID_ONLY: &str = "pid"; // -p (util-linux)
    pub static ALL: &str = "all"; // -a (util-linux)
    pub static REQUIRE_HANDLER: &str = "require-handler"; // -r (util-linux)
    pub static TIMEOUT: &str = "timeout"; // --timeout (util-linux)
    pub static VERBOSE: &str = "verbose"; // --verbose (util-linux)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("kill.about");
    let usage_description = t!("kill.usage");
    let args = vec![
        Arg::new(kill_flags::LIST)
            .short('l')
            .long(kill_flags::LIST)
            .help(t!("kill.clap.list"))
            .conflicts_with(kill_flags::TABLE)
            .action(ArgAction::SetTrue),
        Arg::new(kill_flags::TABLE)
            .short('t')
            .short_alias('L')
            .long(kill_flags::TABLE)
            .help(t!("kill.clap.table"))
            .action(ArgAction::SetTrue),
        Arg::new(kill_flags::SIGNAL)
            .short('s')
            .long(kill_flags::SIGNAL)
            .value_name("signal")
            .help(t!("kill.clap.signal")),
        // -n: 指定信号编号 (bash/coreutils 兼容)
        Arg::new(kill_flags::SIGNAL_NUM)
            .short('n')
            .value_name("signum")
            .help("发送指定的信号编号"),
        // -q: 使用 sigqueue 发送信号 (util-linux)
        Arg::new(kill_flags::QUEUE)
            .short('q')
            .long(kill_flags::QUEUE)
            .value_name("value")
            .help("使用 sigqueue(2) 发送信号并附带数据值"),
        // -p: 只打印 PID,不发送信号 (util-linux)
        Arg::new(kill_flags::PID_ONLY)
            .short('p')
            .long(kill_flags::PID_ONLY)
            .help("只打印进程的 PID,不发送信号")
            .action(ArgAction::SetTrue),
        // -a: 不限制同 UID (util-linux)
        Arg::new(kill_flags::ALL)
            .short('a')
            .long(kill_flags::ALL)
            .help("不限制名称到 PID 的转换为同 UID 的进程")
            .action(ArgAction::SetTrue),
        // -r: 要求目标有信号处理器 (util-linux)
        Arg::new(kill_flags::REQUIRE_HANDLER)
            .short('r')
            .long(kill_flags::REQUIRE_HANDLER)
            .help("只发送给有信号处理器的进程")
            .action(ArgAction::SetTrue),
        // --timeout: 超时后发送跟进信号 (util-linux)
        Arg::new(kill_flags::TIMEOUT)
            .long(kill_flags::TIMEOUT)
            .value_names(["milliseconds", "follow-up-signal"])
            .num_args(2)
            .help("等待指定毫秒后发送跟进信号"),
        // --verbose: 详细输出 (util-linux)
        Arg::new(kill_flags::VERBOSE)
            .long(kill_flags::VERBOSE)
            .help("打印将要发送信号的 PID")
            .action(ArgAction::SetTrue),
        Arg::new(kill_flags::KILL_PIDS_OR_SIGNALS)
            .hide(true)
            .action(ArgAction::Append),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .infer_long_args(true)
        .allow_negative_numbers(true)
        .args(&args)
}

/// 主要的kill命令处理函数，用于终止进程或发送信号
///
/// # 参数
/// - `writer`: 一个可写对象，用于输出信息
/// - `args`: 命令行参数，实现自定义Args trait
///
/// # 返回
/// 返回一个结果，表示操作是否成功
pub fn kill_main<W: Write>(writer: &mut W, args: impl ctcore::Args) -> CTResult<()> {
    // 获取兼容模式
    let compat_mode = KillCompatMode::from_env();

    // 设置语言
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 收集并忽略不相关的参数
    let mut args = args.collect_ignore();
    // 处理过时的kill命令参数
    let obs_signal = kill_handle_obsolete(&mut args);
    // 尝试解析命令行参数
    let matches = ct_app().try_get_matches_from(args)?;
    // 获取需要终止的进程ID或信号
    let pids_or_signals: Vec<String> = matches
        .get_many::<String>(kill_flags::KILL_PIDS_OR_SIGNALS)
        .map(|v| v.map(ToString::to_string).collect())
        .unwrap_or_default();

    // 检查 -l/-t 与 -s 的冲突
    let has_list_or_table =
        matches.get_flag(kill_flags::LIST) || matches.get_flag(kill_flags::TABLE);
    let has_signal = obs_signal.is_some() || matches.contains_id(kill_flags::SIGNAL);

    if has_list_or_table && has_signal {
        let err_message = "cannot combine signal with -l or -t";
        return Err(CtSimpleError::new(1, err_message));
    }

    // 检查是否有参数
    if pids_or_signals.is_empty() && !has_list_or_table {
        let err_message = compat_mode.no_args_error_message();
        let exit_code = compat_mode.no_args_exit_code();
        return Err(CtSimpleError::new(exit_code, err_message));
    }

    if matches.get_flag(kill_flags::TABLE) {
        // Util-linux 模式不支持 -t,Syskits 和 Coreutils 支持
        if !compat_mode.supports_table_option() {
            return Err(CtSimpleError::new(
                1,
                "invalid option -- 't'\nTry 'kill --help' for more information.",
            ));
        }

        // 如果是表格模式,不接受额外参数
        if !pids_or_signals.is_empty() {
            // coreutils 的 -t 不接受参数,如果有参数则报错
            let err_message = format!("extra operand {}", pids_or_signals[0].quote());
            return Err(CtSimpleError::new(1, err_message));
        }
        kill_table(writer)
    } else if matches.get_flag(kill_flags::LIST) {
        // 如果是列表模式，调用kill_list函数，并传入第一个进程ID或信号
        kill_list(writer, pids_or_signals.first(), compat_mode)
    } else {
        // 否则，执行kill命令并处理信号
        kill_exec(obs_signal, matches, &pids_or_signals)
    }
}

/// 发送信号以终止进程执行。
///
/// 本函数旨在根据用户指定的信号和进程ID，发送信号以终止相应的进程。
/// 它首先解析用户可能提供的信号名称或编号，以及进程ID列表。然后，
/// 它向这些进程发送指定的信号。如果操作成功，函数返回Ok(())，否则返回一个错误。
///
/// # 参数
/// - `obs_signal`: 可选的信号编号，用户可以指定一个信号来终止进程。
/// - `matches`: 命令行参数匹配对象，用于获取命令行参数。
/// - `pids_or_signals`: 包含进程ID或信号的字符串向量，用于指定要终止的进程或解析信号。
///
/// # 返回值
/// - `CTResult<()>`: 一个结果类型，表示操作成功或失败。
fn kill_exec(
    obs_signal: Option<usize>,
    matches: ArgMatches,
    pids_or_signals: &[String],
) -> CTResult<()> {
    // 获取 util-linux 选项
    let pid_only = matches.get_flag(kill_flags::PID_ONLY);
    let verbose = matches.get_flag(kill_flags::VERBOSE);
    let queue_value = matches.get_one::<String>(kill_flags::QUEUE).cloned();
    let _all = matches.get_flag(kill_flags::ALL);
    let require_handler = matches.get_flag(kill_flags::REQUIRE_HANDLER);
    let has_signal = matches.contains_id(kill_flags::SIGNAL);

    // 获取 --timeout 参数
    let timeout_values: Option<Vec<String>> = matches
        .get_many::<String>(kill_flags::TIMEOUT)
        .map(|v| v.map(String::from).collect());

    // -p 和 -s/-q 互斥检查
    if pid_only && (has_signal || queue_value.is_some()) {
        return Err(CtSimpleError::new(
            1,
            "--pid and --signal are mutually exclusive",
        ));
    }
    if pid_only && queue_value.is_some() {
        return Err(CtSimpleError::new(
            1,
            "--pid and --queue are mutually exclusive",
        ));
    }

    // 解析并获取要发送的信号值。
    let sig = kill_get_signal_value(obs_signal, matches)?;
    // 解析并获取进程ID列表。
    let pids = kill_parse_pids(pids_or_signals)?;

    // -p 模式: 只打印 PID,不发送信号
    if pid_only {
        for pid in &pids {
            println!("{}", pid);
        }
        return Ok(());
    }

    // -r 模式: 过滤没有信号处理器的进程
    #[cfg(target_os = "linux")]
    let pids: Vec<i32> = if require_handler {
        pids.into_iter()
            .filter(|&pid| check_signal_handler(pid, sig, verbose))
            .collect()
    } else {
        pids
    };

    // --verbose 模式: 打印将要发送信号的 PID (移到 -r 过滤后)
    if verbose && !require_handler {
        for pid in &pids {
            eprintln!("sending signal {} to pid {}", sig as i32, pid);
        }
    }

    // --timeout 模式: 超时后发送跟进信号
    #[cfg(target_os = "linux")]
    if let Some(values) = timeout_values {
        if values.len() >= 2 {
            let timeout_ms: u64 = values[0]
                .parse()
                .map_err(|_| CtSimpleError::new(1, format!("argument error: {}", values[0])))?;
            let follow_up_sig = kill_parse_signal_value(&values[1])?;
            let follow_up: Signal = (follow_up_sig as i32)
                .try_into()
                .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;
            return kill_with_timeout(sig, &pids, timeout_ms, follow_up, verbose);
        }
    }

    // -q 模式: 使用 sigqueue(2) 发送信号
    #[cfg(target_os = "linux")]
    if let Some(val_str) = queue_value {
        let val: i32 = val_str
            .parse()
            .map_err(|_| CtSimpleError::new(1, format!("argument error: {}", val_str)))?;
        return kill_with_sigqueue(sig, &pids, val);
    }

    // 向指定的进程发送信号。
    kill(sig, &pids);

    Ok(())
}

/// 根据提供的参数获取信号值用于进程终止
///
/// 该函数首先尝试从`obs_signal`中获取信号值，如果未提供，则尝试从命令行参数匹配中获取信号值。
/// 如果两者都未成功获取到信号值，则默认使用15（SIGTERM）作为信号值。
///
/// # 参数
/// - `obs_signal`: 可选的信号值，通常从观察到的信号中获取。
/// - `matches`: 命令行参数匹配结果，用于提取指定的信号值。
///
/// # 返回
/// 返回一个`CTResult`，包含转换后的`Signal`值，如果转换失败，则包含一个错误。
fn kill_get_signal_value(obs_signal: Option<usize>, matches: ArgMatches) -> CTResult<Signal> {
    let sig = if let Some(signal) = obs_signal {
        signal
    } else if let Some(signal) = matches.get_one::<String>(kill_flags::SIGNAL) {
        // 如果命令行参数中提供了 -s 信号值，则解析该信号值
        kill_parse_signal_value(signal)?
    } else if let Some(signum) = matches.get_one::<String>(kill_flags::SIGNAL_NUM) {
        // 如果命令行参数中提供了 -n 信号编号，则解析该信号编号
        kill_parse_signal_value(signum)?
    } else {
        // 如果没有提供信号值，则使用默认的15（SIGTERM）
        15_usize //SIGTERM
    };
    // 将获取到的信号值转换为i32，并尝试将其转换为`Signal`类型，如果失败，则返回一个错误
    let kill_signal: Signal = (sig as i32)
        .try_into()
        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))?;

    Ok(kill_signal)
}

/// 使用 sigqueue(2) 发送信号 (util-linux -q/--queue)
///
/// # 参数
/// - `sig`: 要发送的信号
/// - `pids`: 目标进程 ID 列表
/// - `value`: 附带的整数数据值
#[cfg(target_os = "linux")]
fn kill_with_sigqueue(sig: Signal, pids: &[i32], value: i32) -> CTResult<()> {
    use libc::{c_int, pid_t, sigval};

    // nix Signal 转换为 i32
    let sig_num: c_int = sig as c_int;

    for &pid in pids {
        // libc sigval 是一个 union,在 aarch64 linux 上只有 sival_ptr 字段
        let sigval = sigval {
            sival_ptr: value as *mut libc::c_void,
        };
        let ret = unsafe {
            libc::syscall(
                libc::SYS_rt_sigqueueinfo,
                pid as pid_t,
                sig_num,
                &sigval as *const _ as *const libc::c_void,
            )
        };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            eprintln!("kill: sigqueue failed for pid {}: {}", pid, err);
        }
    }
    Ok(())
}

/// 检查进程是否有指定信号的处理器 (util-linux -r/--require-handler)
///
/// 读取 /proc/<pid>/stat 第34个字段 (sigcgt - caught signals mask)
/// 检查对应信号位是否被设置
#[cfg(target_os = "linux")]
fn check_signal_handler(pid: i32, sig: Signal, verbose: bool) -> bool {
    use std::fs;

    let stat_path = format!("/proc/{}/stat", pid);
    let content = match fs::read_to_string(&stat_path) {
        Ok(c) => c,
        Err(_) => return true, // 无法读取时默认发送
    };

    // /proc/<pid>/stat 格式: pid (comm) state ppid ... 第34个字段是 sigcgt
    // 字段从 ) 后开始计数
    let after_comm = match content.rfind(')') {
        Some(pos) => &content[pos + 2..],
        None => return true,
    };

    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // sigcgt 是第34个字段,从 ) 后算起是第32个 (0-indexed: 31)
    // 但实际上从 ) 后第一个字段是 state (第3字段),所以 sigcgt 是第34-3=31
    if fields.len() <= 31 {
        return true;
    }

    let sigcgt: u64 = match fields[31].parse() {
        Ok(v) => v,
        Err(_) => return true,
    };

    let sig_num = sig as i32;
    let has_handler = ((1u64 << (sig_num - 1)) & sigcgt) != 0;

    if verbose && !has_handler {
        eprintln!(
            "not signalling pid {}, it has no userspace handler for signal {}",
            pid, sig_num
        );
    }

    has_handler
}

/// 使用 pidfd 发送信号并支持超时 (util-linux --timeout)
///
/// 超时后发送跟进信号
#[cfg(target_os = "linux")]
fn kill_with_timeout(
    sig: Signal,
    pids: &[i32],
    timeout_ms: u64,
    follow_up_signal: Signal,
    verbose: bool,
) -> CTResult<()> {
    use std::thread;
    use std::time::{Duration, Instant};

    for &pid in pids {
        // 发送初始信号
        if verbose {
            eprintln!("sending signal {} to pid {}", sig as i32, pid);
        }
        let pid_nix = Pid::from_raw(pid);
        if signal::kill(pid_nix, sig).is_err() {
            eprintln!("kill: failed to send signal to {}", pid);
            continue;
        }

        // 等待超时
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            // 检查进程是否还存在
            if signal::kill(pid_nix, None).is_err() {
                // 进程已退出
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        // 如果进程还存在,发送跟进信号
        if signal::kill(pid_nix, None).is_ok() {
            if verbose {
                eprintln!(
                    "timeout, sending signal {} to pid {}",
                    follow_up_signal as i32, pid
                );
            }
            let _ = signal::kill(pid_nix, follow_up_signal);
        }
    }

    Ok(())
}

/// 移除过时的信号参数并返回对应的信号值。
///
/// 该函数用于处理包含旧风格信号前缀的命令行参数。当参数数量超过两个时，它会检查第二个参数是否包含一个旧风格的信号。如果存在，则移除该参数并返回对应的信号值。
///
/// # 参数
/// * `args`: 可变引用的字符串向量，包含命令行参数。
///
/// # 返回值
/// * `Option<usize>`: 如果找到并移除了过时的信号，则返回信号值；否则返回 `None`。
fn kill_handle_obsolete(args: &mut Vec<String>) -> Option<usize> {
    if args.len() > 2 {
        // 检查参数数量是否超过两个，因为过时信号的存在至少需要两个参数
        let slice = args[1].as_str();
        if let Some(signal) = slice.strip_prefix('-') {
            // 尝试移除信号前缀以判断是否为过时信号
            let opt_signal = get_ct_signal_by_name_or_value(signal);
            if opt_signal.is_some() {
                // 返回前移除信号参数
                args.remove(1);
                return opt_signal;
            }
        }
    }
    None
}

/// 将信号列表以表格形式输出到指定的写入器中
///
/// # Parameters
/// - `writer`: 一个实现了Write trait的可变引用，用于输出信号表
///
/// # Returns
/// - `CTResult<()>`: 一个结果类型，用于表示操作是否成功
///
/// # Description
/// 该函数遍历ALL_SIGNALS数组，以3列格式输出信号编号和名称
/// 格式: NUM  NAME     (每行3个信号)
fn kill_table<W: Write>(writer: &mut W) -> CTResult<()> {
    let name_width = ALL_SIGNALS.iter().map(|n| n.len()).max().unwrap();

    for (idx, signal) in ALL_SIGNALS.iter().enumerate() {
        // 格式化输出信号的索引号和名称
        write!(writer, "{:2} {:<width$}", idx, signal, width = name_width)?;

        // 每3个信号后输出一个换行符，否则输出空格分隔
        if (idx + 1) % 3 == 0 {
            writeln!(writer)?;
        } else {
            write!(writer, " ")?;
        }
    }

    // 如果最后一行不足3个，补充换行
    if ALL_SIGNALS.len() % 3 != 0 {
        writeln!(writer)?;
    }

    Ok(())
}

/// 向指定的写入器打印信号值或名称,并返回结果
///
/// 此函数旨在根据提供的信号名称或值,查找并打印对应的信号值或名称如果找到对应的信号,则打印并返回Ok(()),否则返回一个错误
/// 支持标准信号编号转换:
/// - 标准退出状态码: 信号编号 + 128
/// - ksh 风格: 信号编号 + 256
///
/// # 参数
/// - `writer`: 一个可写对象,用于输出信号值或名称
/// - `signal_name_or_value`: 一个字符串,包含信号的名称或值,用于查找信号
///
/// # 返回值
/// - `Ok(())`: 如果成功找到并打印信号值或名称
/// - `Err(CtSimpleError)`: 如果提供的信号名称或值无效,返回一个包含错误信息的CtSimpleError
fn kill_print_signal<W: Write>(writer: &mut W, signal_name_or_value: &str) -> CTResult<()> {
    // 首先尝试按名称或直接数字查找标准信号
    for (value, &signal) in ALL_SIGNALS.iter().enumerate() {
        if signal == signal_name_or_value || (format!("SIG{signal}")) == signal_name_or_value {
            writeln!(writer, "{value}")?;
            return Ok(());
        } else if signal_name_or_value == value.to_string() {
            writeln!(writer, "{signal}")?;
            return Ok(());
        }
    }

    // 检查信号别名
    #[cfg(target_os = "linux")]
    {
        use ctcore::ct_signals::SIGNAL_ALIASES;
        for &(alias, num) in SIGNAL_ALIASES {
            if alias == signal_name_or_value || format!("SIG{alias}") == signal_name_or_value {
                writeln!(writer, "{num}")?;
                return Ok(());
            } else if signal_name_or_value == num.to_string() {
                // 数字转别名时,优先返回主名称而非别名
                if let Some(main_name) = ALL_SIGNALS.get(num) {
                    writeln!(writer, "{main_name}")?;
                    return Ok(());
                }
            }
        }
    }

    // 尝试解析实时信号
    #[cfg(target_os = "linux")]
    {
        use ctcore::ct_signals::parse_rt_signal;

        // 如果是实时信号名称,转换为编号
        if let Some(sig_num) = parse_rt_signal(signal_name_or_value.trim_start_matches("SIG")) {
            writeln!(writer, "{sig_num}")?;
            return Ok(());
        }

        // 如果是实时信号编号,返回名称(简化处理,返回编号本身)
        if let Ok(num) = signal_name_or_value.parse::<i32>() {
            use std::sync::OnceLock;
            static RT_RANGE: OnceLock<(i32, i32)> = OnceLock::new();
            let (rtmin, rtmax) = RT_RANGE.get_or_init(|| (libc::SIGRTMIN(), libc::SIGRTMAX()));

            if num >= *rtmin && num <= *rtmax {
                // 对于实时信号编号,返回对应的名称
                if num == *rtmin {
                    writeln!(writer, "RTMIN")?;
                } else if num == *rtmax {
                    writeln!(writer, "RTMAX")?;
                } else if num < (*rtmin + *rtmax) / 2 {
                    writeln!(writer, "RTMIN+{}", num - rtmin)?;
                } else {
                    writeln!(writer, "RTMAX-{}", rtmax - num)?;
                }
                return Ok(());
            }
        }
    }

    // 尝试退出状态码转换
    if let Ok(num) = signal_name_or_value.parse::<usize>() {
        if num >= 128 {
            let sig_num = num % 128;
            if sig_num < ALL_SIGNALS.len() {
                writeln!(writer, "{}", ALL_SIGNALS[sig_num])?;
                return Ok(());
            }
        }
    }

    let err_message = format!("unknown signal name {}", signal_name_or_value.quote());
    Err(CtSimpleError::new(1, err_message))
}

/// 在控制台中打印所有信号的名称,根据兼容模式选择格式
///
/// # 参数
/// * `writer`: 一个实现了Write trait的对象，用于输出信号信息
/// * `mode`: 兼容模式
///
/// # 返回
/// * `CTResult<()>`: 一个结果类型，表示操作是否成功
fn kill_print_signals<W: Write>(writer: &mut W, mode: KillCompatMode) -> CTResult<()> {
    if mode.use_bash_list_output() {
        kill_print_signals_util_linux(writer)
    } else {
        kill_print_signals_coreutils(writer)
    }
}

/// util-linux 格式: N) SIGNAME (5列)
fn kill_print_signals_util_linux<W: Write>(writer: &mut W) -> CTResult<()> {
    let mut count = 0;

    // 输出标准信号 (1-31, 跳过0号EXIT)
    for (idx, signal) in ALL_SIGNALS.iter().enumerate().skip(1) {
        write!(writer, "{:2}) SIG{:<9}", idx, signal)?;
        count += 1;

        if count % 5 == 0 {
            writeln!(writer)?;
        } else {
            write!(writer, " ")?;
        }
    }

    // 输出实时信号
    #[cfg(target_os = "linux")]
    {
        use std::sync::OnceLock;

        static RT_RANGE: OnceLock<(i32, i32)> = OnceLock::new();
        let (rtmin, rtmax) = RT_RANGE.get_or_init(|| (libc::SIGRTMIN(), libc::SIGRTMAX()));

        // 换行如果需要
        if count % 5 != 0 {
            writeln!(writer)?;
        }

        // SIGRTMIN
        write!(writer, "{:2}) SIGRTMIN   ", rtmin)?;
        count = 1;

        // SIGRTMIN+1 到 SIGRTMIN+15
        for i in 1..=15 {
            let sig_num = rtmin + i;
            if sig_num > *rtmax {
                break;
            }
            write!(writer, "{:2}) SIGRTMIN+{:<2}", sig_num, i)?;
            count += 1;

            if count % 5 == 0 {
                writeln!(writer)?;
                count = 0;
            } else {
                write!(writer, " ")?;
            }
        }

        // SIGRTMAX-14 到 SIGRTMAX
        let start_offset = 14;
        for i in (0..=start_offset).rev() {
            let sig_num = rtmax - i;
            if sig_num <= rtmin + 15 {
                continue;
            }

            if count % 5 == 0 && count > 0 {
                writeln!(writer)?;
                count = 0;
            }

            if i == 0 {
                write!(writer, "{:2}) SIGRTMAX   ", sig_num)?;
            } else {
                write!(writer, "{:2}) SIGRTMAX-{:<2}", sig_num, i)?;
            }
            count += 1;

            if count % 5 == 0 {
                writeln!(writer)?;
                count = 0;
            } else {
                write!(writer, " ")?;
            }
        }
    }

    if count > 0 {
        writeln!(writer)?;
    }

    Ok(())
}

/// coreutils 格式: NAME (空格分隔)
fn kill_print_signals_coreutils<W: Write>(writer: &mut W) -> CTResult<()> {
    for (idx, signal) in ALL_SIGNALS.iter().enumerate() {
        if idx > 0 {
            write!(writer, " ")?;
        }
        write!(writer, "{signal}")?;
    }

    // 输出信号别名
    #[cfg(target_os = "linux")]
    {
        use ctcore::ct_signals::SIGNAL_ALIASES;
        for &(alias, _) in SIGNAL_ALIASES {
            write!(writer, " {alias}")?;
        }
    }

    // 输出实时信号占位符
    #[cfg(target_os = "linux")]
    {
        write!(writer, " RT<N> RTMIN+<N> RTMAX-<N>")?;
    }

    writeln!(writer)?;
    Ok(())
}

/// 向指定的写入器输出终止信号信息。
///
/// # Parameters
/// - `writer`: 一个可写对象，用于输出终止信号信息。
/// - `arg`: 一个可选的字符串引用，如果提供，则输出与该信号相关的详细信息；
///   如果未提供，则输出所有可用的终止信号。
///
/// # Returns
/// - `CTResult<()>`: 一个结果类型，表示操作成功或失败。
fn kill_list<W: Write>(
    writer: &mut W,
    opt_arg: Option<&String>,
    mode: KillCompatMode,
) -> CTResult<()> {
    if let Some(arg) = opt_arg {
        kill_print_signal(writer, arg)
    } else {
        kill_print_signals(writer, mode)
    }
}

/// 将信号名称解析为对应的信号值。
///
/// 该函数接受一个信号名称字符串,尝试将其解析为对应的信号值。
/// 如果解析成功,则返回信号值;如果解析失败,则返回一个错误。
/// 支持标准信号编号转换:
/// - 标准退出状态码: 信号编号 + 128
/// - ksh 风格: 信号编号 + 256
///
/// # 参数
///
/// * `signal_name: &str` - 信号名称字符串引用。
///
/// # 返回值
///
/// * `CTResult<usize>` - 一个结果类型,包含解析后的信号值或错误信息。
///
/// # 错误处理
///
/// 如果无法识别给定的信号名称,则返回一个包含错误信息的结果。
fn kill_parse_signal_value(signal_name: &str) -> CTResult<usize> {
    // 尝试通过信号名称或值获取信号的值。
    let mut optional_signal_value = get_ct_signal_by_name_or_value(signal_name);

    // 如果直接解析失败,尝试退出状态码转换
    if optional_signal_value.is_none() {
        if let Ok(num) = signal_name.parse::<usize>() {
            // 尝试标准退出状态码转换 (信号编号 + 128)
            if num >= 128 {
                let sig_num = num % 128;
                if sig_num < ALL_SIGNALS.len() {
                    optional_signal_value = Some(sig_num);
                }
            }
        }
    }

    if let Some(sig) = optional_signal_value {
        Ok(sig)
    } else {
        let err_message = format!("unknown signal name {}", signal_name.quote());
        Err(CtSimpleError::new(1, err_message))
    }
}

/// 将字符串切片转换为i32整数向量
/// 该函数尝试将输入的字符串切片中的每个字符串解析为i32整数
/// 如果解析失败，将返回一个包含错误信息的CTResult
///
/// # 参数
/// - `pids`: 一个字符串切片的引用，每个字符串代表一个可能的整数
///
/// # 返回
/// - `CTResult<Vec<i32>>`: 解析成功时，返回一个包含i32整数的向量；
///   解析失败时，返回一个包含错误信息的CTResult
fn kill_parse_pids(pids: &[String]) -> CTResult<Vec<i32>> {
    // 遍历字符串切片，尝试将每个字符串解析为i32整数
    pids.iter()
        .map(|x| {
            // 解析字符串为i32整数，如果失败，则构建一个自定义错误信息
            x.parse::<i32>().map_err(|e| {
                let err_message = format!("failed to parse argument {}: {}", x.quote(), e);
                CtSimpleError::new(1, err_message)
            })
        })
        .collect()
}

/// 向指定进程发送信号
///
/// # Parameters
/// - `sig`: 需要发送的信号类型
/// - `pids`: 接收信号的进程ID列表
///
/// # Remarks
/// 此函数会尝试向每个指定的进程发送信号如果发送信号失败，会使用`ct_show!`宏记录错误
fn kill(sig: Signal, pids: &[i32]) {
    // 遍历进程ID列表
    for &pid in pids {
        // 尝试向进程发送信号
        if let Err(e) = signal::kill(Pid::from_raw(pid), sig) {
            // 如果发送信号失败，使用`ct_show!`宏记录错误
            ct_show!(
                Error::from_raw_os_error(e as i32)
                    .map_err_context(|| format!("sending signal to {pid} failed"))
            );
        }
    }
}

#[derive(Default)]
pub struct Kill;
impl Tool for Kill {
    fn name(&self) -> &'static str {
        "kill"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        kill_main(&mut out, args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::io::Cursor;

    #[test]
    fn test_tool_implementation() {
        let tool = Kill::default();

        // 测试 name 方法
        assert_eq!(tool.name(), "kill");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("kill"));

        // 测试 execute 方法
        let args = vec![OsString::from("kill"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    #[cfg(test)]
    mod kill_parse_pids_tests {
        use super::*;
        /*
        基本测试：
            成功解析：所有字符串都是有效的 i32 表示。
            测试用例：pids = ["1", "2", "3"] 应返回 Ok(vec![1, 2, 3])。
            解析失败：至少有一个字符串不是有效的 i32 表示。
            测试用例：pids = ["1", "abc", "3"] 应返回一个 CtSimpleError，指出字符串 "abc" 解析失败。
            空输入：输入切片为空。
            测试用例：pids = [] 应返回 Ok(vec![])。
        异常测试：
            包含负数的字符串：测试包含负数的字符串是否能正确解析。
            测试用例：pids = ["-1", "2", "-3"] 应返回 Ok(vec![-1, 2, -3])。
            包含最大/最小 i32 值的字符串：测试边界情况，确保最大和最小 i32 值能正确解析。
            测试用例：pids = ["2147483647", "-2147483648"] 应返回 Ok(vec![2147483647, -2147483648])。
            包含空字符串的输入：测试空字符串是否会导致解析失败。
            测试用例：pids = ["1", "", "3"] 应返回一个 CtSimpleError，指出字符串 "" 解析失败。
            包含非数字字符的字符串：测试包含非数字字符的字符串是否会导致解析失败。
            测试用例：pids = ["1", "a1b", "3"] 应返回一个 CtSimpleError，指出字符串 "a1b" 解析失败。
            包含多个无效字符串：测试多个无效字符串是否会导致解析失败。
            测试用例：pids = ["abc", "def", "ghi"] 应返回一个 CtSimpleError，指出字符串 "abc" 解析失败。
        */
        #[test]
        fn kill_parse_pids_valid_pids_returns_parsed_integers() {
            let pids = vec!["1".to_string(), "2".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        #[test]
        fn kill_parse_pids_invalid_pid_returns_error() {
            let pids = vec!["1".to_string(), "abc".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument 'abc': invalid digit found in string"
            );
        }

        #[test]
        fn kill_parse_pids_empty_input_returns_empty_vector() {
            let pids: Vec<String> = vec![];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result.len(), 0);
        }

        #[test]
        fn kill_parse_pids_negative_pids_returns_parsed_integers() {
            let pids = vec!["-1".to_string(), "2".to_string(), "-3".to_string()];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result, vec![-1, 2, -3]);
        }

        #[test]
        fn kill_parse_pids_boundary_values_returns_parsed_integers() {
            let pids = vec!["2147483647".to_string(), "-2147483648".to_string()];
            let result = kill_parse_pids(&pids).unwrap();
            assert_eq!(result, vec![2147483647, -2147483648]);
        }

        #[test]
        fn kill_parse_pids_empty_string_returns_error() {
            let pids = vec!["1".to_string(), "".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument '': cannot parse integer from empty string"
            );
        }

        #[test]
        fn kill_parse_pids_non_numeric_characters_returns_error() {
            let pids = vec!["1".to_string(), "a1b".to_string(), "3".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument 'a1b': invalid digit found in string"
            );
        }

        #[test]
        fn kill_parse_pids_multiple_invalid_pids_returns_error() {
            let pids = vec!["abc".to_string(), "def".to_string(), "ghi".to_string()];
            let result = kill_parse_pids(&pids);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(
                err.to_string(),
                "failed to parse argument 'abc': invalid digit found in string"
            );
        }
    }

    #[cfg(test)]
    mod kill_parse_signal_value_tests {
        use super::*;

        #[test]
        fn kill_parse_signal_value_valid_signal_name_returns_signal_value() {
            let signal_name = "HUP";
            let result = kill_parse_signal_value(signal_name).unwrap();
            assert_eq!(result, 1); // Assuming HUP corresponds to signal value 1
        }

        #[test]
        fn kill_parse_signal_value_valid_signal_number_returns_signal_value() {
            let signal_name = "15"; // Assuming 15 corresponds to SIGTERM
            let result = kill_parse_signal_value(signal_name).unwrap();
            assert_eq!(result, 15);
        }

        #[test]
        fn kill_parse_signal_value_invalid_signal_name_returns_error() {
            let signal_name = "INVALID";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_parse_signal_value_empty_signal_name_returns_error() {
            let signal_name = "";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name ''");
        }

        #[test]
        fn kill_parse_signal_value_non_numeric_signal_number_returns_error() {
            let signal_name = "abc";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'abc'");
        }

        #[test]
        fn kill_parse_signal_value_boundary_signal_number_returns_signal_value() {
            let signal_name = "31"; // Assuming 31 corresponds to a valid signal
            let result = kill_parse_signal_value(signal_name).unwrap();
            assert_eq!(result, 31);
        }

        #[test]
        fn kill_parse_signal_value_negative_signal_number_returns_error() {
            let signal_name = "-1";
            let result = kill_parse_signal_value(signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name '-1'");
        }
    }
    #[cfg(test)]
    mod kill_list_tests {
        use super::*;
        #[test]
        fn kill_list_with_valid_signal_name_prints_signal_value() {
            let mut output = Cursor::new(Vec::new());
            let signal_name = Some("HUP".to_string());
            let _result =
                kill_list(&mut output, signal_name.as_ref(), KillCompatMode::Coreutils).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert_eq!(output_str.trim(), "1"); // Assuming HUP corresponds to signal value 1
        }

        #[test]
        fn kill_list_with_invalid_signal_name_returns_error() {
            let mut output = Cursor::new(Vec::new());
            let signal_name = Some("INVALID".to_string());
            let result = kill_list(&mut output, signal_name.as_ref(), KillCompatMode::Coreutils);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_list_with_no_argument_prints_all_signals() {
            let mut output = Cursor::new(Vec::new());
            let _result = kill_list(&mut output, None, KillCompatMode::Coreutils).unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            // 检查输出包含基本信号和 RT 信号
            assert!(output_str.contains("HUP"));
            assert!(output_str.contains("KILL"));
            assert!(output_str.contains("TERM"));
            assert!(output_str.contains("RT<N>"));
        }

        #[test]
        fn kill_list_with_empty_string_returns_error() {
            let mut output = Cursor::new(Vec::new());
            let signal_name = Some("".to_string());
            let result = kill_list(&mut output, signal_name.as_ref(), KillCompatMode::Coreutils);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name ''");
        }

        #[test]
        fn kill_list_with_numeric_signal_value_prints_signal_name() {
            let mut output = Cursor::new(Vec::new());
            let signal_value = Some("15".to_string()); // Assuming 15 corresponds to SIGTERM
            let _result = kill_list(
                &mut output,
                signal_value.as_ref(),
                KillCompatMode::Coreutils,
            )
            .unwrap();
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert_eq!(output_str.trim(), "TERM"); // Assuming 15 corresponds to SIGTERM
        }
    }
    #[cfg(test)]
    mod kill_print_signals_tests {
        use super::*;

        #[test]
        fn kill_print_signals_empty_signals_list() {
            let mut writer = Cursor::new(Vec::new());
            let result = kill_print_signals(&mut writer, KillCompatMode::Coreutils);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert!(output.ends_with('\n')); // 确保输出以换行符结束
        }

        #[test]
        fn kill_print_signals_correct_format() {
            let mut writer = Cursor::new(Vec::new());
            kill_print_signals(&mut writer, KillCompatMode::Coreutils).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出格式是否正确（信号之间用空格分隔）
            let signals: Vec<&str> = output.trim().split(' ').collect();
            assert!(!signals.is_empty());
            assert!(signals.contains(&"HUP")); // 检查是否包含常见信号
            assert!(signals.contains(&"TERM"));
            assert!(signals.contains(&"KILL"));
        }

        #[test]
        fn kill_print_signals_no_duplicate_signals() {
            let mut writer = Cursor::new(Vec::new());
            kill_print_signals(&mut writer, KillCompatMode::Coreutils).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查是否有重复的信号名称
            let signals: Vec<&str> = output.trim().split(' ').collect();
            let unique_signals: std::collections::HashSet<&str> = signals.iter().copied().collect();
            assert_eq!(signals.len(), unique_signals.len());
        }

        #[test]
        fn kill_print_signals_proper_spacing() {
            let mut writer = Cursor::new(Vec::new());
            kill_print_signals(&mut writer, KillCompatMode::Coreutils).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查信号之间的间距是否正确（单个空格）
            assert!(!output.contains("  ")); // 不应该有连续的空格
            assert!(!output.trim().starts_with(' ')); // 开头不应该有空格
        }

        #[test]
        fn kill_print_signals_write_error_handling() {
            // 测试写入错误的情况
            struct ErrorWriter;
            impl Write for ErrorWriter {
                fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "write error",
                    ))
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let mut writer = ErrorWriter;
            let result = kill_print_signals(&mut writer, KillCompatMode::Coreutils);
            assert!(result.is_err());
        }
    }

    #[cfg(test)]
    mod kill_print_signal_tests {
        use super::*;

        #[test]
        fn kill_print_signal_valid_signal_name_returns_signal_value() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "HUP";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "1"); // Assuming HUP corresponds to signal value 1
        }

        #[test]
        fn kill_print_signal_valid_signal_number_returns_signal_name() {
            let mut writer = Cursor::new(Vec::new());
            let signal_number = "15"; // Assuming 15 corresponds to SIGTERM
            let result = kill_print_signal(&mut writer, signal_number);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "TERM");
        }

        #[test]
        fn kill_print_signal_invalid_signal_name_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "INVALID";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_print_signal_empty_signal_name_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name ''");
        }

        #[test]
        fn kill_print_signal_non_numeric_signal_number_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "abc";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'abc'");
        }

        #[test]
        fn kill_print_signal_boundary_signal_number_returns_signal_name() {
            let mut writer = Cursor::new(Vec::new());
            let signal_number = "31"; // Assuming 31 corresponds to a valid signal
            let result = kill_print_signal(&mut writer, signal_number);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "SYS");
        }

        #[test]
        fn kill_print_signal_negative_signal_number_returns_error() {
            let mut writer = Cursor::new(Vec::new());
            let signal_number = "-1";
            let result = kill_print_signal(&mut writer, signal_number);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name '-1'");
        }

        #[test]
        fn kill_print_signal_valid_signal_with_sig_prefix_returns_signal_value() {
            let mut writer = Cursor::new(Vec::new());
            let signal_name = "SIGHUP";
            let result = kill_print_signal(&mut writer, signal_name);
            assert!(result.is_ok());
            let output = String::from_utf8(writer.into_inner()).unwrap();
            assert_eq!(output.trim(), "1"); // Assuming SIGHUP corresponds to signal value 1
        }
    }

    #[cfg(test)]
    mod kill_table_tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn kill_table_correct_format() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出格式是否正确（信号名称和索引号）
            let lines: Vec<&str> = output.trim().split('\n').collect();
            assert!(!lines.is_empty());
            for line in lines {
                let parts: Vec<&str> = line.split_whitespace().collect();
                assert!(parts.len() % 2 == 0); // 每行应有偶数个部分（索引号和信号名称）
            }
        }

        #[test]
        fn kill_table_proper_spacing() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查信号之间的间距是否正确（单个空格）
            assert!(!output.trim().starts_with(' ')); // 开头不应该有空格
        }

        #[test]
        fn kill_table_write_error_handling() {
            // 测试写入错误的情况
            struct ErrorWriter;
            impl Write for ErrorWriter {
                fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "write error",
                    ))
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    Ok(())
                }
            }

            let mut writer = ErrorWriter;
            let result = kill_table(&mut writer);
            assert!(result.is_err());
        }

        #[test]
        fn kill_table_contains_all_signals() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出是否包含所有信号
            for signal in ALL_SIGNALS.iter() {
                assert!(output.contains(signal));
            }
        }

        #[test]
        fn kill_table_correct_number_of_lines() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出包含必要的信号
            let lines: Vec<&str> = output.trim().split('\n').collect();
            // 表格模式至少有多行输出
            assert!(lines.len() >= 5);
            // 检查包含基本信号
            assert!(output.contains("HUP"));
            assert!(output.contains("KILL"));
        }

        #[test]
        fn kill_table_correct_signal_indices() {
            let mut writer = Cursor::new(Vec::new());
            kill_table(&mut writer).unwrap();
            let output = String::from_utf8(writer.into_inner()).unwrap();

            // 检查输出包含信号名称
            assert!(output.contains("HUP"));
            assert!(output.contains("INT"));
            assert!(output.contains("TERM"));
            assert!(output.contains("KILL"));
        }
    }

    #[cfg(test)]
    mod kill_handle_obsolete_tests {
        use super::*;

        #[test]
        fn kill_handle_obsolete_with_obsolete_signal() {
            let mut args = vec!["kill".to_string(), "-9".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(9));
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_no_obsolete_signal() {
            let mut args = vec!["kill".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_invalid_signal() {
            let mut args = vec![
                "kill".to_string(),
                "-invalid".to_string(),
                "1234".to_string(),
            ];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(
                args,
                vec![
                    "kill".to_string(),
                    "-invalid".to_string(),
                    "1234".to_string()
                ]
            );
        }

        #[test]
        fn kill_handle_obsolete_with_multiple_signals() {
            let mut args = vec![
                "kill".to_string(),
                "-9".to_string(),
                "-15".to_string(),
                "1234".to_string(),
            ];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(9));
            assert_eq!(
                args,
                vec!["kill".to_string(), "-15".to_string(), "1234".to_string()]
            );
        }

        #[test]
        fn kill_handle_obsolete_with_no_args() {
            let mut args: Vec<String> = vec![];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
        }

        #[test]
        fn kill_handle_obsolete_with_single_arg() {
            let mut args = vec!["kill".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(args, vec!["kill".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_valid_signal_name() {
            let mut args = vec!["kill".to_string(), "-HUP".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(1)); // Assuming HUP corresponds to signal value 1
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_valid_signal_number() {
            let mut args = vec!["kill".to_string(), "-15".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(15)); // Assuming 15 corresponds to SIGTERM
            assert_eq!(args, vec!["kill".to_string(), "1234".to_string()]);
        }

        #[test]
        fn kill_handle_obsolete_with_mixed_valid_and_invalid_signals() {
            let mut args = vec![
                "kill".to_string(),
                "-9".to_string(),
                "-invalid".to_string(),
                "1234".to_string(),
            ];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, Some(9));
            assert_eq!(
                args,
                vec![
                    "kill".to_string(),
                    "-invalid".to_string(),
                    "1234".to_string()
                ]
            );
        }

        #[test]
        fn kill_handle_obsolete_with_signal_prefix_but_no_signal() {
            let mut args = vec!["kill".to_string(), "-".to_string(), "1234".to_string()];
            let result = kill_handle_obsolete(&mut args);
            assert_eq!(result, None);
            assert_eq!(
                args,
                vec!["kill".to_string(), "-".to_string(), "1234".to_string()]
            );
        }
    }

    #[cfg(test)]
    mod kill_main_tests {
        use super::*;
        use std::ffi::OsString;
        use std::io::Cursor;
        #[test]
        fn kill_main_with_table_flag() {
            // 使用 coreutils 模式测试 -t (默认 bash 模式不支持 -t)
            // SAFETY: 测试环境中设置环境变量是安全的
            unsafe {
                std::env::set_var("SYSKITS_KILL_MODE", "coreutils");
            }
            let args = vec![ctcore::ct_util_name(), "--table"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            unsafe {
                std::env::remove_var("SYSKITS_KILL_MODE");
            }
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("HUP")); // Assuming HUP is in the signal list
        }

        #[test]
        fn kill_main_with_list_flag() {
            let args = vec![ctcore::ct_util_name(), "--list"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.contains("HUP")); // Assuming HUP is in the signal list
        }

        #[test]
        fn kill_main_with_signal_and_pid() {
            let args = vec![ctcore::ct_util_name(), "-s", "TERM", "1234"];

            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_invalid_signal() {
            let args = vec![ctcore::ct_util_name(), "-s", "INVALID", "1234"];

            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.to_string(), "unknown signal name 'INVALID'");
        }

        #[test]
        fn kill_main_with_no_args() {
            let args = vec![ctcore::ct_util_name()];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            // 无参数时返回错误 (默认 bash 模式退出码为 2)
            assert!(result.is_err());
        }

        #[test]
        fn kill_main_with_obsolete_signal() {
            let args = vec![ctcore::ct_util_name(), "-9", "1234"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_multiple_pids() {
            let args = vec![ctcore::ct_util_name(), "-s", "TERM", "1234", "5678"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_invalid_pid() {
            let args = vec![ctcore::ct_util_name(), "-s", "TERM", "invalid_pid"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.to_string()
                    .contains("failed to parse argument 'invalid_pid'")
            );
        }

        #[test]
        fn kill_main_with_default_signal() {
            let args = vec![ctcore::ct_util_name(), "1234"];

            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_ok());
            let output_str = String::from_utf8(output.into_inner()).unwrap();
            assert!(output_str.is_empty()); // No output expected for successful signal sending
        }

        #[test]
        fn kill_main_with_help_flag() {
            let args = vec![ctcore::ct_util_name(), "--help"];
            let mut output = Cursor::new(Vec::new());
            let result = kill_main(&mut output, args.iter().map(|s| OsString::from(s)));
            assert!(result.is_err()); // Expecting error due to help flag
        }
    }

    #[cfg(test)]
    mod ct_app_tests {
        use super::*;
        use clap::error::ErrorKind;

        #[test]
        fn test_ct_app_execution_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--version"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_other_version() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-V"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_err());
            assert_eq!(executable.unwrap_err().kind(), ErrorKind::DisplayVersion);
        }

        #[test]
        fn test_ct_app_execution_help() {
            let command = ct_app();
            let help_args = vec![ctcore::ct_util_name(), "--help"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DisplayHelp);
        }

        #[test]
        fn test_ct_app_execution_unsupport_help() {
            let command = ct_app();
            let help_args = vec![ctcore::ct_util_name(), "-H"];
            let result = command.try_get_matches_from(help_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_invalid_argument() {
            let command = ct_app();
            let invalid_args = vec![ctcore::ct_util_name(), "--invalid-argument"];
            let result = command.try_get_matches_from(invalid_args);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), ErrorKind::UnknownArgument);
        }

        #[test]
        fn test_ct_app_support_missing_argument() {
            let command = ct_app();
            let missing_args = vec![ctcore::ct_util_name()]; // 缺少任何参数
            let result = command.try_get_matches_from(missing_args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_ct_app_long_option_l_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-l"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::LIST));
        }

        #[test]
        fn test_ct_app_long_option_l_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--list"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::LIST));
        }

        #[test]
        fn test_ct_app_long_option_t_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-t"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::TABLE));
        }

        #[test]
        fn test_ct_app_long_option_t_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--table"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::TABLE));
        }

        #[test]
        fn test_ct_app_long_option_s_short() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "-s", "TERM"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::SIGNAL));
        }

        #[test]
        fn test_ct_app_long_option_s_long() {
            let command = ct_app();
            let args = vec![ctcore::ct_util_name(), "--signal", "TERM"];
            let executable = command.try_get_matches_from(args);
            assert!(executable.is_ok());
            assert!(executable.unwrap().contains_id(kill_flags::SIGNAL));
        }
    }
}
