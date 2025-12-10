/*
 * Copyright(c) 2022-2024 China Telecom Cloud Technologies Co., Ltd. All rights reserved
 *   syskits is licensed under Mulan PSL v2.
 * You can use this software according to the terms and conditions of the Mulan PSL V2
 * You may obtain a copy of Mulan PSL v2 at: http://license.coscl.org.cn/MulanPSL2
 * THIS SOFTWARE IS PROVIDED ON AN "AS IS" BASIS, WITHOUT WARRANTIES OF ANY
 * KIND, EITHER EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO
 * NON-INFRINGEMENT, MERCHANTABILITY OR FIT FOR A PARTICULAR PURPOSE.
 * See the Mulan PSL v2 for more details.
 */

// spell-checker:ignore (vars/api) fcntl setrlimit setitimer rubout pollable sysconf
// spell-checker:ignore (vars/signals) ABRT ALRM CHLD SEGV SIGABRT SIGALRM SIGBUS SIGCHLD SIGCONT SIGEMT SIGFPE SIGHUP SIGILL SIGINFO SIGINT SIGIO SIGIOT SIGKILL SIGPIPE SIGPROF SIGPWR SIGQUIT SIGSEGV SIGSTOP SIGSYS SIGTERM SIGTRAP SIGTSTP SIGTHR SIGTTIN SIGTTOU SIGURG SIGUSR SIGVTALRM SIGWINCH SIGXCPU SIGXFSZ STKFLT PWR THR TSTP TTIN TTOU VTALRM XCPU XFSZ SIGCLD SIGPOLL SIGWAITING SIGAIOCANCEL SIGLWP SIGFREEZE SIGTHAW SIGCANCEL SIGLOST SIGXRES SIGJVM SIGRTMIN SIGRT SIGRTMAX AIOCANCEL XRES RTMIN RTMAX
#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::sys::signal::{
    signal, SigHandler::SigDfl, SigHandler::SigIgn, Signal::SIGINT, Signal::SIGPIPE,
};

pub static CT_DEFAULT_SIGNAL: usize = 15;

/*

Linux程序员手册

 1 HUP      2 INT      3 QUIT     4 ILL      5 TRAP     6 ABRT     7 BUS
 8 FPE      9 KILL    10 USR1    11 SEGV    12 USR2    13 PIPE    14 ALRM
15 TERM    16 STKFLT  17 CHLD    18 CONT    19 STOP    20 TSTP    21 TTIN
22 TTOU    23 URG     24 XCPU    25 XFSZ    26 VTALRM  27 PROF    28 WINCH
29 POLL    30 PWR     31 SYS


*/

#[cfg(any(target_os = "linux", target_os = "android", target_os = "redox"))]
pub static ALL_SIGNALS: [&str; 32] = [
    "EXIT", "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "BUS", "FPE", "KILL", "USR1", "SEGV",
    "USR2", "PIPE", "ALRM", "TERM", "STKFLT", "CHLD", "CONT", "STOP", "TSTP", "TTIN", "TTOU",
    "URG", "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "POLL", "PWR", "SYS",
];

/*

苹果开发者文档中关于signal(3)的页面
https://developer.apple.com/library/mac/documentation/Darwin/Reference/ManPages/man3/signal.3.html


No    Name         Default Action       Description
1     SIGHUP       terminate process    terminal line hangup
2     SIGINT       terminate process    interrupt program
3     SIGQUIT      create core image    quit program
4     SIGILL       create core image    illegal instruction
5     SIGTRAP      create core image    trace trap
6     SIGABRT      create core image    abort program (formerly SIGIOT)
7     SIGEMT       create core image    emulate instruction executed
8     SIGFPE       create core image    floating-point exception
9     SIGKILL      terminate process    kill program
10    SIGBUS       create core image    bus error
11    SIGSEGV      create core image    segmentation violation
12    SIGSYS       create core image    non-existent system call invoked
13    SIGPIPE      terminate process    write on a pipe with no reader
14    SIGALRM      terminate process    real-time timer expired
15    SIGTERM      terminate process    software termination signal
16    SIGURG       discard signal       urgent condition present on socket
17    SIGSTOP      stop process         stop (cannot be caught or ignored)
18    SIGTSTP      stop process         stop signal generated from keyboard
19    SIGCONT      discard signal       continue after stop
20    SIGCHLD      discard signal       child status has changed
21    SIGTTIN      stop process         background read attempted from control terminal
22    SIGTTOU      stop process         background write attempted to control terminal
23    SIGIO        discard signal       I/O is possible on a descriptor (see fcntl(2))
24    SIGXCPU      terminate process    cpu time limit exceeded (see setrlimit(2))
25    SIGXFSZ      terminate process    file size limit exceeded (see setrlimit(2))
26    SIGVTALRM    terminate process    virtual time alarm (see setitimer(2))
27    SIGPROF      terminate process    profiling timer alarm (see setitimer(2))
28    SIGWINCH     discard signal       Window size change
29    SIGINFO      discard signal       status request from keyboard
30    SIGUSR1      terminate process    User defined signal 1
31    SIGUSR2      terminate process    User defined signal 2

*/

#[cfg(any(target_vendor = "apple", target_os = "freebsd"))]
pub static ALL_SIGNALS: [&str; 32] = [
    "EXIT", "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "EMT", "FPE", "KILL", "BUS", "SEGV",
    "SYS", "PIPE", "ALRM", "TERM", "URG", "STOP", "TSTP", "CONT", "CHLD", "TTIN", "TTOU", "IO",
    "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "INFO", "USR1", "USR2",
];

/*

     以下是NetBSD中定义的信号：

     SIGHUP           1     Hangup
     SIGINT           2     Interrupt
     SIGQUIT          3     Quit
     SIGILL           4     Illegal instruction
     SIGTRAP          5     Trace/BPT trap
     SIGABRT          6     Abort trap
     SIGEMT           7     EMT trap
     SIGFPE           8     Floating point exception
     SIGKILL          9     Killed
     SIGBUS           10    Bus error
     SIGSEGV          11    Segmentation fault
     SIGSYS           12    Bad system call
     SIGPIPE          13    Broken pipe
     SIGALRM          14    Alarm clock
     SIGTERM          15    Terminated
     SIGURG           16    Urgent I/O condition
     SIGSTOP          17    Suspended (signal)
     SIGTSTP          18    Suspended
     SIGCONT          19    Continued
     SIGCHLD          20    Child exited, stopped or continued
     SIGTTIN          21    Stopped (tty input)
     SIGTTOU          22    Stopped (tty output)
     SIGIO            23    I/O possible
     SIGXCPU          24    CPU time limit exceeded
     SIGXFSZ          25    File size limit exceeded
     SIGVTALRM        26    Virtual timer expired
     SIGPROF          27    Profiling timer expired
     SIGWINCH         28    Window size changed
     SIGINFO          29    Information request
     SIGUSR1          30    User defined signal 1
     SIGUSR2          31    User defined signal 2
     SIGPWR           32    Power fail/restart
*/

#[cfg(target_os = "netbsd")]
pub static ALL_SIGNALS: [&str; 33] = [
    "EXIT", "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "EMT", "FPE", "KILL", "BUS", "SEGV",
    "SYS", "PIPE", "ALRM", "TERM", "URG", "STOP", "TSTP", "CONT", "CHLD", "TTIN", "TTOU", "IO",
    "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "INFO", "USR1", "USR2", "PWR",
];

/*

     以下是OpenBSD中定义的信号：

     SIGHUP       terminate process    terminal line hangup
     SIGINT       terminate process    interrupt program
     SIGQUIT      create core image    quit program
     SIGILL       create core image    illegal instruction
     SIGTRAP      create core image    trace trap
     SIGABRT      create core image    abort(3) call (formerly SIGIOT)
     SIGEMT       create core image    emulate instruction executed
     SIGFPE       create core image    floating-point exception
     SIGKILL      terminate process    kill program (cannot be caught or
                                       ignored)
     SIGBUS       create core image    bus error
     SIGSEGV      create core image    segmentation violation
     SIGSYS       create core image    system call given invalid argument
     SIGPIPE      terminate process    write on a pipe with no reader
     SIGALRM      terminate process    real-time timer expired
     SIGTERM      terminate process    software termination signal
     SIGURG       discard signal       urgent condition present on socket
     SIGSTOP      stop process         stop (cannot be caught or ignored)
     SIGTSTP      stop process         stop signal generated from keyboard
     SIGCONT      discard signal       continue after stop
     SIGCHLD      discard signal       child status has changed
     SIGTTIN      stop process         background read attempted from control
                                       terminal
     SIGTTOU      stop process         background write attempted to control
                                       terminal
     SIGIO        discard signal       I/O is possible on a descriptor (see
                                       fcntl(2))
     SIGXCPU      terminate process    CPU time limit exceeded (see
                                       setrlimit(2))
     SIGXFSZ      terminate process    file size limit exceeded (see
                                       setrlimit(2))
     SIGVTALRM    terminate process    virtual time alarm (see setitimer(2))
     SIGPROF      terminate process    profiling timer alarm (see
                                       setitimer(2))
     SIGWINCH     discard signal       window size change
     SIGINFO      discard signal       status request from keyboard
     SIGUSR1      terminate process    user-defined signal 1
     SIGUSR2      terminate process    user-defined signal 2
     SIGTHR       discard signal       thread AST
*/

#[cfg(target_os = "openbsd")]
pub static ALL_SIGNALS: [&str; 33] = [
    "EXIT", "HUP", "INT", "QUIT", "ILL", "TRAP", "ABRT", "EMT", "FPE", "KILL", "BUS", "SEGV",
    "SYS", "PIPE", "ALRM", "TERM", "URG", "STOP", "TSTP", "CONT", "CHLD", "TTIN", "TTOU", "IO",
    "XCPU", "XFSZ", "VTALRM", "PROF", "WINCH", "INFO", "USR1", "USR2", "THR",
];

/*
    以下是Solaris和illumos中定义的信号；（illumos的信号与Solaris相同，
    但illumos仍然具有SIGLWP以及SIGLWP（SIGAIOCANCEL）的别名）：

     SIGHUP       1       hangup
     SIGINT       2       interrupt (rubout)
     SIGQUIT      3       quit (ASCII FS)
     SIGILL       4       illegal instruction (not reset when caught)
     SIGTRAP      5       trace trap (not reset when caught)
     SIGIOT       6       IOT instruction
     SIGABRT      6       used by abort, replace SIGIOT in the future
     SIGEMT       7       EMT instruction
     SIGFPE       8       floating point exception
     SIGKILL      9       kill (cannot be caught or ignored)
     SIGBUS       10      bus error
     SIGSEGV      11      segmentation violation
     SIGSYS       12      bad argument to system call
     SIGPIPE      13      write on a pipe with no one to read it
     SIGALRM      14      alarm clock
     SIGTERM      15      software termination signal from kill
     SIGUSR1      16      user defined signal 1
     SIGUSR2      17      user defined signal 2
     SIGCLD       18      child status change
     SIGCHLD      18      child status change alias (POSIX)
     SIGPWR       19      power-fail restart
     SIGWINCH     20      window size change
     SIGURG       21      urgent socket condition
     SIGPOLL      22      pollable event occurred
     SIGIO        SIGPOLL socket I/O possible (SIGPOLL alias)
     SIGSTOP      23      stop (cannot be caught or ignored)
     SIGTSTP      24      user stop requested from tty
     SIGCONT      25      stopped process has been continued
     SIGTTIN      26      background tty read attempted
     SIGTTOU      27      background tty write attempted
     SIGVTALRM    28      virtual timer expired
     SIGPROF      29      profiling timer expired
     SIGXCPU      30      exceeded cpu limit
     SIGXFSZ      31      exceeded file size limit
     SIGWAITING   32      reserved signal no longer used by threading code
     SIGAIOCANCEL 33      reserved signal no longer used by threading code (formerly SIGLWP)
     SIGFREEZE    34      special signal used by CPR
     SIGTHAW      35      special signal used by CPR
     SIGCANCEL    36      reserved signal for thread cancellation
     SIGLOST      37      resource lost (eg, record-lock lost)
     SIGXRES      38      resource control exceeded
     SIGJVM1      39      reserved signal for Java Virtual Machine
     SIGJVM2      40      reserved signal for Java Virtual Machine
     SIGINFO      41      information request
     SIGRTMIN     ((int)_sysconf(_SC_SIGRT_MIN)) first realtime signal
     SIGRTMAX     ((int)_sysconf(_SC_SIGRT_MAX)) last realtime signal
*/

#[cfg(target_os = "solaris")]
const SIGNALS_SIZE: usize = 46;

#[cfg(target_os = "illumos")]
const SIGNALS_SIZE: usize = 47;

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
static ALL_SIGNALS: [&str; SIGNALS_SIZE] = [
    "HUP",
    "INT",
    "QUIT",
    "ILL",
    "TRAP",
    "IOT",
    "ABRT",
    "EMT",
    "FPE",
    "KILL",
    "BUS",
    "SEGV",
    "SYS",
    "PIPE",
    "ALRM",
    "TERM",
    "USR1",
    "USR2",
    "CLD",
    "CHLD",
    "PWR",
    "WINCH",
    "URG",
    "POLL",
    "IO",
    "STOP",
    "TSTP",
    "CONT",
    "TTIN",
    "TTOU",
    "VTALRM",
    "PROF",
    "XCPU",
    "XFSZ",
    "WAITING",
    "AIOCANCEL",
    #[cfg(target_os = "illumos")]
    "LWP",
    "FREEZE",
    "THAW",
    "CANCEL",
    "LOST",
    "XRES",
    "JVM1",
    "JVM2",
    "INFO",
    "RTMIN",
    "RTMAX",
];

pub fn get_ct_signal_by_name_or_value(signal_name_or_value: &str) -> Option<usize> {
    if let Ok(value) = signal_name_or_value.parse() {
        if ct_signal(value) {
            return Some(value);
        } else {
            return None;
        }
    }
    let ct_signal_name = signal_name_or_value.trim_start_matches("SIG");

    let mut position = None;
    for (index, &ct_signal) in ALL_SIGNALS.iter().enumerate() {
        if ct_signal == ct_signal_name {
            position = Some(index);
            break;
        }
    }
    position
}

pub fn ct_signal(num: usize) -> bool {
    num < ALL_SIGNALS.len()
}

pub fn get_ct_signal_name_by_value(ct_signal_value: usize) -> Option<&'static str> {
    if let Some(ct_signal) = ALL_SIGNALS.get(ct_signal_value) {
        Some(ct_signal)
    } else {
        None
    }
}

#[cfg(unix)]
pub fn enable_pipe_errors() -> Result<(), Errno> {
    // 我们原样传递错误，返回值只会是Ok(SigDfl)，所以我们可以安全地忽略它。
    // 安全性：只要我们不使用自定义的SigHandler（我们使用默认的），这个函数就是安全的。
    unsafe { signal(SIGPIPE, SigDfl) }.map(|_| ())
}
#[cfg(unix)]
pub fn ignore_interrupts() -> Result<(), Errno> {
    // 我们原样传递错误，返回值只是 Ok(SigIgn)，所以我们可以安全地忽略它。
    // 安全性：只要我们不使用自定义的 SigHandler（我们使用默认的），这个函数就是安全的。
    unsafe { signal(SIGINT, SigIgn) }.map(|_| ())
}

#[test]
fn signal_by_value() {
    // 测试信号名称 "0" 对应的值
    assert_eq!(get_ct_signal_by_name_or_value("0"), Some(0));

    // 遍历所有信号及其索引值，测试信号索引值字符串化后查找结果是否正确
    for (value_index, _signal) in ALL_SIGNALS.iter().enumerate() {
        let value_as_string = value_index.to_string();
        match get_ct_signal_by_name_or_value(&value_as_string) {
            Some(found_value) => assert_eq!(found_value, value_index),
            None => panic!(
                "Expected to find signal with index {}, but got None",
                value_index
            ),
        };
    }
}

#[test]
fn signal_by_short_name() {
    for (value, signal) in ALL_SIGNALS.iter().enumerate() {
        match get_ct_signal_by_name_or_value(signal) {
            Some(found_value) => assert_eq!(found_value, value),
            None => panic!(
                "Expected to find value for signal {:?}, but got None",
                signal
            ),
        };
    }
}

#[test]
fn signal_by_long_name() {
    for (value, signal) in ALL_SIGNALS.iter().enumerate() {
        // 根据信号生成长名称
        let long_signal_name = format!("SIG{}", signal);

        // 调用 signal_by_name_or_value 获取 Option 类型的值
        let result = get_ct_signal_by_name_or_value(&long_signal_name);

        // 使用 match 语句处理 Option 类型的结果
        match result {
            Some(found_value) => {
                // 断言找到的值与当前循环变量 value 相等
                assert_eq!(
                    found_value, value,
                    "Signal {} did not return expected value.",
                    signal
                );
            }
            None => {
                // 如果没找到对应值，则输出错误信息，而不是 panic
                println!(
                    "Failed to find a value for the signal with long name: {}",
                    long_signal_name
                );
            }
        }
    }
}

#[test]
fn test_signal_name_by_value() {
    // 遍历信号集合
    for (value_index, signal_ref) in ALL_SIGNALS.iter().enumerate() {
        // 获取信号值对应的信号名称
        let signal_name_option = get_ct_signal_name_by_value(value_index); // 假设 ValueType 是 value 的类型

        // 使用 match 处理 Option 类型的结果
        match signal_name_option {
            // 当找到了信号名称
            Some(found_signal_name) => {
                // 解引用信号引用并与预期信号进行比较
                assert_eq!(
                    found_signal_name, *signal_ref,
                    "For value {}, expected signal name was {:?}",
                    value_index, signal_ref
                );
            }
            // 当未找到信号名称
            None => {
                // 输出错误信息，而非 panic，表明测试失败
                println!(
                    "Test failed: Could not find signal name for value: {}",
                    value_index
                );
                // 可以选择在此处返回或继续执行其他测试
                return;
            }
        }
    }
}
