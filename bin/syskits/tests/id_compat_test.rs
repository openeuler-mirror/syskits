#[cfg(all(target_os = "linux", feature = "id"))]
use std::path::Path;
#[cfg(all(target_os = "linux", feature = "id"))]
use std::process::{Command, Output};

#[cfg(all(target_os = "linux", feature = "id"))]
fn run_with_split_credentials(program: &str, args: &[&str]) -> Output {
    Command::new("/usr/bin/setpriv")
        .args([
            "--ruid=65534",
            "--euid=0",
            "--rgid=65534",
            "--egid=0",
            "--clear-groups",
            program,
        ])
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run {program}: {err}"))
}

#[cfg(all(target_os = "linux", feature = "id"))]
#[test]
fn group_list_includes_distinct_real_and_effective_gids_like_gnu() {
    if unsafe { libc::geteuid() } != 0
        || !Path::new("/usr/bin/setpriv").exists()
        || !Path::new("/usr/bin/id").exists()
    {
        eprintln!("skipping: test requires root, /usr/bin/setpriv, and /usr/bin/id");
        return;
    }

    for args in [["-G"], ["-Gr"]] {
        let gnu = run_with_split_credentials("/usr/bin/id", &args);
        let syskits = run_with_split_credentials(env!("CARGO_BIN_EXE_syskits"), &["id", args[0]]);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{args:?} status");
        assert_eq!(syskits.stderr, gnu.stderr, "{args:?} stderr");
        assert_eq!(syskits.stdout, gnu.stdout, "{args:?} stdout");
    }
}
