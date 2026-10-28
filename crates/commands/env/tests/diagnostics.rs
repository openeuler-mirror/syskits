#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn env_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_env"));
    command
        .arg0("env")
        .env_clear()
        .env("LANG", "C.UTF-8")
        .env("PATH", "/usr/bin:/bin");
    command
}

#[test]
fn debug_arguments_use_utf8_locale_quote_marks() {
    let output = env_command()
        .args(["--debug", "-i", "-S", "A=1 /usr/bin/printf ok"])
        .output()
        .expect("run env --debug");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"ok");
    assert_eq!(
        output.stderr,
        concat!(
            "split -S:  ‘A=1 /usr/bin/printf ok’\n",
            " into:    ‘A=1’\n",
            "     &    ‘/usr/bin/printf’\n",
            "     &    ‘ok’\n",
            "cleaning environ\n",
            "setenv:   A=1\n",
            "executing: /usr/bin/printf\n",
            "   arg[0]= ‘/usr/bin/printf’\n",
            "   arg[1]= ‘ok’\n",
        )
        .as_bytes()
    );
}

#[test]
fn command_error_uses_utf8_locale_quote_marks() {
    let output = env_command()
        .arg("missing command")
        .output()
        .expect("run env with a missing command");

    assert_eq!(output.status.code(), Some(127));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        concat!(
            "env: ‘missing command’: No such file or directory\n",
            "env: use -[v]S to pass options in shebang lines\n",
        )
        .as_bytes()
    );
}
