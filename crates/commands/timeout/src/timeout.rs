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

//! 运行 timeout <命令>，若该命令在 <持续时间> 后仍在运行，则将其杀死。
//! timeout 退出码定义：
//! 如果命令超时且未设置 --preserve-status，程序退出的状态值将为 124。
//! 否则将使用所运行程序的退出状态值作为退出状态值。
//! 如果没有指定信号则默认使用 TERM 信号。TERM 信号在进程没有捕获此信号时将
//! 杀死进程。对于另一些进程可能需要使用 KILL (9)信号。因此信号无法被捕获，
//! 退出返回值将为 128+9 而非 124。

extern crate rust_i18n;
mod exit_status;

use crate::exit_status::ExitStatus;
use rust_i18n::t;
rust_i18n::i18n!("locales", fallback = "en-US");
use clap::{crate_version, Arg, ArgAction, Command};
use ctcore::ct_display::Quotable;
use ctcore::ct_error::{CTResult, CTsageError, CtSimpleError, UClapError};
use ctcore::ct_process::CtChildExt;
use ctcore::Tool;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::os::unix::process::ExitStatusExt;
use std::process::{self, Child, Stdio};
use std::time::Duration;
use sys_locale::get_locale;

#[cfg(unix)]
use ctcore::ct_signals::enable_pipe_errors;

use ctcore::{
    ct_show_error,
    ct_signals::{get_ct_signal_by_name_or_value, get_ct_signal_name_by_value},
};

use std::os::unix::process::CommandExt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
// 用于给 C 风格的信号处理器传递子进程状态
static CHILD_PID: AtomicI32 = AtomicI32::new(0);
static TIMEOUT_SIGNAL: AtomicI32 = AtomicI32::new(0);
static IS_FOREGROUND: AtomicBool = AtomicBool::new(false);
static ALARM_RECEIVED: AtomicBool = AtomicBool::new(false);

#[inline]
fn forward_signal_to_monitored_process(pid: i32, sig: i32, is_foreground: bool) {
    if pid <= 0 {
        return;
    }

    unsafe {
        // Always signal the direct child first.
        libc::kill(pid, sig);

        // In background mode we also signal the entire monitored process group.
        if !is_foreground {
            libc::kill(-pid, sig);
        }
    }
}

/// 专用于兼容 GNU 行为的 SIGALRM 处理器
extern "C" fn handle_sigalrm(_sig: libc::c_int) {
    ALARM_RECEIVED.store(true, Ordering::Relaxed);
    let pid = CHILD_PID.load(Ordering::Relaxed);
    let sig = TIMEOUT_SIGNAL.load(Ordering::Relaxed);
    let is_fg = IS_FOREGROUND.load(Ordering::Relaxed);
    forward_signal_to_monitored_process(pid, sig, is_fg);
    // 我们只负责发信号，然后立刻返回，让操作系统的 wait() 正常回收子进程。
}

/// 转发 timeout 自身收到的终止信号，避免子进程残留
extern "C" fn handle_forwarding_signal(sig: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::Relaxed);
    let is_fg = IS_FOREGROUND.load(Ordering::Relaxed);
    forward_signal_to_monitored_process(pid, sig, is_fg);
}

pub mod timeout_flags {
    pub static TIMEOUT_FOREGROUND: &str = "foreground";
    pub static TIMEOUT_KILL_AFTER: &str = "kill-after";
    pub static TIMEOUT_SIGNAL: &str = "signal";
    pub static TIMEOUT_PRESERVE_STATUS: &str = "preserve-status";
    pub static TIMEOUT_VERBOSE: &str = "verbose";

    // Positional args.
    pub static TIMEOUT_DURATION: &str = "duration";
    pub static TIMEOUT_COMMAND: &str = "command";
}

struct TimeoutFlags {
    is_foreground: bool,
    kill_after: Option<Duration>,
    signal: usize,
    duration: Duration,
    is_preserve_status: bool,
    is_verbose: bool,

    command: Vec<String>,
}

impl TimeoutFlags {
    // 根据提供的命令行参数创建一个新的CTimeout实例
    fn new(timeout_flags: &clap::ArgMatches) -> CTResult<Self> {
        let signal = match timeout_flags.get_one::<String>(timeout_flags::TIMEOUT_SIGNAL) {
            Some(signal_) => {
                let signal_result = get_ct_signal_by_name_or_value(signal_);
                match signal_result {
                    None => {
                        return Err(CTsageError::new(
                            ExitStatus::TimeoutFailed.into(),
                            format!("{}: invalid signal", signal_.quote()),
                        ));
                    }
                    Some(signal_value) => signal_value,
                }
            }
            _ => ctcore::ct_signals::get_ct_signal_by_name_or_value("TERM").unwrap(),
        };

        let kill_after = match timeout_flags.get_one::<String>(timeout_flags::TIMEOUT_KILL_AFTER) {
            None => None,
            Some(kill_after_str) => match ctcore::ct_parse_time::ct_from_str(kill_after_str) {
                Ok(mut k) => {
                    // 处理正数下溢出。如果解析为0，但输入包含数字(1-9)，说明是极端微小的浮点数，向上取整为1纳秒
                    if k.is_zero() && kill_after_str.chars().any(|c| c >= '1' && c <= '9') {
                        k = Duration::from_nanos(1);
                    }
                    Some(k)
                }
                Err(err) => {
                    return Err(CTsageError::new(ExitStatus::TimeoutFailed.into(), err));
                }
            },
        };

        let duration_str = timeout_flags
            .get_one::<String>(timeout_flags::TIMEOUT_DURATION)
            .unwrap();
        let mut duration = match ctcore::ct_parse_time::ct_from_str(duration_str) {
            Ok(duration) => duration,
            Err(err) => {
                return Err(CTsageError::new(ExitStatus::TimeoutFailed.into(), err));
            }
        };

        // 同理，处理主超时的正数下溢出。这使得 1e-10000 变成 1纳秒而不是无限等待
        if duration.is_zero() && duration_str.chars().any(|c| c >= '1' && c <= '9') {
            duration = Duration::from_nanos(1);
        }

        let is_preserve_status = timeout_flags.get_flag(timeout_flags::TIMEOUT_PRESERVE_STATUS);
        let is_foreground = timeout_flags.get_flag(timeout_flags::TIMEOUT_FOREGROUND);
        let is_verbose = timeout_flags.get_flag(timeout_flags::TIMEOUT_VERBOSE);

        let command = timeout_flags
            .get_many::<String>(timeout_flags::TIMEOUT_COMMAND)
            .unwrap()
            .map(String::from)
            .collect::<Vec<_>>();

        Ok(Self {
            is_foreground,
            kill_after,
            signal,
            duration,
            is_preserve_status,
            is_verbose,
            command,
        })
    }
}

pub fn timeout_main(args: impl ctcore::Args) -> CTResult<()> {
    let lang_code = get_locale().unwrap_or_else(|| String::from("en-US"));
    rust_i18n::set_locale(&lang_code);
    // 尝试解析命令行参数，如果失败则返回错误码125
    let matches = ct_app().try_get_matches_from(args).with_exit_code(125)?;

    // 从命令行参数创建 TimeoutFlags 实例
    let flags = TimeoutFlags::new(&matches)?;

    // 执行超时命令
    timeout(&flags)
}

pub fn ct_app() -> Command {
    let utility_name = ctcore::ct_util_name();
    let command_version = crate_version!();
    let application_info = t!("timeout.about");
    let usage_description = t!("timeout.usage");
    let args = vec![
        Arg::new(timeout_flags::TIMEOUT_FOREGROUND)
            .long(timeout_flags::TIMEOUT_FOREGROUND)
            .help(
                "when not running timeout directly from a shell prompt, allow \
                COMMAND to read from the TTY and get TTY signals; in this mode, \
                children of COMMAND will not be timed out",
            )
            .action(ArgAction::SetTrue),
        Arg::new(timeout_flags::TIMEOUT_KILL_AFTER)
            .long(timeout_flags::TIMEOUT_KILL_AFTER)
            .short('k')
            .value_name("DURATION")
            .help(
                "also send a KILL signal if COMMAND is still running this long \
                after the initial signal was sent",
            ),
        Arg::new(timeout_flags::TIMEOUT_PRESERVE_STATUS)
            .long(timeout_flags::TIMEOUT_PRESERVE_STATUS)
            .help(t!("timeout.clap.timeout_preserve_status"))
            .action(ArgAction::SetTrue),
        Arg::new(timeout_flags::TIMEOUT_SIGNAL)
            .short('s')
            .long(timeout_flags::TIMEOUT_SIGNAL)
            .value_name("SIGNAL")
            .help(
                "specify the signal to be sent on timeout; SIGNAL may be a name like \
                'HUP' or a number; see 'kill -l' for a list of signals",
            ),
        Arg::new(timeout_flags::TIMEOUT_VERBOSE)
            .short('v')
            .long(timeout_flags::TIMEOUT_VERBOSE)
            .help(t!("timeout.clap.timeout_verbose"))
            .action(ArgAction::SetTrue),
        Arg::new(timeout_flags::TIMEOUT_DURATION)
            .value_name("DURATION")
            .required(true)
            .help(t!("timeout.clap.timeout_duration")),
        Arg::new(timeout_flags::TIMEOUT_COMMAND)
            .value_name("COMMAND [ARGS]")
            .required(true)
            .action(ArgAction::Append)
            .value_hint(clap::ValueHint::CommandName)
            .help(t!("timeout.clap.timeout_command")),
    ];

    Command::new(utility_name)
        .version(command_version)
        .about(application_info)
        .override_usage(usage_description)
        .trailing_var_arg(true)
        .infer_long_args(true)
        .args(&args)
}

/// 移除可能使等待子进程退出码失败的预存在SIGCHLD处理程序
fn timeout_unblock_sigchld() {
    unsafe {
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGCHLD,
            nix::sys::signal::SigHandler::SigDfl,
        )
        .unwrap();
    }
}

/// 在详细模式下报告超时并发送信号给指定命令
fn timeout_report_if_verbose(signal: usize, cmd: &str, is_verbose: bool) {
    if is_verbose {
        let s = get_ct_signal_name_by_value(signal).unwrap();
        ct_show_error!("sending signal {} to command {}", s, cmd.quote());
    }
}

/// 发送信号给一个带有超时的进程，处理前台和后台进程
fn timeout_send_signal(process: &mut Child, signal: usize, is_foreground: bool) {
    unsafe {
        if is_foreground {
            libc::kill(process.id() as libc::c_int, signal as libc::c_int);
        } else {
            libc::kill(-(process.id() as libc::c_int), signal as libc::c_int);
        }
    }
}

/// 根据指定的超时标志设置进程的超时行为
///
/// 此函数负责根据`TimeoutFlags`结构体中的标志，设置进程的超时行为。
/// 如果进程不是在前台运行，则尝试将其置于后台。此外，它还会处理与Unix系统相关的管道错误，
/// 并尝试执行指定的命令。如果命令执行失败，它会根据错误类型返回相应的错误代码。
/// 最后，它会设置信号以防止子进程阻塞，并处理进程的超时逻辑。
///
/// # 参数
/// * `flags`: `&TimeoutFlags` - 包含超时设置和命令信息的引用
///
/// # 返回
/// * `CTResult<()>` - 一个结果类型，包装了可能的错误
/// 根据指定的超时标志设置进程的超时行为
fn timeout(flags: &TimeoutFlags) -> CTResult<()> {
    #[cfg(unix)]
    enable_pipe_errors()?;

    let mut cmd = process::Command::new(&flags.command[0]);
    cmd.args(&flags.command[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let is_foreground = flags.is_foreground;

    // 子进程的预处理 (pre_exec)
    unsafe {
        cmd.pre_exec(move || {
            // 1. 如果不是前台模式，自立门户 (新建进程组)
            if !is_foreground {
                libc::setpgid(0, 0);
            }

            // 2. 信号洗礼：清除 Shell 强加给后台任务的 SIG_IGN 魔法
            // 确保子进程 (如脚本) 可以正常使用 trap 捕获这些信号！
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGALRM, libc::SIG_DFL);
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);

            Ok(())
        });
    }

    let process = &mut cmd.spawn().map_err(|err| {
        let status = if err.kind() == ErrorKind::NotFound {
            ExitStatus::CommandNotFound
        } else {
            ExitStatus::CommandNotExecutable
        };
        CtSimpleError::new(status.into(), format!("failed to execute process: {err}"))
    })?;

    timeout_unblock_sigchld();
    handle_process_timeout(process, flags)
}

/// 处理进程超时逻辑
///
/// 此函数用于在指定时间内监控一个进程的运行状态。如果进程在规定时间内正常退出，
/// 或者运行时间超过规定时间（超时），此函数将进行相应处理。
///
/// # 参数
/// - `process`: &mut Child - 一个可变引用，指向需要监控的子进程。
/// - `flags`: &TimeoutFlags - 一个不可变引用，包含超时相关的配置，如持续时间、信号等。
///
/// # 返回值
/// - `CTResult<()>` - 一个结果类型，表示处理是否成功。如果进程正常退出或被处理，
///   则返回Ok(())；否则返回一个错误状态。
/// 处理进程超时逻辑
fn handle_process_timeout(process: &mut Child, flags: &TimeoutFlags) -> CTResult<()> {
    CHILD_PID.store(process.id() as i32, Ordering::Relaxed);
    TIMEOUT_SIGNAL.store(flags.signal as i32, Ordering::Relaxed);
    IS_FOREGROUND.store(flags.is_foreground, Ordering::Relaxed);
    ALARM_RECEIVED.store(false, Ordering::Relaxed);

    unsafe {
        let alarm_sa = nix::sys::signal::SigAction::new(
            nix::sys::signal::SigHandler::Handler(handle_sigalrm),
            nix::sys::signal::SaFlags::SA_RESTART,
            nix::sys::signal::SigSet::empty(),
        );
        let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGALRM, &alarm_sa);

        let forwarding_sa = nix::sys::signal::SigAction::new(
            nix::sys::signal::SigHandler::Handler(handle_forwarding_signal),
            nix::sys::signal::SaFlags::SA_RESTART,
            nix::sys::signal::SigSet::empty(),
        );

        for sig in [
            nix::sys::signal::Signal::SIGINT,
            nix::sys::signal::Signal::SIGQUIT,
            nix::sys::signal::Signal::SIGHUP,
            nix::sys::signal::Signal::SIGTERM,
        ] {
            let _ = nix::sys::signal::sigaction(sig, &forwarding_sa);
        }

        if let Ok(custom_signal) = nix::sys::signal::Signal::try_from(flags.signal as i32) {
            if custom_signal != nix::sys::signal::Signal::SIGALRM {
                let _ = nix::sys::signal::sigaction(custom_signal, &forwarding_sa);
            }
        }
    }

    let wait_result = if flags.duration.is_zero() {
        process.wait().map(|status| Some(status))
    } else {
        process.wait_or_timeout(flags.duration)
    };

    let result = if ALARM_RECEIVED.load(Ordering::Relaxed) {
        // 阻塞等待子进程执行完 trap 清理逻辑并正式退出
        let _ = process.wait();
        Err(ExitStatus::CommandTimedOut.into()) // 等子进程死透了，我们再优雅返回 124
    } else {
        match wait_result {
            Ok(Some(status)) => {
                // 进程在超时前结束 (未超时)
                if let Some(signal) = status.signal() {
                    Err(ExitStatus::SignalTerminated(signal).into())
                } else {
                    let exit_code = status.code().unwrap_or(0);
                    if exit_code == 0 {
                        Ok(())
                    } else {
                        Err(exit_code.into())
                    }
                }
            }
            Ok(None) => {
                // 进程超时，发送信号
                timeout_report_if_verbose(flags.signal, &flags.command[0], flags.is_verbose);
                timeout_send_signal(process, flags.signal, flags.is_foreground);

                if flags.signal == get_ct_signal_by_name_or_value("KILL").unwrap() {
                    let _ = process.wait()?;
                    Err(ExitStatus::SignalTerminated(9).into())
                } else {
                    handle_timeout_exceeded(process, flags)
                }
            }
            Err(_) => Err(ExitStatus::TimeoutFailed.into()),
        }
    };

    CHILD_PID.store(0, Ordering::Relaxed);
    TIMEOUT_SIGNAL.store(0, Ordering::Relaxed);
    IS_FOREGROUND.store(false, Ordering::Relaxed);

    result
}

/// 处理超时情况
///
/// 当进程运行时间超过指定的超时限制时，调用此函数来处理超时情况。
/// 它首先报告超时（如果设置了详细模式），然后向进程发送超时信号，
/// 最后根据是否设置了kill_after参数来决定下一步的行为。
///
/// # 参数
///
/// - `process`: &mut Child - 对子进程的引用，用于发送信号。
/// - `flags`: &TimeoutFlags - 包含超时配置的引用，包括信号类型、命令、是否详细模式等。
///
/// # 返回
///
/// - `CTResult<()>` - 一个结果类型，表示操作是否成功。
/// 处理超时情况
fn handle_timeout_exceeded(process: &mut Child, flags: &TimeoutFlags) -> CTResult<()> {
    match flags.kill_after {
        None => {
            // 等待初始信号（如 TERM）的结果
            let status = process.wait()?;

            // 如果启用了 --preserve-status，必须透传子进程的真实死法状态
            if flags.is_preserve_status {
                if let Some(signal) = status.signal() {
                    return Err(ExitStatus::SignalTerminated(signal).into()); // 例如 143
                } else {
                    let code = status.code().unwrap_or(0);
                    if code == 0 {
                        return Ok(());
                    } else {
                        return Err(code.into());
                    }
                }
            }

            // 默认情况：由于超时触发，返回 124
            Err(ExitStatus::CommandTimedOut.into()) // 124
        }
        Some(kill_after) => {
            // 等待 kill_after 时间
            let wait_result = if kill_after.is_zero() {
                process.wait().map(|s| Some(s))
            } else {
                process.wait_or_timeout(kill_after)
            };

            match wait_result {
                Ok(Some(status)) => {
                    // 进程在触发 KILL 之前自己死掉了 (可能是被第一波信号干掉了)
                    if flags.is_preserve_status {
                        if let Some(signal) = status.signal() {
                            return Err(ExitStatus::SignalTerminated(signal).into());
                        } else {
                            let code = status.code().unwrap_or(0);
                            if code == 0 {
                                return Ok(());
                            } else {
                                return Err(code.into());
                            }
                        }
                    }
                    Err(ExitStatus::CommandTimedOut.into()) // 124
                }
                Ok(None) => {
                    // 触发了二次击杀！发送 KILL 信号
                    let kill_signal = get_ct_signal_by_name_or_value("KILL").unwrap();
                    timeout_report_if_verbose(kill_signal, &flags.command[0], flags.is_verbose);
                    timeout_send_signal(process, kill_signal, flags.is_foreground);

                    let _ = process.wait()?;

                    // GNU 规定，只要发射了 KILL 信号，退出码绝对是 137 (128+9)
                    // 即使没有开启 preserve-status，也不能返回 124！
                    Err(ExitStatus::SignalTerminated(9).into()) // 137
                }
                Err(_) => Err(ExitStatus::TimeoutFailed.into()),
            }
        }
    }
}

#[derive(Default)]
pub struct Timeout;
impl Tool for Timeout {
    fn name(&self) -> &'static str {
        "timeout"
    }

    fn command(&self) -> Command {
        ct_app()
    }

    fn execute(&self, args: &[OsString]) -> CTResult<()> {
        timeout_main(args.iter().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    // 新增：测试 Tool trait 的基本实现
    #[test]
    fn test_tool_implementation() {
        let tool = Timeout;

        // 测试 name 方法
        assert_eq!(tool.name(), "timeout");

        // 测试 command 方法
        let command = tool.command();
        assert!(command.get_name().contains("timeout"));

        // 测试 execute 方法
        let args = vec![OsString::from("timeout"), OsString::from("--version")];
        assert!(tool.execute(&args).is_err());
    }

    mod timeout_flags_tests {
        use super::*;

        #[test]
        fn test_flags_basic() {
            let args = vec![ctcore::ct_util_name(), "5s", "sleep", "1"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = TimeoutFlags::new(&matches).unwrap();

            assert!(!flags.is_foreground);
            assert!(flags.kill_after.is_none());
            assert_eq!(flags.duration, Duration::from_secs(5));
            assert!(!flags.is_preserve_status);
            assert!(!flags.is_verbose);
            assert_eq!(flags.command, vec!["sleep".to_string(), "1".to_string()]);
        }

        #[test]
        fn test_flags_with_signal() {
            let args = vec![ctcore::ct_util_name(), "--signal=KILL", "5s", "yes"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = TimeoutFlags::new(&matches).unwrap();
            assert_eq!(
                flags.signal,
                get_ct_signal_by_name_or_value("KILL").unwrap()
            );
        }

        #[test]
        fn test_flags_with_kill_after() {
            let args = vec![ctcore::ct_util_name(), "-k", "2s", "5s", "sleep", "5"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = TimeoutFlags::new(&matches).unwrap();
            assert_eq!(flags.kill_after, Some(Duration::from_secs(2)));
        }

        #[test]
        fn test_flags_with_preserve_status() {
            let args = vec![ctcore::ct_util_name(), "--preserve-status", "1s", "false"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = TimeoutFlags::new(&matches).unwrap();
            assert!(flags.is_preserve_status);
        }

        #[test]
        fn test_flags_with_verbose() {
            let args = vec![ctcore::ct_util_name(), "--verbose", "1s", "true"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let flags = TimeoutFlags::new(&matches).unwrap();
            assert!(flags.is_verbose);
        }

        #[test]
        fn test_flags_invalid_signal() {
            let args = vec![
                ctcore::ct_util_name(),
                "--signal=INVALID",
                "5s",
                "sleep",
                "10",
            ];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = TimeoutFlags::new(&matches);
            assert!(result.is_err());
        }

        #[test]
        fn test_flags_invalid_duration() {
            let args = vec![ctcore::ct_util_name(), "invalid", "sleep", "1"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = TimeoutFlags::new(&matches);
            assert!(result.is_err());
        }

        #[test]
        fn test_flags_invalid_kill_after() {
            let args = vec![ctcore::ct_util_name(), "-k", "invalid", "5s", "sleep", "10"];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = TimeoutFlags::new(&matches);
            assert!(result.is_err());
        }
    }

    mod timeout_execution_tests {
        use super::*;
        use std::fs;
        use std::os::unix::process::CommandExt;
        use std::process::Command;
        use std::thread;

        #[test]
        fn test_timeout_normal_exit() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: None,
                signal: get_ct_signal_by_name_or_value("TERM").unwrap(),
                duration: Duration::from_secs(1),
                is_preserve_status: false,
                is_verbose: false,
                command: vec!["true".to_string()],
            });
            // 进程在超时前正常结束，应该返回成功
            assert!(result.is_ok());
        }

        #[test]
        fn test_timeout_command_not_found() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: None,
                signal: get_ct_signal_by_name_or_value("TERM").unwrap(),
                duration: Duration::from_secs(1),
                is_preserve_status: false,
                is_verbose: false,
                command: vec!["nonexistent_command".to_string()],
            });
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 127);
        }

        #[test]
        fn test_timeout_with_preserve_status() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: None,
                signal: get_ct_signal_by_name_or_value("TERM").unwrap(),
                duration: Duration::from_secs(1), // 增加超时时间以确保false命令能正常退出
                is_preserve_status: true,
                is_verbose: false,
                command: vec!["false".to_string()],
            });
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }

        #[test]
        fn test_timeout_normal_exit_with_preserve_status() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: None,
                signal: get_ct_signal_by_name_or_value("TERM").unwrap(),
                duration: Duration::from_secs(1),
                is_preserve_status: true,
                is_verbose: false,
                command: vec!["true".to_string()],
            });
            // 进程正常退出（退出码0），preserve_status模式下应该返回成功
            assert!(result.is_ok());
        }

        #[test]
        fn test_timeout_actual_timeout() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: None,
                signal: get_ct_signal_by_name_or_value("TERM").unwrap(),
                duration: Duration::from_millis(100), // 很短的超时时间
                is_preserve_status: false,
                is_verbose: false,
                command: vec!["sleep".to_string(), "1".to_string()], // sleep 1秒
            });
            // 进程应该超时并返回124
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 124);
        }

        #[test]
        fn test_timeout_with_kill_after() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: Some(Duration::from_millis(50)), // 50ms后发送KILL信号
                signal: get_ct_signal_by_name_or_value("TERM").unwrap(),
                duration: Duration::from_millis(100), // 100ms超时
                is_preserve_status: false,
                is_verbose: false,
                command: vec!["sleep".to_string(), "1".to_string()], // sleep 1秒
            });
            // 进程应该超时并返回124（兼容性要求）
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 124);
        }

        #[test]
        fn test_timeout_with_kill_after_and_signal_zero() {
            let result = timeout(&TimeoutFlags {
                is_foreground: false,
                kill_after: Some(Duration::from_millis(50)),
                signal: get_ct_signal_by_name_or_value("0").unwrap(),
                duration: Duration::from_millis(100),
                is_preserve_status: false,
                is_verbose: false,
                command: vec!["sleep".to_string(), "1".to_string()],
            });
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 137);
        }

        #[test]
        fn test_forward_signal_terminates_background_group() {
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }

            let mut child = cmd.spawn().unwrap();
            forward_signal_to_monitored_process(child.id() as i32, libc::SIGTERM, false);

            let status = child.wait_or_timeout(Duration::from_secs(2)).unwrap();
            if status.is_none() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child process was not terminated by forwarded SIGTERM");
            }

            assert_eq!(status.unwrap().signal(), Some(libc::SIGTERM));
        }

        #[test]
        fn test_forward_signal_stop_does_not_force_continue() {
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }

            let mut child = cmd.spawn().unwrap();
            forward_signal_to_monitored_process(child.id() as i32, libc::SIGSTOP, false);
            thread::sleep(Duration::from_millis(100));

            let status =
                fs::read_to_string(format!("/proc/{}/status", child.id())).expect("read status");
            let state_line = status
                .lines()
                .find(|line| line.starts_with("State:"))
                .expect("state line exists");
            assert!(
                state_line.contains("\tT"),
                "unexpected process state: {state_line}"
            );

            unsafe {
                libc::kill(child.id() as i32, libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }

    mod timeout_main_tests {
        use super::*;

        #[test]
        fn test_main_normal_timeout() {
            let args = [ctcore::ct_util_name(), "1s", "sleep", "2"];
            let result = timeout_main(args.iter().map(OsString::from));
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 124);
        }

        #[test]
        fn test_main_command_exits() {
            let args = [ctcore::ct_util_name(), "2s", "true"];
            let result = timeout_main(args.iter().map(OsString::from));
            // 进程在超时前正常结束，应该返回成功
            assert!(result.is_ok());
        }

        #[test]
        fn test_main_command_with_nonzero_exit() {
            let args = [ctcore::ct_util_name(), "--preserve-status", "2s", "false"];
            let result = timeout_main(args.iter().map(OsString::from));
            // 进程正常退出但返回非零退出码，preserve-status模式下应该保留原始退出码
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().code(), 1);
        }
    }

    mod ct_app_tests {
        use super::*;

        #[test]
        fn test_app_all_options() {
            let args = vec![
                ctcore::ct_util_name(),
                "--foreground",
                "-k",
                "2s",
                "--preserve-status",
                "--signal=TERM",
                "--verbose",
                "5s",
                "sleep",
                "5",
            ];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_app_minimal_options() {
            let args = vec![ctcore::ct_util_name(), "5s", "sleep", "1"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_app_invalid_options() {
            let args = vec![
                ctcore::ct_util_name(),
                "--invalid-option",
                "5s",
                "sleep",
                "1",
            ];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
        }

        #[test]
        fn test_app_missing_duration() {
            let args = vec![ctcore::ct_util_name(), "--signal=TERM", "command", "arg1"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_app_missing_duration_with_option() {
            let args = vec![ctcore::ct_util_name(), "--foreground", "command", "arg1"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_ok());
        }

        #[test]
        fn test_app_duration_validation() {
            let args = vec![
                ctcore::ct_util_name(),
                "--foreground",
                "not_a_duration",
                "command",
            ];
            let matches = ct_app().try_get_matches_from(args).unwrap();
            let result = TimeoutFlags::new(&matches);
            assert!(result.is_err());
        }

        #[test]
        fn test_app_missing_command() {
            let args = vec![ctcore::ct_util_name(), "5s"];
            let result = ct_app().try_get_matches_from(args);
            assert!(result.is_err());
        }
    }
}
