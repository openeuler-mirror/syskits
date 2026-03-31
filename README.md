# syskits CTyunOS基础组件库
## 1. 目标

### 为ctshell 提供自研基本系统组件的lib和bin。
* lib 提供给ctshell 内嵌命令使用，结合ctshell提供的数据流操作逻辑，提供给用户良好体验。(属于rust 库源码引用)
* bin 提供给ctshell 作为优先调用命令。(属于rust 应用引用)

### 替换基础组件coreutils
* 功能层面达到替换基础组件coreutils能力
* 接口层面兼容coreutils


## 2. 开发逻辑

### 需求
* rust 版本1.70.0

### 构建逻辑 
* 默认构建

```shell
cargo build --release
```

* 平台构建
Windows平台计划支持，目的服务于ctshell在Windows使用

```shell
cargo build --release --features windows
```

```shell
cargo build --features unix
```

* 单个特性应用构建，例如: 构建ls命令(集成到syskits逻辑)

```shell
cargo build --features ls
```

* 单个特性应用构建，例如: 构建ls命令(单独可执行程序)

```shell
cargo build -p ct_ls
```

### 测试
* 全平台测试(提交代码前，必须通过此项)

```shell
cargo test --all -p ct
```

* 单个特性测试，例如: 测试ls

```shell
cargo test --all -p ct_ls
```

### 静态扫描
* check测试

```shell
cargo check --all
```

* clippy测试(提交代码前，必须通过此项)

```shell
cargo clippy --all-targets --all-features
```

### 代码格式化
提交代码前，必须格式化

```shell
cargo fmt --all
```

### Debug 测试
```shell
rust-gdb --args target/debug/syskits ls
(gdb) b ls.rs:79
(gdb) run
```

### 国际化和本地化
* 配置文件

locales/en-US/en-US.yml
提供英文帮助信息

locales/zh-CN/zh-CN.yml
提供中文帮助信息

* 支持中文和英文

* 支持语言设置/切换

```shell
unset LC_ALL && unset LANG && export LANG=en_US.UTF-8/zh_CN.UTF-8
```

*库及API接口

获取当前环境语言信息
sys-locale::get_locale()

设置当前环境语言信息
rust-i18n::set_locale()

字符串翻译
rust-i18n::t!(&str)
