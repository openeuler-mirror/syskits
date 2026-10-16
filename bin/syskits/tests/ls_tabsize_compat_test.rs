use std::process::Command;

#[test]
fn invalid_tabsize_is_fatal_for_ls_dir_and_vdir() {
    for command in ["ls", "dir", "vdir"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .args([command, "--tabsize=bad", "."])
            .output()
            .unwrap_or_else(|err| panic!("run syskits {command}: {err}"));

        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(
            output.stdout.is_empty(),
            "{command}: stdout should be empty, got {:?}",
            String::from_utf8_lossy(&output.stdout)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("invalid tab size") && stderr.contains("bad"),
            "{command}: stderr was {stderr:?}",
        );
    }
}
