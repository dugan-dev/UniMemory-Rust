"""Build four fixed backend profiles, run paired controls serially, retain data and draw charts."""

import argparse
import csv
import ctypes
import datetime
import hashlib
import io
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
BACKENDS = ("rust", "standard", "mimalloc", "jemalloc")


def run(command, *, env=None):
    result = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.returncode:
        raise RuntimeError(
            f"{' '.join(map(str, command))}\n{result.stdout}\n{result.stderr}"
        )
    return result


def fingerprint():
    files = [
        ROOT / "Cargo.toml",
        ROOT / "Cargo.lock",
        ROOT / "build.rs",
        *sorted((ROOT / "src").glob("*.rs")),
        *sorted((ROOT / "native").glob("*.cpp")),
        *sorted((ROOT / "benches").glob("*.rs")),
    ]
    return hashlib.sha256(
        b"".join(
            p.relative_to(ROOT).as_posix().encode() + b"\0" + p.read_bytes()
            for p in files
        )
    ).hexdigest()


def pin_cpu():
    if sys.platform == "win32":
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.GetCurrentProcess.restype = ctypes.c_void_p
        kernel.GetProcessAffinityMask.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(ctypes.c_size_t),
            ctypes.POINTER(ctypes.c_size_t),
        ]
        kernel.SetProcessAffinityMask.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        handle = kernel.GetCurrentProcess()
        allowed, system = ctypes.c_size_t(), ctypes.c_size_t()
        if not kernel.GetProcessAffinityMask(
            handle, ctypes.byref(allowed), ctypes.byref(system)
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        cpus = [i for i in range(64) if allowed.value & (1 << i)]
        cpu = cpus[min(2, len(cpus) - 1)]
        if not kernel.SetProcessAffinityMask(handle, 1 << cpu):
            raise ctypes.WinError(ctypes.get_last_error())
        return {"pinned": True, "logical_cpu": cpu}
    if hasattr(os, "sched_getaffinity"):
        cpus = sorted(os.sched_getaffinity(0))
        cpu = cpus[min(2, len(cpus) - 1)]
        os.sched_setaffinity(0, {cpu})
        return {"pinned": True, "logical_cpu": cpu}
    return {"pinned": False, "reason": "no supported process-affinity API"}


def render(summary, destination):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    import numpy as np
    from matplotlib import font_manager
    from matplotlib.text import Text

    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 10,
            "axes.spines.top": False,
            "axes.spines.right": False,
            "axes.titleweight": "bold",
        }
    )
    colors = {
        "rust": "#2563eb",
        "standard": "#64748b",
        "mimalloc": "#16a34a",
        "jemalloc": "#c76a12",
    }
    destination.mkdir(parents=True, exist_ok=True)
    chinese_font = next(
        (
            p
            for p in [
                Path("C:/Windows/Fonts/msyh.ttc"),
                Path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            ]
            if p.is_file()
        ),
        None,
    )

    def save(fig, name):
        fig.savefig(destination / f"{name}.png", dpi=180)
        fig.savefig(destination / f"{name}.svg")
        if chinese_font:
            mapping = {
                "Native": "原生",
                "UniMemory": "封装",
                "Rust": "Rust 默认",
                "Standard": "Standard (C++)",
                "Global 64 B": "Global · 64 B",
                "Zeroed 64 B": "清零分配 · 64 B",
                "Resize 64→512 B": "扩容 · 64→512 B",
                "Heap 64 B": "Heap · 64 B",
                "Object 64 B": "创建与销毁对象 · 64 B",
                "Vector 256 × u64": "容器 · 256 个 u64",
                "Stack 64 B": "Stack · 64 B",
                "ns / complete operation · lower is faster": "ns / 完整操作 · 越低越快",
                "ns / allocate + write + free · log scale": "ns / 分配 + 写入 + 释放（对数轴）",
                "Global allocation · identical 8-byte alignment": "Global 分配 · 统一 8 字节对齐",
                "Relative throughput · 100% = native speed\nHigher is better; timing is advisory": "相对原生吞吐 · 100% 表示相同速度\n数值越高越好，性能仅供参考",
                "Native time / UniMemory time × 100%": "原生耗时 / 封装耗时 × 100%",
            }

            def translate(value):
                if value.startswith("Native vs UniMemory Rust"):
                    value = f"原生与 UniMemory Rust 对比 · {summary['platform']}\nRelease · 统计关闭 · {summary['runs']} 个进程 × 9 组交错试验的中位数"
                else:
                    value = mapping.get(value, value)
                    value = (
                        value.replace(" native", " 原生")
                        .replace(" UniMemory", " 封装")
                        .replace(" · align ", " · 对齐 ")
                    )
                    labels = {
                        "global": "全局分配",
                        "heap": "独立分配组",
                        "stack": "固定缓冲区",
                        "object": "对象",
                        "vector": "容器",
                        "zeroed": "清零分配",
                        "resize": "扩容",
                    }
                    for label, translated in labels.items():
                        if value.startswith(label + " · "):
                            value = translated + value[len(label) :]
                            break
                return value

            for text in fig.findobj(Text):
                text.set_text(translate(text.get_text()))
                text.set_fontproperties(
                    font_manager.FontProperties(
                        fname=str(chinese_font),
                        size=text.get_fontsize(),
                        weight=text.get_fontweight(),
                    )
                )
            for axis in fig.axes:
                for direction in ("x", "y"):
                    labels = getattr(axis, f"get_{direction}ticklabels")()
                    translated = [translate(label.get_text()) for label in labels]
                    if any(a != b.get_text() for a, b in zip(translated, labels)):
                        getattr(axis, f"set_{direction}ticks")(
                            getattr(axis, f"get_{direction}ticks")(), translated
                        )
            fig.savefig(destination / f"{name}.zh-CN.png", dpi=180)
            fig.savefig(destination / f"{name}.zh-CN.svg")
        plt.close(fig)
        for path in destination.glob(f"{name}*.svg"):
            path.write_text(
                "\n".join(
                    line.rstrip()
                    for line in path.read_text(encoding="utf-8").splitlines()
                )
                + "\n",
                encoding="utf-8",
                newline="\n",
            )

    rows = summary["rows"]
    cases = [
        ("global", 64, 8, "Global 64 B"),
        ("zeroed", 64, 16, "Zeroed 64 B"),
        ("resize", 512, 16, "Resize 64→512 B"),
        ("heap", 64, 16, "Heap 64 B"),
        ("object", 64, 8, "Object 64 B"),
        ("vector", 2048, 8, "Vector 256 × u64"),
        ("stack", 64, 16, "Stack 64 B"),
    ]
    indexed = {(r["backend"], r["case"], r["bytes"], r["alignment"]): r for r in rows}
    fig, axes = plt.subplots(2, 2, figsize=(13, 8), constrained_layout=True)
    for backend, axis in zip(BACKENDS, axes.flat):
        points = [indexed[(backend, c, n, a)] for c, n, a, _ in cases]
        y = np.arange(len(points))
        axis.barh(
            y - 0.18,
            [r["native_ns"] for r in points],
            height=0.33,
            color=colors[backend],
            alpha=0.3,
            label="Native",
        )
        axis.barh(
            y + 0.18,
            [r["unified_ns"] for r in points],
            height=0.33,
            color=colors[backend],
            label="UniMemory",
        )
        axis.set_yticks(y, [c[3] for c in cases])
        axis.invert_yaxis()
        axis.set_xlabel("ns / complete operation · lower is faster")
        axis.set_title(backend.capitalize())
        axis.grid(axis="x", alpha=0.15)
        axis.set_axisbelow(True)
        axis.legend(frameon=False, loc="lower right")
    fig.suptitle(
        f"Native vs UniMemory Rust · {summary['platform']}\nRelease · statistics OFF · median of {summary['runs']} processes × 9 paired trials",
        fontsize=15,
    )
    save(fig, "workloads")

    fig, axis = plt.subplots(figsize=(11, 5.5), constrained_layout=True)
    sizes = [16, 64, 256, 4096, 65536]
    for backend in BACKENDS:
        points = [indexed[(backend, "global", n, 8)] for n in sizes]
        axis.plot(
            sizes,
            [r["native_ns"] for r in points],
            "--o",
            color=colors[backend],
            alpha=0.5,
            label=f"{backend} native",
        )
        axis.plot(
            sizes,
            [r["unified_ns"] for r in points],
            "-o",
            color=colors[backend],
            label=f"{backend} UniMemory",
        )
    axis.set_xscale("log", base=2)
    axis.set_yscale("log")
    axis.set_xticks(sizes, ["16 B", "64 B", "256 B", "4 KiB", "64 KiB"])
    axis.set_ylabel("ns / allocate + write + free · log scale")
    axis.set_title("Global allocation · identical 8-byte alignment")
    axis.grid(alpha=0.15)
    axis.legend(ncol=2, frameon=False, fontsize=9)
    save(fig, "global")

    keys = sorted(
        {(r["case"], r["bytes"], r["alignment"]) for r in rows},
        key=lambda k: (k[0], k[1], k[2]),
    )
    matrix = np.array(
        [
            [
                indexed[(backend, *key)]["native_over_unified_percent"]
                for backend in BACKENDS
            ]
            for key in keys
        ]
    )
    fig, axis = plt.subplots(figsize=(8, 10), constrained_layout=True)
    picture = axis.imshow(matrix, vmin=50, vmax=110, cmap="RdYlGn", aspect="auto")
    axis.set_xticks(range(4), [b.capitalize() for b in BACKENDS])
    axis.set_yticks(range(len(keys)), [f"{c} · {n} B · align {a}" for c, n, a in keys])
    for y in range(len(keys)):
        for x in range(4):
            axis.text(
                x, y, f"{matrix[y, x]:.1f}%", ha="center", va="center", fontsize=10
            )
    axis.set_title(
        "Relative throughput · 100% = native speed\nHigher is better; timing is advisory"
    )
    fig.colorbar(
        picture, ax=axis, shrink=0.6, label="Native time / UniMemory time × 100%"
    )
    save(fig, "relative")


def write_report(summary, results):
    indexed = {
        (r["backend"], r["case"], r["bytes"], r["alignment"]): r
        for r in summary["rows"]
    }
    keys = sorted({(r["case"], r["bytes"], r["alignment"]) for r in summary["rows"]})
    text = [
        "# Performance / 性能分析",
        "",
        f"{summary['platform']} · Release · 统计编译关闭 · 每项 {summary['runs']} 个独立进程 × 9 次交错原生/封装试验。单位 ns/完整操作，越低越快。",
        "",
        "## Workloads / 常见操作",
        "",
        "浅色为原生，深色为 UniMemory Rust。",
        "",
        "![Native and unified workloads](images/workloads.png)",
        "",
        "## Sizes / 分配规模",
        "",
        "固定 8 字节对齐；虚线原生，实线封装。",
        "",
        "![Global sizes](images/global.png)",
        "",
        "## Relative throughput / 相对原生吞吐",
        "",
        "100% 表示相同耗时；高于 100% 也可能来自采样和代码布局差异，不代表封装普遍比原生更快。",
        "",
        "![Relative throughput](images/relative.png)",
        "",
        "## Complete table / 全部数据",
        "",
        "每格为 **原生 / 封装** 的中位数，覆盖所有已测项目。",
        "",
        "| 操作 · 字节 · 对齐 | Rust | Standard | mimalloc | jemalloc |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for key in keys:
        cells = [
            f"{indexed[(backend, *key)]['native_ns']:.2f} / {indexed[(backend, *key)]['unified_ns']:.2f}"
            for backend in BACKENDS
        ]
        text.append(f"| {key[0]} · {key[1]} · {key[2]} | " + " | ".join(cells) + " |")
    text += [
        "",
        "## Scope / 范围",
        "",
        "- Rust：当前应用全局分配器；本次基准未替换它。Standard：C++ new/delete；不是 Rust System 的别名。",
        "- mimalloc 3.3.2、jemalloc 5.3.1。原生和封装共享同一份 SDK 和相同参数；Windows 原生库统一静态链接、动态 MSVC CRT。",
        "- Heap：mimalloc/ jemalloc 对比原生 heap/arena；Rust/Standard 没有同等原生分配组，基线为普通分配，封装额外提供分组回收。",
        "- Stack：对比最小固定缓冲区控制，封装另有容量、检查点和生命周期规则；不调用上述原生堆后端。",
        "- vector：两侧均使用 allocator-api2 Vec、相同预留、填充和销毁；原生侧直接调用选定后端，不混用默认 Vec。",
        "- 同一可执行文件中先预热再交错计时；多进程后端顺序固定种子打乱；记录 CPU 绑定、源码/二进制/SDK 哈希和完整试验。",
        "- 这些是本机微基准，不包括 RSS、GPU、长时间压力或跨平台性能结论。性能不是正确性门槛。",
        "",
        "[原始试验](results/samples.csv) · [汇总表](results/summary.csv) · [环境与版本](results/benchmark.json)",
        "",
        "## Reproduce / 复现",
        "",
        "```sh",
        "uv run --with matplotlib==3.10.7 python tools/benchmark.py",
        "```",
        "",
        "Windows 先运行 tools/native-jemalloc.ps1；原生 SDK 二进制保存在忽略的 artifacts/ 目录。",
        "",
    ]
    (results.parent / "performance.md").write_text("\n".join(text), encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--iterations", type=int, default=200_000)
    parser.add_argument("--jemalloc-prefix", type=Path)
    parser.add_argument("--render-only", action="store_true")
    args = parser.parse_args()
    results = ROOT / "docs/results"
    results.mkdir(parents=True, exist_ok=True)
    artifacts = ROOT / "artifacts/benchmark"
    artifacts.mkdir(parents=True, exist_ok=True)
    output = results / "benchmark.json"
    if args.render_only:
        render(json.loads(output.read_text()), ROOT / "docs/images")
        return
    if args.runs < 1 or args.iterations < 1:
        raise ValueError("positive runs and iterations required")
    environment = dict(
        os.environ,
        UNIMEMORY_BENCH_ITERATIONS=str(args.iterations),
        RUSTFLAGS="-C target-cpu=native",
    )
    prefix = args.jemalloc_prefix
    if sys.platform == "win32":
        prefix = prefix or ROOT / "artifacts/native-sdk/x64-windows-static-md"
        if not (prefix / "lib/jemalloc_s.lib").is_file():
            raise RuntimeError(
                "Use a static /MD jemalloc SDK for matched linking; run tools/native-jemalloc.ps1 first"
            )
        environment["UNIMEMORY_JEMALLOC_DIR"] = str(prefix.resolve())
    binaries = {}
    for backend in BACKENDS:
        print(f"Building {backend} Release profile", flush=True)
        features = "benchmark" + (f",{backend}" if backend != "rust" else "")
        command = [
            "cargo",
            "bench",
            "--locked",
            "--bench",
            "allocation",
            "--no-default-features",
            "--features",
            features,
            "--no-run",
            "--message-format=json",
        ]
        result = run(command, env=environment)
        (artifacts / f"build-{backend}.jsonl").write_text(result.stdout)
        (artifacts / f"build-{backend}.log").write_text(result.stderr)
        for line in result.stdout.splitlines():
            entry = json.loads(line)
            if entry.get("reason") == "compiler-artifact" and entry.get("executable"):
                binaries[backend] = Path(entry["executable"])
        if backend not in binaries:
            raise RuntimeError(f"no benchmark executable: {backend}")
    source_hash = fingerprint()
    hashes = {
        name: hashlib.sha256(p.read_bytes()).hexdigest() for name, p in binaries.items()
    }
    affinity = pin_cpu()
    samples = []
    order = []
    rng = random.Random(20261019)
    for process_round in range(args.runs):
        backends = list(BACKENDS)
        rng.shuffle(backends)
        order.append(backends)
        for backend in backends:
            print(
                f"Measuring {backend}: process {process_round + 1}/{args.runs}",
                flush=True,
            )
            result = run([str(binaries[backend])], env=environment)
            (artifacts / f"{backend}-{process_round}.csv").write_text(result.stdout)
            (artifacts / f"{backend}-{process_round}.log").write_text(result.stderr)
            rows = list(csv.DictReader(io.StringIO(result.stdout)))
            if len(rows) != 16 * 9:
                raise RuntimeError(f"incomplete {backend} data: {len(rows)}")
            for row in rows:
                if row["backend"] != backend:
                    raise RuntimeError("backend mismatch")
                for key in ("native_ns", "unified_ns"):
                    if not math.isfinite(float(row[key])) or float(row[key]) <= 0:
                        raise RuntimeError("invalid timing")
                row["process"] = process_round
                samples.append(row)
    if fingerprint() != source_hash:
        raise RuntimeError("source changed during measurement")
    if any(
        hashlib.sha256(p.read_bytes()).hexdigest() != hashes[name]
        for name, p in binaries.items()
    ):
        raise RuntimeError("binary changed during measurement")
    with (results / "samples.csv").open("w", newline="", encoding="utf-8") as file:
        writer = csv.DictWriter(file, fieldnames=list(samples[0]))
        writer.writeheader()
        writer.writerows(samples)
    grouped = {}
    for row in samples:
        key = row["backend"], row["case"], int(row["bytes"]), int(row["alignment"])
        grouped.setdefault(key, []).append(row)
    rows = []
    for (backend, case, size, alignment), values in sorted(grouped.items()):
        native = [float(v["native_ns"]) for v in values]
        unified = [float(v["unified_ns"]) for v in values]
        n, u = statistics.median(native), statistics.median(unified)
        rows.append(
            {
                "backend": backend,
                "case": case,
                "bytes": size,
                "alignment": alignment,
                "samples": len(values),
                "native_ns": n,
                "unified_ns": u,
                "native_over_unified_percent": n / u * 100,
                "native_range_ns": [min(native), max(native)],
                "unified_range_ns": [min(unified), max(unified)],
            }
        )
    pins = (
        run(["git", "submodule", "status", "--recursive"]).stdout.strip().splitlines()
    )
    sdk_hash = (
        hashlib.sha256((prefix / "lib/jemalloc_s.lib").read_bytes()).hexdigest()
        if prefix
        else None
    )
    summary = {
        "platform": f"{platform.system()} {platform.machine()}",
        "processor": platform.processor(),
        "date": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "rustc": run(["rustc", "-vV"]).stdout,
        "profile": "Release opt-level=3, thin LTO, codegen-units=1, target-cpu=native, statistics compiled OFF",
        "native_linkage": "static native libraries with dynamic MSVC CRT"
        if sys.platform == "win32"
        else "static native libraries",
        "source_sha256": source_hash,
        "binary_sha256": hashes,
        "jemalloc_sdk_sha256": sdk_hash,
        "submodules": pins,
        "affinity": affinity,
        "runs": args.runs,
        "iterations": args.iterations,
        "backend_order": order,
        "rows": rows,
    }
    output.write_text(json.dumps(summary, indent=2) + "\n")
    with (results / "summary.csv").open("w", newline="", encoding="utf-8") as file:
        fields = [k for k in rows[0] if not k.endswith("range_ns")]
        writer = csv.DictWriter(file, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    write_report(summary, results)
    render(summary, ROOT / "docs/images")
    print(
        f"Saved {len(rows)} comparisons and {len(samples)} paired samples", flush=True
    )


if __name__ == "__main__":
    main()
