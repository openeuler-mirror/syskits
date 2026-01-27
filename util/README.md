# GNU兼容性测试指南

此文档用于帮助您了解如何运行本项目的GNU兼容性测试。

---

## 1. 解压GNU源码

解压本目录的coreutils-9.9.tar.gz至syskits的上一级目录：

```bash
# 当前目录为xxx/syskits/util
tar -xzf coreutils-9.9.tar.gz -C ../..
# 重命名为gnu
mv ../../coreutils-9.9 ../../gnu
```

---

## 2. 运行测试脚本

运行如下的构建和测试脚本：

```bash
# 构建
bash build-gnu.sh
# 测试
bash run-gnu-test.sh
```

---

## 3. 生成测试数据以及可视化

您可以使用如下的命令生成测试结果数据：

```bash
python3 gen_test_result.py
# 使用结果数据绘制可视化页面（如果需要的话）
python3 gen_html.py
```