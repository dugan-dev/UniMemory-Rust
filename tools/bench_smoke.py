"""Run the same-binary benchmark and reject catastrophic overheads.

The benchmark compares a direct backend call with the wrapped resource API for
the same workload, so the ratio is a regression guard rather than a speed claim.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BACKENDS = ("standard", "mimalloc", "jemalloc")
FLOOR = 20.0
LINE = re.compile(
    r"^(?P<backend>\w+) (?P<case>\w+) (?P<bytes>\d+)B: "
    r"native (?P<native>[\d.]+)ns / API (?P<api>[\d.]+)ns \((?P<ratio>[\d.]+)%\)$",
    re.MULTILINE,
)


def main() -> int:
    if len(sys.argv) != 2 or sys.argv[1] not in BACKENDS:
        print(f"usage: {Path(sys.argv[0]).name} {'|'.join(BACKENDS)}", file=sys.stderr)
        return 2
    backend = sys.argv[1]
    iterations = os.environ.get("UNIMEMORY_BENCH_ITERATIONS", "20000")
    features = "benchmark" if backend == "standard" else f"benchmark,{backend}"
    env = dict(os.environ, UNIMEMORY_BENCH_ITERATIONS=iterations, CARGO_TERM_COLOR="never")
    command = ["cargo", "bench", "--no-default-features", "--features", features]
    print(" ".join(command), flush=True)
    result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stdout[-4000:], result.stderr[-4000:], file=sys.stderr)
        print(f"benchmark failed ({result.returncode})", file=sys.stderr)
        return 1

    matches = list(LINE.finditer(result.stderr))
    if not matches:
        print(result.stderr[-4000:], file=sys.stderr)
        print("benchmark produced no cases", file=sys.stderr)
        return 1

    destination = ROOT / "artifacts/benchmarks"
    destination.mkdir(parents=True, exist_ok=True)
    (destination / f"{backend}.csv").write_text(result.stdout, encoding="utf-8")
    (destination / f"{backend}.log").write_text(result.stderr, encoding="utf-8")

    failures = []
    for match in matches:
        ratio = float(match.group("ratio"))
        print(
            f"{backend:9} {match.group('case'):7} {match.group('bytes'):>6} B  "
            f"{float(match.group('native')):8.2f} -> {float(match.group('api')):8.2f} ns  {ratio:6.1f}%"
        )
        if ratio < FLOOR:
            failures.append((match.group("case"), ratio))

    if failures:
        for case, ratio in failures:
            print(f"overhead guard: {case} at {ratio:.1f}% < {FLOOR:.0f}%", file=sys.stderr)
        return 1
    print(f"benchmark ok: {len(matches)} cases, floor {FLOOR:.0f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
