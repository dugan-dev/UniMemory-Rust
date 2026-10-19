# 验证记录

本地 Windows x64 / MSVC，Rust 1.94.0。以下均已通过。

| 范围 | 结果 |
| --- | --- |
| Rust / Standard / mimalloc / jemalloc | Debug、Release、关闭统计、Clippy、示例和独立消费项目 |
| 完整验证脚本 | 36 项检查，520 次测试执行 |
| 性能测量的实际编译配置 | 四后端合计 125 次测试执行，Clippy 无警告 |
| Miri 严格来源检查 | 37 项运行测试，通过 Linux 目标解释执行 |
| Rust 1.85 最低版本 | 四后端均通过 |
| 私人仓库 Git 接入 | 递归获取子模块，四后端编译运行通过 |
| README 与 API 文档 | 四后端共 12 次示例执行通过；文档构建无警告 |
| 原生与封装性能 | 64 项对比，1,728 组配对数据；性能不作为正确性门槛 |

覆盖：零大小、对齐、容量边界、扩缩容失败保留、类型和数组析构、构造/析构异常、所有权借用、整体释放、检查点失效、统计平衡、跨线程分配释放、混合分配压力、容器适配，以及已有应用全局分配器的兼容接入。编译失败用例验证禁止提前回收、资源越界借用和不安全的普通字节缓冲区。

测试次数包含不同配置下的重复执行，并非不同用例数量。Miri 仅验证 Rust 实现，不能执行原生 C/C++ 分配器。最低支持版本 Rust 1.85 已完成四后端本地验证。

GitHub Actions 覆盖 Windows、Linux、macOS × 四后端，以及 Miri、Rust 1.85。远程结果以 [构建记录](https://github.com/dugan-dev/UniMemory-Rust/actions/workflows/build.yml) 为准；本页记录本地验证，不将任务配置视为远程通过证明。

C++ UniMemory 和第三方子模块均未修改。源码指纹与本次性能记录一致。

[机器可读记录](results/validation.json) · [性能分析](performance.md)
