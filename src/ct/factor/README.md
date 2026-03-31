# CT Factor

因式分解库，提供高效的整数因式分解功能。

## 功能特性

- 高效的 Miller-Rabin 素性测试
- 优化的 Pollard's Rho 算法用于因式分解
- 支持大整数因式分解
- 提供优化版实现，使用 num-prime 和 num-modular 库

## 性能测试

本项目提供了几个性能测试程序，用于比较原始实现和优化实现的性能差异：

### 因式分解性能测试

```bash
cargo run --release --bin perf_test
```

这个测试程序会比较原始因式分解实现和优化实现在不同大小整数上的性能差异。

### Miller-Rabin 素性测试性能测试

```bash
cargo run --release --bin prime_test
```

这个测试程序会比较原始 Miller-Rabin 素性测试实现和优化实现在不同大小整数上的性能差异。

### Pollard's Rho 算法性能测试

```bash
cargo run --release --bin rho_test
```

这个测试程序会比较原始 Pollard's Rho 算法实现和优化实现在不同大小合数上的性能差异。

## 基准测试

如果你安装了 Rust nightly 版本，可以运行基准测试：

```bash
rustup run nightly cargo bench
```

## 许可证

本项目采用 Mulan PSL v2 许可证。 