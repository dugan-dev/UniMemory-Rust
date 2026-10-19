# Build and integration / 构建与接入

## Git dependency / Git 依赖

```toml
[dependencies]
unimemory = { git = "https://github.com/dugan-dev/UniMemory-Rust" }
```

Cargo fetches the repository and its submodules. Commit the application's Cargo.lock to pin the resolved revision. This package is not published to crates.io. For a private repository, configure Git authentication; if needed, set `CARGO_NET_GIT_FETCH_WITH_CLI=true` so Cargo uses your existing Git credentials.

Cargo 会获取仓库及子模块。提交应用的 Cargo.lock 即可锁定版本。本库尚未发布到 crates.io；私人仓库需要 Git 访问权限，必要时设置 `CARGO_NET_GIT_FETCH_WITH_CLI=true` 复用本机 Git 凭据。

To reuse Git authentication for future updates, add this to the application's `.cargo/config.toml`.

需要持续复用 Git 认证时，在应用的 `.cargo/config.toml` 中添加：

```toml
[net]
git-fetch-with-cli = true
```

## Local checkout / 本地源码

```sh
git clone --recurse-submodules https://github.com/dugan-dev/UniMemory-Rust.git
cd UniMemory-Rust
cargo run --example basic
cargo run --example containers --features mimalloc
cargo test --locked
```

For an existing clone / 已有克隆：

```sh
git submodule update --init --recursive
```

For a local dependency / 使用本地依赖：

```toml
[dependencies]
unimemory = { path = "../UniMemory-Rust" }
```

## Features / 功能选项

| Feature | Effect / 作用 |
| --- | --- |
| None / 无后端选项 | Use the application's Rust global allocator / 使用应用当前 Rust 分配器 |
| `rust` | Explicit Rust mode; rejects native backend features / 固定 Rust，禁止混用其他后端 |
| `standard` | C++ operator new/delete |
| `mimalloc` | Native mimalloc / 原生 mimalloc |
| `jemalloc` | Native jemalloc / 原生 jemalloc |
| `containers` | Stable allocator-aware Vec/Box; default on / 默认编译容器支持 |
| `statistics` | Compile optional tracking; default on, resources start disabled / 默认编译统计支持，对象默认关闭统计 |
| `benchmark` | Native benchmark controls; for measurements only / 仅用于基准测量 |

Select at most one backend. Cargo unifies features across dependencies, so all components must agree. Change `features = ["mimalloc"]` to switch backend. To remove statistics code while retaining containers:

同一应用仅选择一个后端，依赖之间应保持一致。修改 `features` 即可切换。保留容器并完全移除统计代码：

```toml
unimemory = { git = "https://github.com/dugan-dev/UniMemory-Rust", default-features = false, features = ["mimalloc", "containers"] }
```

## Native toolchains / 原生构建环境

Rust mode needs only Rust. Standard needs a C++17 compiler. mimalloc needs a C compiler. Unix jemalloc needs a C compiler, make and autoconf. SDK sources are pinned Git submodules; allocator algorithms are not reimplemented.

Rust 后端仅需 Rust；Standard 需要 C++17 编译器；mimalloc 需要 C 编译器；Unix jemalloc 需要 C 编译器、make 和 autoconf。第三方源码使用固定提交的 Git 子模块。

Windows MSVC jemalloc uses a separate `je_`-prefixed SDK. With a bootstrapped vcpkg checkout:

Windows MSVC 的 jemalloc 使用独立 SDK。先准备已初始化的 vcpkg，再执行：

```powershell
$env:VCPKG_ROOT = 'C:/path/to/vcpkg'
./tools/native-jemalloc.ps1
$env:UNIMEMORY_JEMALLOC_DIR = (Resolve-Path artifacts/native-sdk/x64-windows-static-md).Path
cargo test --locked --features jemalloc
```

The script builds a static allocator with dynamic MSVC CRT; it keeps C++ allocation overrides disabled. Existing prefixed SDKs are also supported: point `UNIMEMORY_JEMALLOC_DIR` to the installation prefix and add `bin` to PATH when using a dynamic library. Native and wrapped benchmark pairs use the same SDK.

脚本构建静态库、使用动态 MSVC CRT，并关闭 C++ 分配符号覆盖。也可通过 `UNIMEMORY_JEMALLOC_DIR` 接入现有 SDK；动态库需将 `bin` 加入 PATH。

## Global allocation / 标准容器的全局分配器

```rust
#[global_allocator]
static ALLOCATOR: unimemory::NativeAllocator = unimemory::NativeAllocator;
```

This selects the backend for standard Vec, String, Box and Arc. In Rust mode, this explicit registration uses Rust System to avoid recursion. If the application already registers an allocator, keep that registration and use Rust-mode resources to reuse it.

这会让标准 Vec、String、Box、Arc 使用所选后端。Rust 模式下显式注册时使用 System，避免递归。应用已有全局分配器时保留原注册，Rust 后端会复用它。

## Checks and benchmarks / 验证与测量

```sh
python tools/validate.py
uv run --with matplotlib==3.10.7 python tools/benchmark.py
```

The validator checks all four backends, examples and independent consumers. Native toolchains/SDKs must be installed first. Benchmark output goes to `docs/results` and `docs/images`; build products and logs go to ignored `target` and `artifacts`. Results are advisory, never a correctness gate.

完整验证覆盖四后端、示例和独立消费项目；请先准备原生构建环境。性能数据与图片保存在 `docs/results` 和 `docs/images`，构建产物与日志不纳入 Git。性能不作为正确性门槛。

## Repository layout / 目录

```text
.github/    GitHub Actions
src/        Rust public API and private implementation / 接口与实现
native/     Standard C++ ABI bridge / Standard 桥接
external/   Pinned Git submodules / 第三方子模块
examples/   Runnable usage examples / 使用示例
tests/      Correctness and integration / 正确性与接入测试
benches/    Paired native/wrapped benchmarks / 原生与封装对照
tools/      Validation, SDK build and charts / 验证与图表工具
docs/       Guides, measured data and images / 文档、数据和图片
```
