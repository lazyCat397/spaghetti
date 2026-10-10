"""Differential fuzzer for the optimizer: random typed programs must print
the same with and without the superinstruction pass (`--no-opt`).
    python3 tools/perf/fuzz/run.py target/release/spg 0 500
Any "fused!=unfused" above 0 is an optimizer bug; the failing seed's program
is kept in fuzz_progs/<seed>.spg."""
import subprocess, sys, os
HERE = os.path.dirname(os.path.abspath(__file__))
binp, lo, hi = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
OUT = "fuzz_progs"
subprocess.run([sys.executable, os.path.join(HERE, "gen.py"), str(lo), str(hi), OUT], check=True)

def run(extra, f):
    try:
        r = subprocess.run([binp, *extra, f], capture_output=True, text=True, timeout=10)
        return (r.stdout, r.returncode, r.stderr if r.returncode else "")
    except subprocess.TimeoutExpired:
        return ("TIMEOUT", -9, "")

bad = 0
for s in range(lo, hi):
    f = f"{OUT}/{s}.spg"
    if run([], f) != run(["--no-opt"], f):
        bad += 1
        print(f"MISMATCH seed {s}: {f}")
print(f"programs={hi - lo}  fused!=unfused: {bad}")
sys.exit(1 if bad else 0)
