"""Differential fuzz test: Python codec vs the blue-marshal Rust crate.

Generates random value trees, writes each one with randomly chosen (but valid)
wire encodings, shared flags, back-references, version 0/1 headers and CRCs,
then checks that:
  * Python decodes it to exactly the tree that was written
  * Python and Rust re-encode it to identical bytes
  * corrupted variants never make Python succeed with a different result
"""
import os, random, struct, subprocess, sys, zlib, shutil
sys.path.insert(0, os.path.dirname(__file__))
from codec import *  # noqa  (copy eanm_sync.py to codec.py)

RT = sys.argv[1]
WORK = sys.argv[2]
N = int(sys.argv[3]) if len(sys.argv) > 3 else 3000
SEED = int(sys.argv[4]) if len(sys.argv) > 4 else 1


def rand_tree(rng, depth=0):
    kinds = ["none", "bool", "int", "long", "float", "bytes", "str", "global"]
    if depth < 5:
        kinds += ["tuple", "list", "dict", "dict", "instance", "reduce", "callback"]
    k = rng.choice(kinds)
    if k == "none": return None
    if k == "bool": return rng.random() < 0.5
    if k == "int":
        return rng.choice([-1, 0, 1, rng.randint(-128, 127), rng.randint(-32768, 32767),
                           rng.randint(-2**31, 2**31 - 1), rng.randint(-2**63, 2**63 - 1)])
    if k == "long":
        return Long(rng.choice([0, 1, -1, 127, 128, -128, -129, 255, 256, 2**63, -2**63,
                                rng.randint(-2**200, 2**200), 134360692511673899]))
    if k == "float":
        return rng.choice([0.0, -0.0, 1.5, -2.25, 1e300, float("inf"), float("-inf"), rng.uniform(-1e9, 1e9)])
    if k == "bytes":
        n = rng.choice([0, 1, 2, 30, 31, 254, 255, 256, 300])
        if rng.random() < 0.15:
            return rng.choice(STR_TABLE).encode()
        return bytes(rng.randrange(256) for _ in range(n))
    if k == "str":
        alphabet = "abcXYZ_ 09é中€\U0001F680﻿"
        n = rng.choice([0, 1, 2, 5, 40, 300])
        return "".join(rng.choice(alphabet) for _ in range(n))
    if k == "global": return Global(rng.choice(["a.b", "util.KeyVal", "x"]))
    if k == "tuple": return Tuple(rand_tree(rng, depth + 1) for _ in range(rng.choice([0, 1, 2, 3, 6])))
    if k == "list": return [rand_tree(rng, depth + 1) for _ in range(rng.choice([0, 1, 2, 4, 260 if depth == 0 else 3]))]
    if k == "dict":
        return Dict((rng.choice([rand_tree(rng, 9), b"key%d" % i, i]), rand_tree(rng, depth + 1))
                    for i in range(rng.choice([0, 1, 3, 8])))
    if k == "instance": return Instance(rng.choice(["mod.Cls", "util.Row"]), rand_tree(rng, depth + 1))
    if k == "reduce":
        newobj = rng.random() < 0.5
        args = Tuple(rand_tree(rng, depth + 1) for _ in range(rng.randint(0, 2)))
        state = rand_tree(rng, depth + 1) if rng.random() < 0.6 else MISSING
        return Reduce(newobj, None if newobj else Global("mod.f"), args, state,
                      [rand_tree(rng, depth + 1) for _ in range(rng.randint(0, 2))],
                      [(rand_tree(rng, 9), rand_tree(rng, depth + 1)) for _ in range(rng.randint(0, 2))])
    return Callback(rand_tree(rng, depth + 1))


MISSING = sys.modules["codec"]._MISSING


class VariantWriter:
    """Writes a tree with random valid encodings; returns what a correct
    decoder must produce (references substitute earlier shared objects)."""

    def __init__(self, rng, version, mapping=None):
        self.rng, self.version, self.mapping = rng, version, mapping
        self.out = bytearray()
        self.allocs = 0
        self.completed = []

    def integer(self, n):
        # draw first so the random sequence never depends on n (ref ids)
        r = self.rng.random()
        if 0 <= n < 255 and r < 0.9:
            self.out.append(n)
        else:
            self.out.append(0xFF); self.out += struct.pack("<i", n)

    def tag(self, t, shared=False):
        self.out.append(t | (0x40 if shared else 0))

    def alloc(self):
        k = self.allocs
        self.allocs += 1
        if self.version == 0:
            return self.mapping[k] if self.mapping else k + 1
        return k

    def maybe_share(self):
        return self.rng.random() < 0.3

    def done(self, ref_id, value):
        if ref_id is not None:
            self.completed.append((ref_id, value))
        return value

    def emit(self, v, allow_ref=True):
        rng = self.rng
        if allow_ref and self.completed and rng.random() < 0.1:
            ref_id, val = rng.choice(self.completed)
            self.tag(27); self.integer(ref_id)
            return val
        junk_flag = rng.random() < 0.2      # shared flag on a type that ignores it
        if v is None: self.tag(1, junk_flag); return None
        if v is True: self.tag(31, junk_flag); return True
        if v is False: self.tag(32, junk_flag); return False
        if isinstance(v, Long):
            sh = self.maybe_share()
            n = int(v)
            if n == 0 and rng.random() < 0.5:
                b = b""
            else:
                b = n.to_bytes((n.bit_length() + 8) // 8 + rng.choice([0, 0, 1]), "little", signed=True)
            self.tag(47, sh); self.integer(len(b)); self.out += b
            return self.done(self.alloc() if sh else None, v)
        if isinstance(v, int):
            opts = []
            if v in (-1, 0, 1): opts.append({-1: 7, 0: 8, 1: 9}[v])
            if -128 <= v <= 127: opts.append(6)
            if -32768 <= v <= 32767: opts.append(5)
            if -2**31 <= v < 2**31: opts.append(4)
            opts.append(3)
            t = rng.choice(opts)
            self.tag(t, junk_flag)
            self.out += {6: lambda: struct.pack("<b", v), 5: lambda: struct.pack("<h", v),
                         4: lambda: struct.pack("<i", v), 3: lambda: struct.pack("<q", v)}.get(t, lambda: b"")()
            return v
        if isinstance(v, float):
            if v == 0.0 and rng.random() < 0.5:
                self.tag(11, junk_flag); return 0.0
            self.tag(10, junk_flag); self.out += struct.pack("<d", v); return v
        if isinstance(v, bytes):
            opts = ["str", "buffer"]
            if not v: opts.append("empty")
            if len(v) == 1: opts.append("char")
            if len(v) < 256: opts.append("short")
            try:
                idx = STR_TABLE.index(v.decode()) + 1
                opts += ["table", "table"]
            except (ValueError, UnicodeDecodeError):
                idx = None
            o = rng.choice(opts)
            if o in ("str", "buffer"):
                sh = self.maybe_share()
                self.tag(13 if o == "str" else 19, sh); self.integer(len(v)); self.out += v
                return self.done(self.alloc() if sh else None, v)
            if o == "empty": self.tag(14, junk_flag)
            elif o == "char": self.tag(15, junk_flag); self.out += v
            elif o == "short": self.tag(16, junk_flag); self.out.append(len(v)); self.out += v
            else: self.tag(17, junk_flag); self.out.append(idx)
            return v
        if isinstance(v, Global):
            sh = self.maybe_share()
            b = v.encode()
            self.tag(2, sh); self.integer(len(b)); self.out += b
            return self.done(self.alloc() if sh else None, v)
        if isinstance(v, str):
            u16 = v.encode("utf-16-le")
            opts = ["utf8", "utf16"]
            if not v: opts.append("u0")
            if len(u16) == 2: opts.append("u1")
            o = rng.choice(opts)
            if o == "u0": self.tag(40, junk_flag)
            elif o == "u1": self.tag(41, junk_flag); self.out += u16
            elif o == "utf16": self.tag(18, junk_flag); self.integer(len(u16) // 2); self.out += u16
            else:
                b = v.encode(); self.tag(46, junk_flag); self.integer(len(b)); self.out += b
            return v
        if isinstance(v, (Tuple, list)) and not isinstance(v, Dict):
            is_t = isinstance(v, Tuple)
            n = len(v)
            special = {0: 36 if is_t else 38, 1: 37 if is_t else 39}
            if is_t: special[2] = 44
            if n in special and rng.random() < 0.6:
                t = special[n]
                if n == 0:
                    self.tag(t, junk_flag)
                    return Tuple() if is_t else []
                sh = self.maybe_share()
                self.tag(t, sh)
            else:
                sh = self.maybe_share()
                self.tag(20 if is_t else 21, sh); self.integer(n)
            ref = self.alloc() if sh else None
            items = [self.emit(x) for x in v]
            return self.done(ref, Tuple(items) if is_t else items)
        if isinstance(v, Dict):
            sh = self.maybe_share()
            self.tag(22, sh); self.integer(len(v))
            ref = self.alloc() if sh else None
            pairs = Dict()
            for k, val in v:
                ev = self.emit(val)
                ek = self.emit(k)
                pairs.append((ek, ev))
            return self.done(ref, pairs)
        if isinstance(v, Instance):
            sh = self.maybe_share()
            self.tag(23, sh)
            ref = self.alloc() if sh else None
            self.emit(v.cls.encode(), allow_ref=False)
            state = self.emit(v.state)
            return self.done(ref, Instance(v.cls, state))
        if isinstance(v, Reduce):
            sh = self.maybe_share()
            self.tag(35 if v.newobj else 34, sh)
            ref = self.alloc() if sh else None
            head = [v.args] if v.newobj else [v.callable, v.args]
            if v.state is not MISSING:
                head.append(v.state)
            eh = self.emit(Tuple(head), allow_ref=False)
            items = [self.emit(x) for x in v.list_items]
            self.out.append(45)
            ditems = []
            for k, val in v.dict_items:
                ek = self.emit(k); ev = self.emit(val)
                ditems.append((ek, ev))
            self.out.append(45)
            if v.newobj:
                r = Reduce(True, None, eh[0], eh[1] if len(eh) > 1 else MISSING, items, ditems)
            else:
                r = Reduce(False, eh[0], eh[1], eh[2] if len(eh) > 2 else MISSING, items, ditems)
            return self.done(ref, r)
        if isinstance(v, Callback):
            self.tag(25, junk_flag)
            return Callback(self.emit(v.inner))
        raise TypeError(type(v))


def build_stream(tree, seed, version, crc, permute):
    def run(mapping):
        w = VariantWriter(random.Random(seed), version, mapping)
        expected = w.emit(tree, allow_ref=False)
        return w, expected
    w, expected = run(None)
    mapping = None
    if version == 0 and permute and w.allocs:
        mapping = list(range(1, w.allocs + 1))
        random.Random(seed ^ 0x5EED).shuffle(mapping)
        w, expected = run(mapping)
    body = bytes(w.out)
    if version == 0:
        head = b"\x7e" + struct.pack("<i", w.allocs)
        tail = struct.pack("<%di" % w.allocs, *(mapping or range(1, w.allocs + 1)))
        if crc:
            payload = body + tail
            return head + b"\x1c" + struct.pack("<I", zlib.adler32(payload, 1)) + payload, expected
        return head + body + tail, expected
    head = b"\x7d" + bytes([version])
    if crc:
        return head + b"\x1c" + struct.pack("<I", zlib.adler32(body, 1)) + body, expected
    return head + body, expected


def corrupt(rng, data):
    b = bytearray(data)
    m = rng.choice(["flip", "trunc", "byte", "insert"])
    if m == "trunc":
        return bytes(b[:rng.randrange(1, len(b))])
    i = rng.randrange(len(b))
    if m == "flip": b[i] ^= 1 << rng.randrange(8)
    elif m == "byte": b[i] = rng.randrange(256)
    else: b.insert(i, rng.randrange(256))
    return bytes(b)


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(WORK)
    rng = random.Random(SEED)
    cases = {}
    for i in range(N):
        tree = rand_tree(rng)
        version = rng.choice([0, 0, 1, 1, 2])
        crc = rng.random() < 0.4
        data, expected = build_stream(tree, rng.randrange(2**31), version, crc, permute=rng.random() < 0.7)
        cases["v%05d" % i] = (data, expected, crc, True)
        for j in range(2):
            cases["c%05d_%d" % (i, j)] = (corrupt(rng, data), None, None, False)
    for name, (data, _, _, _) in cases.items():
        open(os.path.join(WORK, name + ".bin"), "wb").write(data)
    subprocess.run([RT, WORK], check=True)

    stats = dict(valid=0, corrupt=0, both_ok=0, both_err=0, py_stricter=0, rust_err_py_ok=0)
    failures = []
    stricter = {}
    for name, (data, expected, crc, valid) in cases.items():
        rs_path = os.path.join(WORK, name + ".rs")
        rust = open(rs_path, "rb").read() if os.path.exists(rs_path) else None
        try:
            d = marshal_decode(data)
            py = marshal_encode(d.value, d.had_crc)
            pyerr = None
        except MarshalError as e:
            py, pyerr = None, str(e)
        except Exception as e:  # any other exception type is a bug
            failures.append((name, "python raised %s: %s" % (type(e).__name__, e)))
            continue
        if valid:
            stats["valid"] += 1
            want = marshal_encode(expected, crc)
            if py != want:
                failures.append((name, "python decode != written tree (%s)" % pyerr))
            if rust != want:
                failures.append((name, "rust output != written tree (%s)" % (
                    open(os.path.join(WORK, name + ".rserr")).read() if rust is None else "bytes differ")))
            if py is not None and d.trailing != 0:
                failures.append((name, "valid stream reported trailing bytes"))
        else:
            stats["corrupt"] += 1
            if py is not None and rust is not None:
                stats["both_ok"] += 1
                if py != rust:
                    failures.append((name, "corrupt input: both decoded but outputs differ"))
            elif py is None and rust is None:
                stats["both_err"] += 1
            elif py is None:
                stats["py_stricter"] += 1
                stricter[pyerr] = stricter.get(pyerr, 0) + 1
            else:
                stats["rust_err_py_ok"] += 1
                failures.append((name, "python accepted input rust rejects: " +
                                 open(os.path.join(WORK, name + ".rserr")).read()))
    print(stats)
    if stricter:
        print("python stricter than rust:", stricter)
    print("FAILURES:", len(failures))
    for f in failures[:15]:
        print("  ", f)


main()
