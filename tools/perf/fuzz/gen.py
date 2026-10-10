import random, sys

class Gen:
    def __init__(self, seed):
        self.r = random.Random(seed)
        self.n = 0
        self.fcount = 0
        self.globals = ["g1", "g2"]
    def uid(self, p):
        self.n += 1
        return f"{p}{self.n}"
    def pick(self, xs): return self.r.choice(xs)
    def chance(self, p): return self.r.random() < p

    # ---- expressions ----
    def iexpr(self, sc, d):
        r = self.r
        if d <= 0 or self.chance(0.25):
            c = r.random()
            if c < 0.35 and (sc["ints"] or sc["ro"]): return self.pick(sc["ints"] + sc["ro"])
            if c < 0.45 and self.globals: return self.pick(self.globals)
            k = r.randint(0, 20)
            return f"{k}" if self.chance(0.8) else f"(0 - {k})"
        c = r.random()
        if c < 0.40:
            return f"({self.iexpr(sc,d-1)} {self.pick(['+','-','*'])} {self.iexpr(sc,d-1)})"
        if c < 0.47: return f"({self.iexpr(sc,d-1)} / {r.randint(1,9)})"
        if c < 0.54: return f"({self.iexpr(sc,d-1)} % {r.randint(1,9)})"
        if c < 0.62: return f"(if ({self.cond(sc,d-1)}) {{ {self.iexpr(sc,d-1)} }} else {{ {self.iexpr(sc,d-1)} }})"
        if c < 0.70 and sc["funcs"]:
            f, ar = self.pick(sc["funcs"])
            return f"{f}({', '.join(self.iexpr(sc,d-1) for _ in range(ar))})"
        if c < 0.78 and sc["arrs"]: return f"{self.pick(sc['arrs'])}[{self.idx(sc,d-1)}]"
        if c < 0.84 and sc["structs"]: return f"{self.pick(sc['structs'])}.{self.pick(['a','b'])}"
        if c < 0.88: return f"(-{self.iexpr(sc,d-1)})"
        if c < 0.92 and sc["arrs"]: return f"len({self.pick(sc['arrs'])})"
        if c < 0.96 and sc["floats"]: return f"@into.int({self.fexpr(sc,d-1)})"
        return f"({self.iexpr(sc,d-1)} + {self.iexpr(sc,d-1)})"
    def idx(self, sc, d):
        if self.chance(0.4): return str(self.r.randint(0, 3))
        if self.chance(0.4) and sc["idxvars"]: return self.pick(sc["idxvars"])
        return f"((({self.iexpr(sc,d)} % 4) + 4) % 4)"
    def fexpr(self, sc, d):
        r = self.r
        if d <= 0 or self.chance(0.3):
            if sc["floats"] and self.chance(0.6): return self.pick(sc["floats"])
            return f"{r.randint(0,9)}.{r.randint(0,9)}"
        c = r.random()
        if c < 0.6: return f"({self.fexpr(sc,d-1)} {self.pick(['+','-','*'])} {self.fexpr(sc,d-1)})"
        if c < 0.75: return f"({self.fexpr(sc,d-1)} / {r.randint(1,5)}.0)"
        if c < 0.85: return f"@into.float({self.iexpr(sc,d-1)})"
        return f"(if ({self.cond(sc,d-1)}) {{ {self.fexpr(sc,d-1)} }} else {{ {self.fexpr(sc,d-1)} }})"
    def cond(self, sc, d):
        r = self.r
        c = r.random()
        if d <= 0 or c < 0.5:
            if sc["floats"] and self.chance(0.2):
                return f"{self.fexpr(sc,d-1)} {self.pick(['<','>','<=','>=','==','!='])} {self.fexpr(sc,d-1)}"
            return f"{self.iexpr(sc,d-1)} {self.pick(['<','>','<=','>=','==','!='])} {self.iexpr(sc,d-1)}"
        if c < 0.68: return f"({self.cond(sc,d-1)}) && ({self.cond(sc,d-1)})"
        if c < 0.84: return f"({self.cond(sc,d-1)}) || ({self.cond(sc,d-1)})"
        if c < 0.92: return f"!({self.cond(sc,d-1)})"
        if sc["strs"] and self.chance(0.5):
            return f"{self.pick(sc['strs'])} {self.pick(['==','!='])} {self.pick(sc['strs'])}"
        return self.pick(["true", "false"])


    def operand(self, sc, kind):
        """a literal, a plain local, or a small expression: the fused index ops
        care about exactly these shapes"""
        r = self.r
        if kind == "i":
            c = r.random()
            if c < 0.35: return str(r.randint(0, 9))
            if c < 0.70 and sc["ints"]: return self.pick(sc["ints"])
            return self.iexpr(sc, 1)
        c = r.random()
        if c < 0.35: return f"{r.randint(0,9)}.{r.randint(0,9)}"
        if c < 0.70 and sc["floats"]: return self.pick(sc["floats"])
        return self.fexpr(sc, 1)
    def ixv(self, sc):
        """an index: mostly in range (a local or a literal), rarely out of range"""
        r = self.r
        if r.random() < 0.03: return self.pick(["4", "9", "-1"])
        return self.idx(sc, 1)
    def index_stmt(self, sc, p):
        r = self.r
        kinds = []
        if sc["arrs"]: kinds += ["ia"] * 4
        if sc["farrs"]: kinds += ["fa"] * 4
        if sc["barrs"]: kinds += ["ba"]
        if sc["arrs2"]: kinds += ["a2"] * 3
        if sc["idxvars"]: kinds += ["setidx"] * 2
        if sc["structs"]: kinds += ["fld"] * 4 + ["fldf"] * 2
        if not kinds: return None
        k = self.pick(kinds)
        if k == "ia":
            return [f"{p}{self.pick(sc['arrs'])}[{self.ixv(sc)}] {self.pick(['=','=','+=','-=','*='])} {self.operand(sc,'i')}"]
        if k == "fa":
            return [f"{p}{self.pick(sc['farrs'])}[{self.ixv(sc)}] {self.pick(['=','+=','-=','*='])} {self.operand(sc,'f')}"]
        if k == "ba":
            return [f"{p}{self.pick(sc['barrs'])}[{self.ixv(sc)}] = {self.pick(['true','false'])}"]
        if k == "a2":
            row = self.pick(sc["rowvars"]) if sc["rowvars"] and r.random() < 0.7 else str(r.randint(0, 1))
            return [f"{p}{self.pick(sc['arrs2'])}[{row}][{self.ixv(sc)}] {self.pick(['=','=','+=','-=','*='])} {self.operand(sc,'i')}"]
        if k == "fld":
            return [f"{p}{self.pick(sc['structs'])}.{self.pick(['a','b'])} {self.pick(['=','=','+=','-=','*='])} {self.operand(sc,'i')}"]
        if k == "fldf":
            return [f"{p}{self.pick(sc['structs'])}.f {self.pick(['=','+=','-=','*='])} {self.operand(sc,'f')}"]
        v = self.pick(sc["idxvars"])
        if sc["rowvars"] and r.random() < 0.25:
            return [f"{p}{self.pick(sc['rowvars'])} = {r.randint(0, 1)}"]
        return [f"{p}{v} = {self.pick([str(r.randint(0,3)), str(r.randint(0,3)), '((' + self.iexpr(sc,1) + ' % 4) + 4) % 4'])}"]

    # ---- statements ----
    def block(self, sc, d, ind, inloop, infunc):
        sc = {k: list(v) if isinstance(v, list) else v for k, v in sc.items()}
        out = []
        for _ in range(self.r.randint(1, 4)):
            out += self.stmt(sc, d, ind, inloop, infunc)
        return out
    def stmt(self, sc, d, ind, inloop, infunc):
        r = self.r; p = "    " * ind; c = r.random()
        if r.random() < 0.30:
            st = self.index_stmt(sc, p)
            if st: return st
        if c < 0.12:
            v = self.uid("v"); s = [f"{p}let {v} = {self.iexpr(sc,2)}"]; sc["ints"].append(v); return s
        if c < 0.16:
            v = self.uid("f"); s = [f"{p}let {v} = {self.fexpr(sc,2)}"]; sc["floats"].append(v); return s
        if c < 0.18:
            v = self.uid("t"); s = [f'{p}let {v} = "{self.pick(["a","b","ab"])}"']; sc["strs"].append(v); return s
        if c < 0.21 and sc["strs"]:
            v = self.pick(sc["strs"]); return [f'{p}{v} = {v} + "{self.pick(["x","y"])}"']
        if c < 0.36 and (sc["ints"] or self.globals):
            v = self.pick(sc["ints"] + self.globals)
            op = self.pick(["=", "=", "+=", "-=", "*="])
            return [f"{p}{v} {op} {self.iexpr(sc,2)}"]
        if c < 0.40 and sc["floats"]:
            v = self.pick(sc["floats"]); return [f"{p}{v} {self.pick(['=','+=','-=','*='])} {self.fexpr(sc,2)}"]
        if c < 0.47 and sc["arrs"]:
            a = self.pick(sc["arrs"]); return [f"{p}{a}[{self.idx(sc,1)}] {self.pick(['=','+=','-=','*='])} {self.iexpr(sc,2)}"]
        if c < 0.53 and sc["structs"]:
            s_ = self.pick(sc["structs"]); return [f"{p}{s_}.{self.pick(['a','b'])} {self.pick(['=','+=','-=','*='])} {self.iexpr(sc,2)}"]
        if c < 0.64 and d > 0:
            s = [f"{p}if ({self.cond(sc,2)}) {{"] + self.block(sc, d-1, ind+1, inloop, infunc)
            if self.chance(0.4):
                s += [f"{p}}} when ({self.cond(sc,2)}) {{"] + self.block(sc, d-1, ind+1, inloop, infunc)
            if self.chance(0.6):
                s += [f"{p}}} else {{"] + self.block(sc, d-1, ind+1, inloop, infunc)
            return s + [f"{p}}}"]
        if c < 0.73 and d > 0:
            cn = self.uid("c"); k = r.randint(0, 6)
            sc2 = {kk: list(v) if isinstance(v, list) else v for kk, v in sc.items()}
            sc2["ro"].append(cn)
            pre = []
            bound = str(k)
            if self.chance(0.5):
                bn = self.uid("b"); pre = [f"{p}let {bn} = {k}"]; bound = bn
                sc2["ro"].append(bn)
            rel = self.pick(["<", "<=", "!="])
            s = pre + [f"{p}let {cn} = 0", f"{p}while ({cn} {rel} {bound}) {{", f"{p}    {cn} += 1"]
            return s + self.block(sc2, d-1, ind+1, True, infunc) + [f"{p}}}"]
        if c < 0.80 and d > 0 and sc["arrs"]:
            v = self.uid("e"); a = self.pick(sc["arrs"])
            sc2 = {kk: list(v_) if isinstance(v_, list) else v_ for kk, v_ in sc.items()}
            sc2["ints"].append(v)
            return [f"{p}for {v} in {a} {{"] + self.block(sc2, d-1, ind+1, True, infunc) + [f"{p}}}"]
        if c < 0.85 and inloop:
            return [f"{p}if ({self.cond(sc,1)}) {{", f"{p}    {self.pick(['break','continue'])}", f"{p}}}"]
        if c < 0.88 and infunc:
            return [f"{p}if ({self.cond(sc,1)}) {{", f"{p}    return {self.iexpr(sc,2)}", f"{p}}}"]
        if c < 0.92:
            return [f"{p}print({self.iexpr(sc,2)})"]
        v = self.pick(sc["ints"]) if sc["ints"] else "g1"
        return [f"{p}{v} = {v} + 1"]

    def function(self, funcs):
        name = f"fn{self.fcount}"; self.fcount += 1
        ar = self.r.randint(1, 3)
        params = [f"p{i}" for i in range(ar)]
        sc = {"ints": list(params), "floats": ["lfv"], "strs": [], "arrs": [], "structs": [], "funcs": list(funcs), "idxvars": ["ia", "ib"], "rowvars": ["rr"], "farrs": ["lf"], "barrs": ["lb"], "arrs2": ["lm"], "ro": []}
        body = []
        # every function gets a local array and a struct so those ops show up in calls too
        sc["arrs"].append("la"); body.append("    let la = [3, 1, 4, 1]")
        sc["structs"].append("ls"); body.append("    let ls = S { a: 2, b: 5, f: 1.5 }")
        body += ["    let lfv = 0.75", "    let ia = 1", "    let ib = 2", "    let rr = 1", "    let lf = [1.5, 2.5, 3.5, 4.5]", "    let lb = [true, false, true, true]", "    let lm = [[1, 2, 3, 4], [5, 6, 7, 8]]"]
        for _ in range(self.r.randint(2, 6)):
            body += self.stmt(sc, 2, 1, False, True)
        body.append(f"    return {self.iexpr(sc,3)}")
        return name, ar, [f"function {name}({', '.join(p + ': int' for p in params)}) -> int {{"] + body + ["}"]

    def program(self):
        lines = ["struct S { a: int, b: int, f: float }", "let g1 = 3", "let g2 = 7", ""]
        funcs = []
        for _ in range(self.r.randint(1, 4)):
            name, ar, code = self.function(funcs)
            lines += code + [""]
            funcs.append((name, ar))
        sc = {"ints": [], "floats": [], "strs": [], "arrs": ["ma"], "structs": ["ms"], "funcs": funcs, "idxvars": ["ia", "ib"], "rowvars": ["rr"], "farrs": ["mfa"], "barrs": ["mba"], "arrs2": ["mm"], "ro": []}
        body = ["    let ma = [5, 6, 7, 8]", "    let ms = S { a: 1, b: 2, f: 2.5 }", "    let ia = 0", "    let ib = 3", "    let rr = 0", "    let mfa = [1.5, 2.5, 3.5, 4.5]", "    let mba = [true, false, true, true]", "    let mm = [[1, 2, 3, 4], [5, 6, 7, 8]]"]
        for k in range(3):
            v = f"m{k}"; body.append(f"    let {v} = {self.iexpr(sc,2)}"); sc["ints"].append(v)
        body.append("    let mf = 1.5"); sc["floats"].append("mf")
        body.append('    let mt = "s"'); sc["strs"].append("mt")
        for _ in range(self.r.randint(4, 10)):
            body += self.stmt(sc, 3, 1, False, True)
        body.append("    print(ma)"); body += ["    print(mfa)", "    print(mba)", "    print(mm)", "    print(ia)", "    print(ib)", "    print(rr)"]; body.append("    print(ms.a)"); body.append("    print(ms.b)"); body.append("    print(ms.f)")
        for v in sc["ints"][:12]: body.append(f"    print({v})")
        body.append("    print(mf)"); body.append("    print(mt)")
        body.append("    return m0 + m1")
        lines += ["function run() -> int {"] + body + ["}", "print(run())", "print(g1)", "print(g2)"]
        # top-level (global) statements too
        gsc = {"ints": [], "floats": [], "strs": [], "arrs": [], "structs": [], "funcs": funcs, "idxvars": [], "rowvars": [], "farrs": [], "barrs": [], "arrs2": [], "ro": []}
        for _ in range(self.r.randint(1, 3)):
            lines += self.stmt(gsc, 1, 0, False, False)
        lines += ["print(g1 + g2)"]
        return "\n".join(lines) + "\n"

if __name__ == "__main__":
    lo, hi = int(sys.argv[1]), int(sys.argv[2])
    import os
    OUT = sys.argv[3] if len(sys.argv) > 3 else "fuzz_progs"
    os.makedirs(OUT, exist_ok=True)
    for s in range(lo, hi):
        open(f"{OUT}/{s}.spg", "w").write(Gen(s).program())
