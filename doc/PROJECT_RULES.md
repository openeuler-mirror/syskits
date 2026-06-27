# 项目规则与规范 (`PROJECT_RULES.md`)

本文件定义了 `syskits` 项目的结构、编码规范、提交流程以及其他相关约定，旨在确保项目代码的一致性、可维护性和高质量。所有贡献者都应遵守这些规则。

## 1. 项目概览与架构

### 1.1. 核心目标
`syskits` 项目旨在提供一个功能丰富的跨平台命令行工具集。它被实现为一个多功能调用二进制文件 (multi-call binary)，用户可以通过主程序 `syskits` 调用不同的工具，或者通过指向主程序的符号链接（例如 `ls`, `cat`）直接执行相应工具。

### 1.2. 主要组件

项目主要由以下几个核心组件构成：

*   **`syskits` (主应用 Crate)**:
    *   **位置**: `bin/syskits/` (项目根目录下的 `bin` 子目录中)。
    *   **源码入口**: `bin/syskits/src/main.rs` (或其他，如 `bin/syskits/src/bin/syskits.rs`，取决于其 `Cargo.toml` 定义)。
    *   **职责**:
        *   作为程序的主入口，解析全局命令行参数。
        *   根据用户提供的工具名称（通过程序名或第一个参数判断）分发执行到相应的工具 Crate。
        *   处理通用的顶层命令，如 `--help`, `--version`。
        *   处理 shell 补全 (`completion`) 和手册页生成 (`manpage`) 等元命令。

*   **`ctcore/` (核心库 Crate)**:
    *   **位置**: `crates/ctcore/` (项目根目录下的 `crates` 子目录中)。
    *   **职责**:
        *   提供所有工具 Crate 共享的核心功能和基础设施。
        *   定义核心的 `Tool` trait，所有工具 Crate 都必须实现此接口。
        *   提供统一的错误处理机制，包括 `CTResult` 类型和 `CTError` trait。
        *   包含通用的辅助模块，如文件系统操作、字符串处理、数值解析、国际化支持框架、输出显示逻辑等。

*   **`commands/` (工具 Crates 集合目录)**:
    *   **位置**: `crates/commands/` (项目根目录下的 `crates` 子目录中)。
    *   **内容**: 包含所有独立的命令行工具 Crates。每个工具都是此目录下的一个子目录，例如 `crates/commands/ls/` 存放 `ct_ls` crate。
    *   **职责**: 实现具体的命令行工具功能，例如 `ls`, `cat`, `mkdir` 等。每个工具是一个独立的 Rust crate，有自己的 `Cargo.toml` 文件。

*   **`tool_derive/` (过程宏 Crate)**:
    *   **位置**: `crates/tool_derive/` (项目根目录下的 `crates` 子目录中)。
    *   **职责**:
        *   提供 `#[derive(Tools)]` 过程宏。
        *   此宏在编译 `syskits` 主包时，自动扫描 `commands/` 目录中符合条件的工具 Crates，并收集所有实现了 `ctcore::Tool` trait 的工具，用于在 `syskits` 运行时进行命令分发。

*   **`compat_test/` (兼容性测试 Crate)**:
    *   **位置**: `bin/compat_test/` (项目根目录下的 `bin` 子目录中)。
    *   **职责**: 包含用于验证 `syskits` 工具与 coreutils 行为兼容性的测试工具和用例。

### 1.3. Workspace (工作空间)

*   项目使用 Cargo 的 workspace 功能进行组织和管理。
*   项目根目录的 `Cargo.toml` 文件是 Workspace 的根定义文件。
*   **成员**: 所有主要组件都应是 workspace 的成员。根 `Cargo.toml` 文件应在 `[workspace.members]` 数组中明确列出所有成员 crates 的路径。
        ```toml
        # In root Cargo.toml
        [workspace]
        resolver = "2"
        members = [
            "bin/syskits",
            "bin/compat_test",
            "crates/ctcore",
            "crates/tool_derive",
            "crates/commands/*"
        ]
        ```
*   `[workspace.package]` 部分（位于根 `Cargo.toml`）定义了共享的元数据。
*   `[workspace.dependencies]` 部分（位于根 `Cargo.toml`）定义了整个 workspace 共享的第三方依赖及其版本。

## 2. 工具 Crate (`ct_<tool_name>`) 规范

### 2.1. 目录与命名约定
*   每个独立的命令行工具都应作为一个单独的 crate 实现。
*   工具 Crate 的源代码应存放于 `crates/commands/<tool_name>/` 目录下。
*   工具 Crate 的包名 (package name) 应遵循 `ct_<tool_name>` 的格式（例如 `ct_ls`, `ct_cat`）。

### 2.2. `Cargo.toml` (工具 Crate 清单文件)
每个工具 Crate 的 `Cargo.toml` 文件必须遵循以下规范：

*   **`package.name`**: 必须设置为 `ct_<tool_name>`。
*   **Workspace 继承**: 必须继承 workspace 的共享元数据，例如：
    ```toml
    [package]
    name = "ct_my_tool"
    version.workspace = true
    authors.workspace = true
    license.workspace = true
    # ... 其他共享元数据
    ```
*   **库目标 (`[lib]`)**: 必须定义一个库目标，这是 `syskits` 主程序调用该工具的入口。
    ```toml
    [lib]
    # path 通常是 "src/lib.rs" 或 "src/<tool_name>.rs"
    # 例如，对于 ct_ls，可能是 path = "src/ls.rs"
    ```
*   **二进制目标 (`[[bin]]`, 可选但推荐)**: 可以定义一个与工具同名的二进制目标，用于独立运行和测试。
    ```toml
    [[bin]]
    name = "<tool_name>" # 例如 "ls"
    path = "src/main.rs"
    ```
*   **依赖 (`[dependencies]`)**:
    *   必须依赖 `ctcore`: `ctcore = { workspace = true, features = ["...", "..."] }`。根据需要启用 `ctcore` 提供的特性。
    *   所有其他第三方依赖应尽可能从 `[workspace.dependencies]` 继承：`other_dependency = { workspace = true }`。
*   **特性 (`[features]`)**:
    *   工具 Crate 可以定义自己的特性，以提供可选功能（例如，`ct_ls` 的 `feat_acl` 特性）。
    *   这些特性可以被 `syskits` 主包的顶层特性聚合。

### 2.3. 库源文件 (`src/lib.rs` 或 `src/<tool_name>.rs`)
此文件是工具 Crate 作为库被 `syskits` 使用时的入口，必须包含：

*   **公共结构体**: 定义一个简单的公共结构体，例如 `pub struct MyTool;`。
*   **`ctcore::Tool` Trait 实现**: 该结构体必须实现 `ctcore::Tool` trait。
    *   **`fn name(&self) -> &'static str;`**:
        *   返回此工具的公共调用名称（例如 `"ls"`, `"cat"`）。
        *   此名称将用于 `syskits` 的命令分发，并应与 `syskits` 主应用 (`bin/syskits/Cargo.toml`) 的 `[features]` 部分中用于激活此工具的名称一致。
    *   **`fn command(&self) -> clap::Command;`**:
        *   使用 `clap` crate 构建并返回该工具的完整命令行接口 (CLI) 定义。这包括所有参数、选项、子命令、帮助文本等。
        *   **建议**: 将 `clap::Arg` 的名称（ID）定义为模块内的常量字符串，以方便管理和引用。
    *   **`fn execute(&self, args: &[OsString]) -> CTResult<()>;`**:
        *   此方法是工具执行其主要逻辑的入口。
        *   **参数解析**: 使用 `self.command()` 返回的 `Command` 对象来解析传入的 `args`。推荐模式：`let matches = self.command().get_matches_from(args)?;`。
        *   **配置管理**: 基于解析后的参数，创建一个特定于工具的配置结构体（例如 `MyToolConfig::from(&matches)`) 来管理和传递选项。
        *   **核心逻辑**: 调用工具的核心业务逻辑函数。
        *   **错误处理**: 所有操作中可能发生的错误都应通过返回包含自定义错误类型（该类型需实现 `CTError` trait）的 `CTResult` 进行报告。
        *   **成功返回**: 成功完成时返回 `Ok(())`。
*   **错误处理**:
    *   为工具定义一个特定的 `enum` 错误类型。
    *   为此错误类型实现 `std::error::Error` 和 `ctcore::CTError` trait。

### 2.4. 二进制源文件 (`src/main.rs`, 可选)
如果工具 Crate 包含一个独立的二进制目标：

*   其 `src/main.rs` 文件应尽可能简洁。
*   主要职责是获取命令行参数，并调用库中定义的 `Tool` 实现的 `execute()` 方法。
*   示例：
    ```rust
    // In commands/my_tool/src/main.rs
    // 假设 MyTool crate 的库部分定义了 MyTool struct
    // 并且该 crate 名为 ct_my_tool
    use ct_my_tool::MyTool;
    use ctcore::{Tool, CTResult}; // 假设 CTResult 和 Tool 在 ctcore 中
    use std::env;
    use std::ffi::OsString;

    fn main() {
        let tool = MyTool;
        let args: Vec<OsString> = env::args_os().collect();

        if let Err(e) = tool.execute(if args.len() > 1 { &args[1..] } else { &[] }) {
            eprintln!("Error: {}", e);
            std::process::exit(e.code());
        }
    }
    ```

## 3. `syskits` 主包集成规范

本节描述 `syskits` 主应用 Crate (位于 `bin/syskits/`) 如何集成和调用工具 Crates。

### 3.1. 工具的条件编译与注册
*   **依赖声明**: `syskits` 主应用 (`bin/syskits/Cargo.toml`) 在其 `[dependencies]` 部分将每个工具 crate (例如 `ct_ls`) 声明为一个可选的路径依赖。路径应相对于 `bin/syskits/` 指向 `crates/commands/<tool_name>`。
    ```toml
    # In bin/syskits/Cargo.toml -> [dependencies]
    ls = { optional = true, package = "ct_ls", path = "../../crates/commands/ls" }
    cat = { optional = true, package = "ct_cat", path = "../../crates/commands/cat" }
    # ... 其他工具
    ```
*   **特性激活**: `syskits` 主应用 (`bin/syskits/Cargo.toml`) 的 `[features]` 部分通过启用相应的依赖特性来"激活"工具。
*   **自动发现与注册**: `#[derive(tool_derive::Tools)]` 宏应用于 `bin/syskits/src/main.rs` (或主应用入口文件) 中的主结构体。此宏在编译时：
    1.  扫描项目根目录下的 `crates/commands/` 目录，查找所有子目录（即工具 crates）。
    2.  在每个工具 crate 的库文件（`src/lib.rs` 或 `src/<tool_name_short>.rs`）中查找实现了 `ctcore::Tool` trait 的公共结构体。
    3.  对于每个发现的 `Tool` 实现，它会调用其 `name()` 方法获取工具的公共调用名称。
    4.  `tool_derive` 宏生成的代码（如 `use` 语句、`ALL_COMMANDS` 数组、`get_tool` 函数）会基于这些发现的工具，并使用 `#[cfg(feature = "<tool_short_name>")]` 进行条件编译，确保只有通过 `syskits` 特性启用的工具才会被包含和注册。

### 3.2. `syskits.rs` (`src/bin/syskits.rs`)
*   应用 `#[derive(tool_derive::Tools)]` 到主应用结构体。
*   包含核心的命令分发逻辑：
    *   检查程序被调用时的名称。
    *   如果不是直接通过工具名称调用，则检查第一个命令行参数。
    *   将匹配到的工具名称和剩余参数传递给相应 `Tool` 实现的 `execute()` 方法。

## 4. `ctcore/` (核心库) 规范

### 4.1. `Tool` Trait
*   定义于 `ctcore/src/lib/tool.rs` (或类似路径)。
*   接口包含 `name()`, `command()`, 和 `execute()` 方法，是所有工具 Crate 必须实现的契约。

### 4.2. 错误处理 (`CTResult`, `CTError`)
*   提供统一的 `Result` 类型别名：
    ```rust
    // In ctcore
    pub trait CTError: std::error::Error + Send + Sync {
        fn code(&self) -> i32; // 返回与错误对应的退出码
    }
    pub type CTResult<T> = Result<T, Box<dyn CTError>>;
    ```
*   所有 `ctcore` 内部函数以及所有工具 Crate 的 `execute` 方法都应使用 `CTResult` 作为返回类型。

### 4.3. 共享模块
*   `ctcore` 应包含被多个工具 Crate 复用的模块，例如：
    *   文件系统操作的抽象和辅助函数。
    *   字符串解析和验证工具。
    *   国际化支持框架的接入点 (其资源可能需要配置正确的相对路径，或由 `syskits` 主应用统一管理)。
    *   标准化的输出显示和格式化逻辑。
    *   颜色和样式的管理。
*   这些模块应设计良好，API清晰稳定。

### 4.4. 特性 (`[features]`)
*   `ctcore` 自身可以使用 Cargo 特性来提供可选的功能模块。
*   工具 Crates 在依赖 `ctcore` 时，可以根据需要启用这些特性。例如：`ctcore = { workspace = true, features = ["colors", "fs_extra"] }`。

## 5. 代码风格与约定

### 5.1. 格式化
*   **强制使用 `rustfmt`**: 所有提交的代码都必须经过 `rustfmt` 格式化。
*   项目根目录应包含一个 `rustfmt.toml` (或 `.rustfmt.toml`) 配置文件，定义统一的格式化规则。

### 5.2. 命名约定
*   **Crates**: `ct_<tool_name>` (例如 `ct_ls`), `ctcore`, `tool_derive`, `compat_test`, `syskits` (主应用包名，定义于 `bin/syskits/Cargo.toml`)。
*   **模块 (Modules) / 目录 (Directories)**: `snake_case` (例如 `file_utils`, `crates/commands/ls`, `crates/ctcore`, `bin/syskits`).
*   **文件 (Files)**: `snake_case.rs` (例如 `file_utils.rs`, `ls.rs`)。
*   **类型 (Structs, Enums, Traits)**: `PascalCase` (例如 `MyStruct`, `FileFormat`, `Tool`)。
*   **函数 (Functions) / 方法 (Methods)**: `snake_case` (例如 `calculate_size`, `obj.get_name()`)。
*   **常量 (Constants) / 静态变量 (Statics)**: `UPPER_SNAKE_CASE` (例如 `DEFAULT_BUFFER_SIZE`, `MAX_RETRIES`)。
*   **特性名 (Cargo Features)**: `snake_case` (例如 `feat_common_core`, `use_serde`) 或代表工具的简单名称 (例如 `ls`, `cat`)，这些简单名称通常对应于启用的依赖项。

### 5.3. 注释
*   **文档注释 (`///`)**: 所有公共 API（包括 crates, public modules, public functions, structs, enums, traits, 及其 public fields）都必须有清晰、准确的文档注释。
*   **常规注释 (`//`)**: 用于解释非显而易见的复杂逻辑、算法选择、潜在问题或待办事项。避免对显而易见的代码进行注释。

### 5.4. 测试
*   **单元测试**:
    *   应与被测试的代码放在同一文件中（使用 `#[cfg(test)] mod tests { ... }`）或同一模块下的 `tests.rs` 文件中。
    *   专注于测试独立的函数和模块逻辑。
*   **集成测试**:
    *   可以放在各个工具 Crate 的 `tests/` 目录下。
*   **兼容性测试**:
    *   位于 `compat_test/` crate 中，用于测试与 coreutils 的兼容性。
*   **测试覆盖率**: 鼓励高测试覆盖率。

### 5.5. 国际化 (i18n)
*   所有面向用户的字符串（例如错误信息、帮助文本、输出内容）都应支持国际化。
*   推荐使用 `rust-i18n` 库（或项目中已选定的类似库）。
*   每个工具 Crate 应将其本地化资源文件（例如 `.json` 或 `.yaml`）存放在其自身的 `locales/` 目录下 (例如 `crates/commands/ls/locales/`)。
*   `syskits` 主应用 (`bin/syskits/`) 如果有自身的本地化字符串，可以存放在 `bin/syskits/locales/`。

### 5.6. Clippy (Lints)
*   定期运行 `cargo clippy` 并解决其提出的所有警告和建议，以保持代码质量和一致性。
*   可以在项目级别配置 Clippy 的 lint 等级。

## 6. 依赖管理

### 6.1. Workspace Dependencies
*   所有共享的第三方依赖项及其版本应在根 `Cargo.toml` 的 `[workspace.dependencies]` 表中集中定义。
*   这有助于确保整个项目中使用一致的依赖版本，简化版本管理和更新。

### 6.2. Crate Dependencies
*   各个成员 Crate (包括工具 Crates, `ctcore`, 等) 在其各自的 `Cargo.toml` 文件中声明依赖时，应通过 `dependency_name = { workspace = true }` 的语法引用在 `[workspace.dependencies]` 中定义的依赖。
*   如果需要为特定 crate 启用依赖的特定特性，可以这样做：`dependency_name = { workspace = true, features = ["feature_a"] }`。

### 6.3. 版本更新
*   更新依赖版本前应进行充分的调研和测试，以避免引入不兼容的变更或新的 bug。
*   推荐使用 `cargo update -p <dependency_name>` 来更新单个依赖，或 `cargo outdated` 查看可更新的依赖。

## 7. 版本控制与提交流程 (简要)

*   **Git 工作流**: 项目可以遵循常见的分支模型，例如 Gitflow (具有 `main`, `develop`, `feature/*`, `release/*`, `hotfix/*` 分支) 或基于主干的开发模型（直接向 `main` 或 `develop` 分支提交，通过特性分支进行较大改动）。具体流程应在团队内达成一致。
*   **提交信息 (Commit Messages)**:
    *   应清晰、简洁地描述本次提交的变更内容。
    *   推荐遵循 Conventional Commits 规范，例如：`feat: add new 'cp' command` 或 `fix(ls): correct sorting by size`。
*   **代码审查 (Code Reviews)**:
    *   所有代码变更（特别是新功能和 bug 修复）在合并到主开发分支前，都应经过至少一位其他团队成员的代码审查。
*   **版本号管理**:
    *   项目版本号应遵循语义化版本控制 (SemVer 2.0.0) 规范 (MAJOR.MINOR.PATCH)。
    *   版本号在根 `Cargo.toml` 的 `[workspace.package]` 中统一管理。
*   **发布流程**:
    *   应包含完整的测试（单元、集成、兼容性）。
    *   构建可发布的二进制文件。
    *   在版本控制系统中打上对应的版本标签 (e.g., `v1.2.3`)。
    *   更新项目的 `CHANGELOG.md` 文件。
