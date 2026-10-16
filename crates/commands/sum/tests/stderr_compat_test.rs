#[cfg(target_os = "linux")]
mod linux {
    use std::fs::File;
    use std::process::{Command, Stdio};

    fn run_with_full_stderr(program: &str, args: &[&str]) -> std::process::ExitStatus {
        let stderr = File::options()
            .write(true)
            .open("/dev/full")
            .expect("open /dev/full");

        Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stderr(Stdio::from(stderr))
            .status()
            .expect("run sum")
    }

    #[test]
    fn missing_file_with_full_stderr_exits_failure_without_panicking() {
        let missing = "/__syskits_sum_missing_stderr_compat__";
        let gnu = run_with_full_stderr("/usr/bin/sum", &[missing]);
        let syskits = run_with_full_stderr(env!("CARGO_BIN_EXE_sum"), &[missing]);

        assert_eq!(gnu.code(), Some(1));
        assert_eq!(syskits.code(), gnu.code());
    }
}
