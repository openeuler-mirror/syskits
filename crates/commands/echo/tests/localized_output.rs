#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn zh_cn_help_output_uses_syskits_information() {
    let output = Command::new(env!("CARGO_BIN_EXE_echo"))
        .arg0("echo")
        .arg("--help")
        .env("LC_ALL", "zh_CN.UTF-8")
        .output()
        .expect("run echo --help");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        r#"用法：echo [短选项]... [字符串]...
　或：echo 长选项
将 <字符串> 回显到标准输出。

  -n             不输出末尾的换行符
  -e             启用对反斜杠转义字符的解释
  -E             禁用对反斜杠转义字符的解释（默认）
      --help        显示此帮助信息并退出
      --version     显示版本信息并退出

若 -e 生效，则以下序列可被识别：

  \\      反斜杠
  \a      警报（BEL 响铃字符）
  \b      退格
  \c      不再产生新的输出
  \e      转义字符
  \f      换页
  \n      换行
  \r      回车
  \t      水平制表符
  \v      垂直制表符
  \0NNN   值为 NNN（1 至 3 位八进制数）的字节
  \xHH    值为 HH（1 至 2 位十六进制数）的字节

注意：您的 shell 可能内置了自己的 echo 程序版本，它通常会取代这里所描述的
版本。请查阅您的 shell 的文档以了解内置版本所支持的选项。

注意：建议使用 printf(1) 替代本命令，
因为它可以正常输出与选项相似的字符串。

syskits 在线帮助：<https://www.ctyun.cn/developer>
"#
        .as_bytes()
    );
    assert!(!output.stdout.windows(3).any(|window| window == b"GNU"));
}

#[test]
fn zh_cn_version_output_uses_syskits_information() {
    let output = Command::new(env!("CARGO_BIN_EXE_echo"))
        .arg0("echo")
        .arg("--version")
        .env("LC_ALL", "C.UTF-8")
        .env("LANGUAGE", "zh_CN")
        .output()
        .expect("run echo --version");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        format!(
            r#"echo (syskits) {}
Copyright (C) 2022-2025 China Telecom Cloud Technologies Co., Ltd.
许可证 Mulan PSL v2：<http://license.coscl.org.cn/MulanPSL2>。
本软件是自由软件：您可以自由修改和重新发布它。
在法律允许的范围内，不提供任何保证。
"#,
            env!("CARGO_PKG_VERSION")
        )
        .as_bytes()
    );
    assert!(!output.stdout.windows(3).any(|window| window == b"GNU"));
}
