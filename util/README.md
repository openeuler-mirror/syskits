# GNU兼容性测试指南

此文档用于帮助您了解如何运行本项目的GNU兼容性测试。

---

## 1. 解压GNU源码

解压本目录的coreutils-9.4.tar.gz至syskits的上一级目录：

```bash
# 当前目录为xxx/syskits/util
tar -xzf coreutils-9.4.tar.gz -C ../..
# 重命名为gnu
mv ../../coreutils-9.4 ../../gnu
```

---

## 2. 运行测试脚本

运行如下的构建和测试脚本：

```bash
# 以root用户构建
FORCE_UNSAFE_CONFIGURE=1 bash build-gnu.sh

# 以非root用户构建
bash build-gnu.sh

# 全量测试
bash run-gnu-test.sh

# 单个命令功能点测试 例如 tail-c 对应的测试脚本为tail-c.sh, 它对应的就是上级目录下gnu目录tests/tail/下面的tail-c.sh测试脚本
DEBUG=1 bash run-gnu-test.sh tests/tail/tail-c.sh
```


---

## 3. 生成测试数据以及可视化

`gen_test_result.py` 支持以下能力：

- 列出 `gnu/tests` 下的测试组件和测试用例
- 从 `util/aggregated-result.json` 列出已有测试结果
- 运行测试并生成 `aggregated-result.json`
- 根据结果生成 `test_coverage.html`

建议在 `util` 目录下执行以下命令。

### 3.1 列出测试组件和用例

如需查看当前 GNU 测试组件和用例列表，可执行：

```bash
# 列出 gnu/tests 下的测试组件以及其下所有 .sh / .pl 用例的绝对路径
python3 gen_test_result.py --list-tests

# 只列出某个测试组件，例如 tail
python3 gen_test_result.py --list-tests tail
```

输出会按测试组件分组展示：

- 测试组件名
- 测试组件目录绝对路径
- 该组件下每个测试用例的绝对路径

组件过滤支持以下形式：

```bash
# 组件名
python3 gen_test_result.py --list-tests tail

# 相对路径
python3 gen_test_result.py --list-tests tests/tail

# 绝对路径
python3 gen_test_result.py --list-tests /mnt/mac/Users/qiny/codespace/gnu/tests/tail
```

### 3.2 列出已有测试结果

从 `util/aggregated-result.json` 列出结果，可执行：

```bash
# 从 util/aggregated-result.json 列出全部测试结果
python3 gen_test_result.py --list-results

# 只列出某个测试组件的结果，例如 tail
python3 gen_test_result.py --list-results tail

# 只列出 FAIL 结果
python3 gen_test_result.py --list-results --fail

# 只列出 tail 组件中的 FAIL / ERROR 结果
python3 gen_test_result.py --list-results tail --fail --error
```

`--list-results` 输出会按测试组件分组展示：

- 测试组件名
- 测试组件目录绝对路径
- 每个测试项的结果状态
- 测试文件绝对路径
- 日志文件绝对路径

结果过滤说明：

- `--pass` 仅显示 `PASS`
- `--fail` 仅显示 `FAIL`
- `--skip` 仅显示 `SKIP`
- `--error` 仅显示 `ERROR`
- 多个过滤 flag 可以叠加使用，含义为“或”关系
- 结果过滤 flag 只能和 `--list-results` 一起使用

### 3.3 运行测试并生成结果 / HTML

您可以使用如下命令完成“运行测试 -> 生成结果 JSON -> 生成 HTML”的闭环：

```bash
# 仅基于现有日志生成结果
python3 gen_test_result.py

# 先跑全量测试，再生成结果
python3 gen_test_result.py --run

# 先跑单个功能点测试，再生成结果
python3 gen_test_result.py --run tests/tail/tail-c.sh

# 单个功能点测试并开启 DEBUG=1
python3 gen_test_result.py --run --debug tests/tail/tail-c.sh
```

该脚本会同时生成：

- `aggregated-result.json`
- `test_coverage.html`

生成的结果数据中，每个测试项除了状态外，还会包含：

- 测试文件绝对路径
- 日志文件绝对路径

HTML 页面会同时展示这两个绝对路径，并以可点击链接形式输出，便于直接跳转查看本地文件。

当指定测试路径时，生成的 JSON 和 HTML 只展示本次指定测试对应的日志结果。

### 3.4 结果文件说明

生成的 `aggregated-result.json` 结构示例如下：

```json
{
  "tail": {
    "tail-c.log": {
      "status": "FAIL",
      "test_file_path": "/abs/path/to/gnu/tests/tail/tail-c.sh",
      "log_file_path": "/abs/path/to/gnu/tests/tail/tail-c.log"
    }
  }
}
```
