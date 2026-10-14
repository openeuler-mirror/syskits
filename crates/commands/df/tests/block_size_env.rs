use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_df_with_env(df_block_size: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_df"));
    command
        .arg0("df")
        .arg("/")
        .env("LC_ALL", "C")
        .env_remove("DF_BLOCK_SIZE")
        .env_remove("BLOCK_SIZE")
        .env_remove("BLOCKSIZE")
        .env_remove("POSIXLY_CORRECT");
    if let Some(value) = df_block_size {
        command.env("DF_BLOCK_SIZE", value);
    }
    command.output().expect("run df")
}

#[test]
fn zero_df_block_size_uses_the_default_block_size() {
    let default_output = run_df_with_env(None);
    let zero_output = run_df_with_env(Some("0"));

    assert!(default_output.status.success());
    assert!(zero_output.status.success());
    assert_eq!(zero_output.stdout, default_output.stdout);
    assert_eq!(zero_output.stderr, default_output.stderr);
}
