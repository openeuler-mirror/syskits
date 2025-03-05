# compat_test兼容性测试目的

一个全面的测试框架，用于确保 Syskits 与 GNU Coreutils 实现的完全兼容性。

# Syskits 兼容性测试框架
## 整体设计框架

总体思想，通过执行syskits 和 coreutils 的相同命令，比较它们的执行结果，来确保syskits 和 coreutils 的兼容性。结果比较主要从4个维度：
1. 命令执行结果的退出码
2. 命令执行结果的标准输出
3. 命令执行结果的标准错误
4. 命令执行结果的功能验证 （通过验证命令来验证）

为了保证测试的准确性，设计了命令行沙箱，来模拟执行环境，并限制执行资源。
为了保证测试的全面性，设计了多种测试模式，支持单个命令测试，也支持多个命令组合测试。
这里解决了两个问题：
1. 就是命令执行结果的基准问题，在测试中，需要以coreutils 的执行结果作为基准，但是不能每次都执行coreutils 命令，这样会降低测试效率。所以设计了基准模式，在第一次执行命令时，将执行结果作为基准，后续执行命令时，可以直接使用基准结果作为期望结果，来比较执行结果。
2. 命令执行功能性验证逻辑可观测思考，命令的不同，他们可能不能使用标准输出和标准错误输出来观察，例如 chmod，所以在设计单元测试用例参数时，添加了verifications功能验证字段来完成功能验证，也就是在这个字段，可以添加其他命令来辅助验证功能是不是生效，来综合判断。

## 命令执行结果环境变量

为了方便在验证阶段引用命令执行结果，框架会自动将命令执行结果保存到以下环境变量中：

- `CMD_EXIT_CODE`: 命令的退出码
- `CMD_STDOUT`: 命令的标准输出
- `CMD_STDERR`: 命令的标准错误

这些环境变量可以在 `verifications` 部分的命令中使用，例如：

```json
"verifications": [
  {
    "command": "test \"$CMD_STDOUT\" = \"expected output\"",
    "expected_exit": 0
  }
]
```

## 工作模式

框架支持两种工作模式，通过配置 `coreutils_path` 自动确定：

1. **基准模式**：配置了 GNU Coreutils 路径时自动启用
   - 首先执行 GNU Coreutils 命令获取基准结果
   - 将基准结果自动作为当前测试用例的期望结果
   - 执行 Syskits 命令并与基准结果比对
   - 适用于确保与 GNU Coreutils 的完全兼容性

2. **标准模式**：未配置 GNU Coreutils 路径时使用
   - 直接使用测试用例中定义的期望结果
   - 执行 Syskits 命令并与期望结果比对
   - 适用于特定场景测试或环境不具备 GNU Coreutils 时

## 目录结构

```
compat_test/
├── src/
│   ├── lib.rs           # 核心类型和特征定义
│   ├── executor.rs      # 命令执行和结果比对
│   ├── test_case.rs     # 测试用例管理
│   ├── config.rs        # 配置管理
│   ├── reporter.rs      # 测试报告生成
│   ├── sandbox.rs         # 沙箱环境实
│   └── bin/
│       └── run_tests.rs # 测试运行器
└── test_cases/         # JSON 格式测试用例
    ├── pathchk.json
    ├── timeout.json
    └── ...
```

## 核心组件

### 1. 测试运行器 (TestRunner)

```rust
pub struct TestRunner {
    config: TestConfig,
    test_manager: TestCaseManager,
}
```

主要职责：
- 加载和管理测试配置
- 协调测试用例的执行
- 支持串行和并行测试执行
- 生成测试报告

### 2. 命令执行器 (CommandExecutor)

```rust
pub struct CommandExecutor {
    config: TestConfig,
}

pub struct ParallelTestExecutor {
    config: TestConfig,
}
```

主要职责：
- 在沙箱环境中执行命令
- 比较 Syskits 和 GNU Coreutils 的执行结果
- 支持资源限制和超时控制
- 处理命令输出和退出码

### 3. 测试用例管理 (TestCaseManager)

```rust
pub struct TestCaseManager {
    test_cases_dir: PathBuf,
}

pub struct TestCase {
    command: String,
    description: String,
    setup_commands: Vec<String>,
    args: Vec<String>,
    expected_exit: i32,
    stdout: String,
    stderr: String,
    verifications: Vec<Verification>,
    cleanup_commands: Vec<String>,
}
```

主要职责：
- 加载和解析测试用例
- 管理测试用例的生命周期
- 提供测试用例的元数据

### 4. 沙箱环境 (IsolatedSandbox)

```rust
pub struct IsolatedSandbox {
    temp_dir: Option<TempDir>,
    resource_limiter: Option<ResourceLimiter>,
}

pub struct ResourceLimiter {
    limits: HashMap<Resource, (u64, u64)>,
}
```

主要职责：
- 提供隔离的测试环境
- 管理临时文件和目录
- 控制资源使用限制
- 处理权限和用户身份

### 5. 配置管理 (Config)

```rust
pub struct Config {
    pub syskits: SyskitsConfig,
    pub test: TestSettings,
}

pub struct TestConfig {
    pub syskits_path: PathBuf,
    pub coreutils_path: Option<PathBuf>,
    pub test_cases_dir: PathBuf,
    pub show_progress: bool,
    pub cleanup: bool,
    pub report_format: String,
    pub report_dir: PathBuf,
    pub default_timeout: u64,
    pub show_diff: bool,
    pub mode: SyskitsMode,
    pub commands_dir: Option<PathBuf>,
    pub verbose: bool,
}
```

主要职责：
- 管理框架配置
- 支持命令行参数和配置文件
- 提供默认配置值

### 6. 报告生成器 (Reporter)

```rust
pub struct Reporter {
    format: ReportFormat,
    output_dir: PathBuf,
}
```

支持的格式：
- 文本格式（人类可读）
- JSON 格式（机器可读）
- HTML 格式（网页展示）

## 测试用例格式

使用 JSON 格式定义测试用例。以下是字段说明：

```json
{
  "tests": [
    {
      // 基本信息（必填）
      "command": "命令名称",
      "description": "测试描述",
      "args": ["参数1", "参数2"],
      
      // 期望结果（必填）
      "expectation": {
        // 命令执行结果
        "execution": {
          "exit_code": 0,               // 预期退出码，设置为null则忽略比较
          "stdout": "",                 // 预期标准输出，设置为null则忽略比较
          "stderr": ""                  // 预期标准错误，设置为null则忽略比较
        },
        // 功能验证命令列表
        "verifications": [
          {
            "command": "验证命令",        // 验证命令
            "expected_exit": 0,          // 预期退出码
            "expected_stdout": null,     // 预期标准输出，设置为null则忽略比较
            "expected_stderr": null      // 预期标准错误，设置为null则忽略比较
          }
        ],
        // 结果比较的忽略配置
        "ignore_fields": {
          "ignore_exit_code": false,    // 是否忽略退出码比较
          "ignore_stdout": false,       // 是否忽略标准输出比较
          "ignore_stderr": false,       // 是否忽略标准错误比较
          "ignore_function": false      // 是否忽略功能验证比较
        }
      },

      // 测试控制（可选）
      "setup_commands": [               // 环境准备命令
        "export VAR=value",            // 设置环境变量
        "mkdir -p test_dir",           // 创建测试目录
        "touch test_file"              // 创建测试文件
      ],
      "cleanup_commands": [             // 环境清理命令
        "unset VAR",                   // 清理环境变量
        "rm -rf test_dir",             // 删除测试目录
        "rm -f test_file"              // 删除测试文件
      ],
      "requires_root": false,           // 是否需要 root 权限
      "timeout": 30,                    // 超时时间（秒）
      "tags": ["标签1", "标签2"],        // 测试标签

      // 沙箱环境配置（可选）
      "environment": {
        "resource_limits": {            // 资源限制
          "cpu_time": 10,              // CPU 时间限制（秒）
          "memory_size": 1048576,      // 内存限制（字节）
          "open_files": 1024           // 最大打开文件数
        }
      }
    }
  ]
}
```

### 字段说明

1. **基本信息（必填）**
   - `command`: 要测试的命令名称
   - `description`: 测试用例描述
   - `args`: 命令参数数组

2. **期望结果（必填）**
   - `expectation.execution`: 命令执行结果
     * `exit_code`: 预期的命令退出码，设置为null则忽略比较
     * `stdout`: 预期的标准输出内容，设置为null则忽略比较
     * `stderr`: 预期的标准错误内容，设置为null则忽略比较
   - `expectation.verifications`: 功能验证命令列表
     * `command`: 要执行的验证命令
     * `expected_exit`: 验证命令的预期退出码
     * `expected_stdout`: 验证命令的预期标准输出
     * `expected_stderr`: 验证命令的预期标准错误
   - `expectation.ignore_fields`: 结果比较的忽略配置
     * `ignore_exit_code`: 忽略退出码比较
     * `ignore_stdout`: 忽略标准输出比较
     * `ignore_stderr`: 忽略标准错误比较
     * `ignore_function`: 忽略功能验证比较

3. **测试控制（可选）**
   - `setup_commands`: 环境准备命令列表
     * 用于设置测试环境
     * 可以包含环境变量设置
     * 可以创建必要的文件和目录
   - `cleanup_commands`: 环境清理命令列表
     * 用于清理测试环境
     * 确保测试前后环境一致
   - `requires_root`: 是否需要 root 权限
   - `timeout`: 命令执行超时时间（秒）
   - `tags`: 测试标签列表

4. **沙箱环境（可选）**
   - `environment.resource_limits`: 资源限制配置
     * `cpu_time`: CPU 时间限制（秒）
     * `memory_size`: 内存使用限制（字节）
     * `open_files`: 最大打开文件数限制

### 示例

1. 基本命令测试：
```json
{
  "tests": [
    {
      "command": "echo",
      "description": "测试基本的 echo 命令",
      "args": ["Hello, World!"],
      "expectation": {
        "execution": {
          "exit_code": 0,
          "stdout": "Hello, World!\n",
          "stderr": ""
        }
      }
    }
  ]
}
```

2. 带环境准备和验证的测试：
```json
{
  "tests": [
    {
      "command": "ls",
      "description": "测试文件列表功能",
      "args": ["-l", "test_dir"],
      "setup_commands": [
        "mkdir -p test_dir",
        "touch test_dir/file1",
        "touch test_dir/file2"
      ],
      "expectation": {
        "execution": {
          "exit_code": 0
        },
        "verifications": [
          {
            "command": "test -d test_dir -a -f test_dir/file1 -a -f test_dir/file2",
            "expected_exit": 0
          }
        ],
        "ignore_fields": {
          "ignore_stdout": true
        }
      },
      "cleanup_commands": [
        "rm -rf test_dir"
      ]
    }
  ]
}
```

3. 带资源限制的测试：
```json
{
  "tests": [
    {
      "command": "sort",
      "description": "测试大文件排序（带资源限制）",
      "args": ["large_file.txt"],
      "setup_commands": [
        "dd if=/dev/urandom bs=1M count=10 of=large_file.txt"
      ],
      "environment": {
        "resource_limits": {
          "cpu_time": 5,
          "memory_size": 52428800
        }
      },
      "expectation": {
        "execution": {
          "exit_code": 0
        }
      },
      "cleanup_commands": [
        "rm -f large_file.txt"
      ],
      "timeout": 10
    }
  ]
}
```

## 使用方法

1. 基本用法：
```bash
cargo run -p compat_test -- --syskits-path target/debug/syskits <command>
```

2. 详细模式：
```bash
cargo run -p compat_test -- --syskits-path target/debug/syskits <command> -v
```

3. 指定 GNU Coreutils 路径：
```bash
cargo run -p compat_test -- --syskits-path target/debug/syskits --coreutils-path /usr/bin <command>
```

## 配置文件

支持通过 `.compat_test.toml` 配置文件设置默认参数：

```toml
[syskits]
syskits_path = "target/debug/syskits"
coreutils_path = "/usr/bin"
mode = "single"
commands_dir = "target/debug"

[test]
test_cases_dir = "test_cases"
default_commands = ["timeout", "pathchk", "cat"]

[test.env]
show_diff = true
default_timeout = 30
cleanup = true
show_progress = true
report_format = "text"
report_dir = "test_reports"
verbose = false
```

## 开发指南

1. 添加新的测试用例：
   - 在 `test_cases` 目录下创建对应的 JSON 文件
   - 遵循测试用例格式规范
   - 包含必要的环境准备和清理命令

2. 扩展测试框架：
   - 遵循模块化设计原则
   - 保持向后兼容性
   - 添加适当的单元测试

3. 调试技巧：
   - 使用 `-v` 参数查看详细输出
   - 设置 `cleanup = false` 保留测试文件
   - 查看测试报告了解失败原因

## 许可证

本项目采用与 Syskits 相同的许可证条款。 