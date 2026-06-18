# syskits: CTyunOS Base Component Library

## 1. Objectives

### Providing Proprietary Base System Component Libraries (libs) and Binaries (bins) for `ctshell`

- lib: Used by built-in `ctshell` commands. Integrated with `ctshell` data stream logic to deliver an optimized user experience. (Referenced as Rust library source code)
- bin: Leveraged by `ctshell` as high-priority execution commands. (Referenced as a Rust application)

### Replacing Coreutils Base Components

- Functionality: Match the capabilities required to fully replace standard `coreutils` base components.
- Interface: Maintain strict backward compatibility with `coreutils` interfaces.

## 2. Development

### Prerequisites

- Rust version 1.70.0

### Build Instructions

- Default build

    ```shell
    cargo build --release
    ```

- Platform support
Windows support is planned, primarily aimed at enabling `ctshell` usage on Windows environments.

    ```shell
    cargo build --release --features windows
    ```

    ```shell
    cargo build --features unix
    ```

- Build a single feature, e.g., build the `ls` command (integrated into syskits logic).

    ```shell
    cargo build --features ls
    ```

- Build a single feature (e.g., a standalone `ls` executable).

    ```shell
    cargo build -p ct_ls
    ```

### Testing

- Cross-platform testing (required before submitting code)

    ```shell
    cargo test --all -p ct
    ```

- Test a single feature (e.g., `ls`).

    ```shell
    cargo test --all -p ct_ls
    ```

### Static Analysis

- `check`

    ```shell
    cargo check --all
    ```

- `clippy` (required before submitting code)

    ```shell
    cargo clippy --all-targets --all-features
    ```

### Code Formatting

Code must be formatted before submission:

```shell
cargo fmt --all
```

### Debugging

```shell
rust-gdb --args target/debug/syskits ls
(gdb) b ls.rs:79
(gdb) run
```
