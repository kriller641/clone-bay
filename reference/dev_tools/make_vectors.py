"""Write test vectors for the Rust codec from the Python reference codec.

Reuses the stream generator in fuzz.py (random valid encodings, shared flags,
back-references, version 0 maps, checksums, plus corrupted variants) and
records what eanm_sync.py makes of each stream.

Usage:
    python make_vectors.py <output file> <N valid streams> <seed>

Each valid stream also gets two corrupted variants. One line per case:

    <V|C> <hex input> <hex of the Python re-encoding | !error message>

V = a valid stream, C = a corrupted one. The Rust test that reads this is
crates/marshal/tests/reference_vectors.rs.
"""
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT, N, SEED = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])

sys.path.insert(0, os.path.dirname(HERE))
import eanm_sync as codec  # noqa: E402

# fuzz.py is a script: it reads sys.argv and runs main() on import. Load its
# definitions without running it, and without editing the reference copy.
sys.modules["codec"] = codec
source = open(os.path.join(HERE, "fuzz.py"), encoding="utf-8").read()
head, sep, tail = source.rpartition("\nmain()")
assert sep and not tail.strip(), "fuzz.py no longer ends with main()"
sys.argv = ["fuzz.py", "", "", "0", "0"]
fuzz = {"__file__": os.path.join(HERE, "fuzz.py"), "__name__": "fuzz"}
exec(compile(head, "fuzz.py", "exec"), fuzz)


def python_result(data):
    try:
        d = codec.marshal_decode(data)
        return codec.marshal_encode(d.value, d.had_crc).hex()
    except codec.MarshalError as e:
        return "!" + str(e)
    except RecursionError:
        return "!recursion"


def main():
    rng = random.Random(SEED)
    lines = []
    for _ in range(N):
        tree = fuzz["rand_tree"](rng)
        version = rng.choice([0, 0, 1, 1, 2])
        crc = rng.random() < 0.4
        data, expected = fuzz["build_stream"](
            tree, rng.randrange(2**31), version, crc, permute=rng.random() < 0.7)
        result = python_result(data)
        # The generator knows what a correct decoder must produce.
        assert result == codec.marshal_encode(expected, crc).hex(), "reference codec disagrees with the generator"
        lines.append("V %s %s" % (data.hex(), result))
        for _ in range(2):
            bad = fuzz["corrupt"](rng, data)
            lines.append("C %s %s" % (bad.hex(), python_result(bad)))
    with open(OUT, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print("%d cases, %d bytes" % (len(lines), os.path.getsize(OUT)))


main()
