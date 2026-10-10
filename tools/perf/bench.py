"""Compare spg binaries on tools/perf/bench/*.spg, alternating them in the
same window (this machine's speed drifts, so never compare separate runs).
    python3 tools/perf/bench.py base=./old/spg,new=target/release/spg 3 [fib,sieve]"""
import subprocess, sys, time, statistics, glob, os
HERE = os.path.dirname(os.path.abspath(__file__))
bins = sys.argv[1].split(",")          # name=path,name=path
rounds = int(sys.argv[2])
only = sys.argv[3].split(",") if len(sys.argv) > 3 else None
bins = [b.split("=") for b in bins]
files = sorted(glob.glob(os.path.join(HERE, "bench", "*.spg")))
if only: files = [f for f in files if os.path.basename(f)[:-4] in only]
print(f"{'bench':12}" + "".join(f"{n:>12}" for n,_ in bins) + ("   speedup(first/last)" if len(bins)>1 else ""))
tot = {n:0.0 for n,_ in bins}
for f in files:
    t = {n:[] for n,_ in bins}; out = {}
    for r in range(rounds):
        for n,p in bins:                # alternate binaries inside the same window
            s = time.perf_counter()
            res = subprocess.run([p, f], capture_output=True, text=True)
            t[n].append(time.perf_counter()-s)
            out[n] = (res.stdout, res.returncode)
    outs = set(out.values())
    flag = "" if len(outs)==1 else "  !!! OUTPUT MISMATCH"
    mins = {n:min(v) for n,v in t.items()}
    for n in mins: tot[n]+=mins[n]
    line = f"{os.path.basename(f)[:-4]:12}" + "".join(f"{mins[n]:11.3f}s" for n,_ in bins)
    if len(bins)>1: line += f"   {mins[bins[0][0]]/mins[bins[-1][0]]:5.2f}x"
    print(line+flag)
print(f"{'TOTAL':12}" + "".join(f"{tot[n]:11.3f}s" for n,_ in bins) + (f"   {tot[bins[0][0]]/tot[bins[-1][0]]:5.2f}x" if len(bins)>1 else ""))
