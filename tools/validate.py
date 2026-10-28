"""Validate one or all backends, without running third-party SDK test suites."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--backend",
        choices=("all", "standard", "mimalloc", "jemalloc"),
        default="all",
    )
    parser.add_argument("--jemalloc-prefix", type=Path)
    args = parser.parse_args()
    backends = (
        ["standard", "mimalloc", "jemalloc"]
        if args.backend == "all"
        else [args.backend]
    )
    artifacts = ROOT / "artifacts/validation"
    artifacts.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    if args.jemalloc_prefix:
        env["UNIMEMORY_JEMALLOC_DIR"] = str(args.jemalloc_prefix.resolve())
    if os.name == "nt":
        prefix = Path(
            env.get(
                "UNIMEMORY_JEMALLOC_DIR",
                ROOT / "artifacts/native-sdk/x64-windows-static-md",
            )
        )
        if (prefix / "lib/jemalloc_s.lib").is_file():
            env["UNIMEMORY_JEMALLOC_DIR"] = str(prefix.resolve())
        elif "UNIMEMORY_JEMALLOC_DIR" in env:
            env["PATH"] = str(prefix / "bin") + os.pathsep + env["PATH"]
        elif env.get("VCPKG_ROOT"):
            env["PATH"] = (
                str(Path(env["VCPKG_ROOT"]) / "installed/x64-windows-unimemory/bin")
                + os.pathsep
                + env["PATH"]
            )
    results = []

    def run(label, command, extra_env=None):
        print(label, flush=True)
        result = subprocess.run(
            command,
            cwd=ROOT,
            env=extra_env or env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        (artifacts / f"{label}.log").write_text(
            result.stdout + result.stderr, encoding="utf-8"
        )
        if result.returncode:
            print(result.stdout + result.stderr, flush=True)
            raise RuntimeError(f"{label} failed ({result.returncode})")
        totals = re.findall(
            r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored",
            result.stdout,
        )
        results.append(
            {
                "check": label,
                "passed": True,
                "tests": sum(int(t[0]) for t in totals),
                "ignored": sum(int(t[2]) for t in totals),
            }
        )

    run("format", ["cargo", "fmt", "--package", "unimemory", "--check"])
    for backend in backends:
        flags = ["--features", backend] if backend != "standard" else []
        run(f"{backend}-debug", ["cargo", "test", "--locked", *flags])
        run(f"{backend}-release", ["cargo", "test", "--locked", "--release", *flags])
        features = "containers" + (f",{backend}" if backend != "standard" else "")
        run(
            f"{backend}-statistics-off",
            [
                "cargo",
                "test",
                "--locked",
                "--no-default-features",
                "--features",
                features,
            ],
        )
        run(
            f"{backend}-clippy",
            [
                "cargo",
                "clippy",
                "--locked",
                "--all-targets",
                *flags,
                "--",
                "-D",
                "warnings",
            ],
        )
        for example in ("basic", "containers", "global_allocator"):
            run(
                f"{backend}-example-{example}",
                ["cargo", "run", "--locked", "--example", example, *flags],
            )
        consumer_env = dict(env, CARGO_TARGET_DIR=str(ROOT / "target/consumer"))
        run(
            f"{backend}-consumer",
            [
                "cargo",
                "run",
                "--manifest-path",
                "tests/consumer/Cargo.toml",
                *flags,
            ],
            consumer_env,
        )
        if backend == "standard":
            run(
                "standard-existing-allocator",
                [
                    "cargo",
                    "run",
                    "--manifest-path",
                    "tests/existing-allocator/Cargo.toml",
                ],
                consumer_env,
            )
    run("minimal", ["cargo", "test", "--locked", "--no-default-features"])
    invalid = subprocess.run(
        ["cargo", "check", "--locked", "--features", "standard,mimalloc"],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
    )
    if invalid.returncode == 0 or "Select exactly one backend" not in invalid.stderr:
        raise RuntimeError("incompatible backend selection was not rejected")
    results.append(
        {
            "check": "reject-conflicting-backends",
            "passed": True,
            "tests": 1,
            "ignored": 0,
        }
    )
    summary = {
        "backends": backends,
        "checks": results,
        "test_executions": sum(r["tests"] for r in results),
    }
    (artifacts / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(
        json.dumps(
            {
                "checks": len(results),
                "test_executions": summary["test_executions"],
                "passed": True,
            }
        )
    )


if __name__ == "__main__":
    main()
