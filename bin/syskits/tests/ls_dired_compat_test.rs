#[cfg(unix)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::process::{Command, Output};

    use tempfile::TempDir;

    fn run_gnu(directory: &TempDir, arguments: &[&str]) -> Output {
        Command::new("/usr/bin/ls")
            .current_dir(directory.path())
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run GNU ls")
    }

    fn run_syskits(directory: &TempDir, arguments: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_syskits"))
            .current_dir(directory.path())
            .env("LC_ALL", "C")
            .arg("ls")
            .args(arguments)
            .output()
            .expect("run syskits ls")
    }

    #[test]
    fn ls_dired_symlink_span_excludes_arrow_and_target() {
        let directory = TempDir::new().expect("tempdir");
        fs::create_dir(directory.path().join("d")).expect("create fixture directory");
        fs::write(directory.path().join("d/a"), b"").expect("create target file");
        symlink("a", directory.path().join("d/link")).expect("create symlink");
        let arguments = ["--dired", "-ln", "--time-style=+%s", "d"];

        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code());
        assert_eq!(syskits.stderr, gnu.stderr);
        assert_eq!(syskits.stdout, gnu.stdout);
    }

    #[test]
    fn ls_recursive_dired_offsets_cover_each_filename() {
        let directory = TempDir::new().expect("tempdir");
        fs::create_dir_all(directory.path().join("d/sub/deep"))
            .expect("create fixture directories");
        fs::write(directory.path().join("d/a"), b"").expect("create top-level file");
        fs::write(directory.path().join("d/sub/b"), b"").expect("create nested file");
        fs::write(directory.path().join("d/sub/deep/c"), b"").expect("create deeply nested file");
        let arguments = ["--dired", "-lnR", "--time-style=+%s", "d"];

        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code());
        assert_eq!(syskits.stderr, gnu.stderr);
        assert_eq!(syskits.stdout, gnu.stdout);
    }

    #[test]
    fn ls_dired_indentation_precedes_inode_and_block_prefixes() {
        let directory = TempDir::new().expect("tempdir");
        fs::create_dir(directory.path().join("d")).expect("create fixture directory");
        fs::write(directory.path().join("d/a"), b"").expect("create first file");
        fs::write(directory.path().join("d/b"), b"").expect("create second file");
        let arguments = ["--dired", "-lnis", "--time-style=+%s", "d"];

        let gnu = run_gnu(&directory, &arguments);
        let syskits = run_syskits(&directory, &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code());
        assert_eq!(syskits.stderr, gnu.stderr);
        assert_eq!(syskits.stdout, gnu.stdout);
    }
}
