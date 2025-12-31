# How to update the internal database

Create the test fixtures by writing the output of the GNU dircolors commands to the fixtures folder:

```shell
dircolors --print-database > /PATH_TO_COREUTILS/tests/fixtures/dircolors/internal.expected
dircolors --print-ls-colors > /PATH_TO_COREUTILS/tests/fixtures/dircolors/ls_colors.expected
dircolors -b > /PATH_TO_COREUTILS/tests/fixtures/dircolors/bash_def.expected
dircolors -c > /PATH_TO_COREUTILS/tests/fixtures/dircolors/csh_def.expected
```

Run the tests:

```shell
cargo test --ct_features "dircolors" --no-default-ct_features
```

Edit `/PATH_TO_COREUTILS/src/ct/dircolors/src/colors.rs` until the tests pass.
