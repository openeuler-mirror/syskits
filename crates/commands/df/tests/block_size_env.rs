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

fn run_df(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_df"))
        .arg0("df")
        .args(arguments)
        .env("LC_ALL", "C")
        .env_remove("DF_BLOCK_SIZE")
        .env_remove("BLOCK_SIZE")
        .env_remove("BLOCKSIZE")
        .env_remove("POSIXLY_CORRECT")
        .output()
        .expect("run df")
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

#[test]
fn special_df_block_size_values_match_human_readable_options() {
    for (value, option) in [("human-readable", "-h"), ("si", "-H")] {
        let from_env = run_df_with_env(Some(value));
        let from_option = run_df(&[option, "/"]);

        assert!(from_env.status.success());
        assert_eq!(from_env.stdout, from_option.stdout, "DF_BLOCK_SIZE={value}");
        assert_eq!(from_env.stderr, from_option.stderr, "DF_BLOCK_SIZE={value}");
    }
}

#[test]
fn special_block_size_arguments_match_human_readable_options() {
    for (arguments, option) in [
        (&["--block-size=human-readable", "/"][..], "-h"),
        (&["--block-size=si", "/"][..], "-H"),
        (&["--block-size", "human-readable", "/"][..], "-h"),
        (&["-B", "human-readable", "/"][..], "-h"),
        (&["-Bhuman", "/"][..], "-h"),
    ] {
        let from_block_size = run_df(arguments);
        let from_option = run_df(&[option, "/"]);

        assert!(from_block_size.status.success(), "{arguments:?}");
        assert_eq!(from_block_size.stdout, from_option.stdout, "{arguments:?}");
        assert_eq!(from_block_size.stderr, from_option.stderr, "{arguments:?}");
    }
}
