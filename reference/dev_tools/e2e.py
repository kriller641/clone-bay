"""End-to-end tests for eanm_sync.py, run the way the user runs it."""
import hashlib, json, os, shutil, subprocess, sys

SCRIPT, REAL, TOJSON, WORK = sys.argv[1:5]
sys.path.insert(0, os.path.dirname(SCRIPT))
import eanm_sync as m  # noqa  (codec + helpers from the delivered script)

PROT = ("slotOrder", "linkedWeapons_")
results = []


def check(name, cond, detail=""):
    results.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name + (("  -- " + detail) if detail and not cond else ""))


def sha(p):
    return hashlib.sha256(open(p, "rb").read()).hexdigest()


def snapshot(folder):
    return {f: sha(os.path.join(folder, f)) for f in sorted(os.listdir(folder))
            if os.path.isfile(os.path.join(folder, f))}


def rust_json(path):
    return json.loads(subprocess.run([TOJSON, path], check=True, stdout=subprocess.PIPE).stdout)


def rust_ui_split(j):
    """Independent view via the Rust crate: (non-ui sections, ui-unprotected, ui-protected)."""
    ui = j.get("bytes:ui", {})
    strip = lambda k: k.split(":", 1)[1] if ":" in k else k
    prot = {k: v for k, v in ui.items() if strip(k).startswith(PROT)}
    rest = {k: v for k, v in ui.items() if not strip(k).startswith(PROT)}
    other = {k: v for k, v in j.items() if k != "bytes:ui"}
    return other, rest, prot


def ui_set(root, name, value):
    ui = [v for k, v in root if m.key_name(k) == "ui"][0]
    for i, (k, _) in enumerate(ui):
        if m.key_name(k) == name:
            if value is None:
                ui.pop(i)
            else:
                ui[i] = (k, value)
            return
    if value is not None:
        ui.append((name.encode(), value))


def layout(ships):
    return m.Tuple([m.Long(134360688266310454), m.Dict(
        (m.Long(s), list(range(27, 35)) + list(range(19, 27)) + list(range(11, 19))[::-1]) for s in ships)])


def run(folder, args, answers):
    p = subprocess.run([sys.executable, SCRIPT, folder] + args, input=answers, stdout=subprocess.PIPE,
                       stderr=subprocess.STDOUT, universal_newlines=True, timeout=300)
    return p.returncode, p.stdout


def build_folder():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(WORK)
    real = open(REAL, "rb").read()
    src = m.marshal_decode(real).value
    w = lambda name, data: open(os.path.join(WORK, name), "wb").write(data)
    w("core_user_111.dat", real)                                    # source (real file)
    a = m.marshal_decode(real).value                                # own layout + groups + toggle
    ui_set(a, "slotOrder", layout([999, 888]))
    ui_set(a, "linkedWeapons_groupsDict", m.Tuple([m.Long(2), m.Dict([(m.Long(999), m.Dict([(27, [28])]))])]))
    ui_set(a, "showEmptySlots", m.Tuple([m.Long(3), False]))
    ui_set(a, "someOtherSetting", m.Tuple([m.Long(4), b"target-only"]))
    w("core_user_222.dat", m.marshal_encode(a, False))
    b = m.marshal_decode(real).value                                # no layout at all
    ui_set(b, "slotOrder", None); ui_set(b, "linkedWeapons_groupsDict", None)
    w("core_user_333.dat", m.marshal_encode(b, False))
    w("core_user_444.dat", real[:len(real) // 2])                   # damaged
    w("core_user_555.dat", m.marshal_encode(a, False))              # will be made read-only
    c = m.marshal_decode(real).value                                # has a CRC
    ui_set(c, "slotOrder", layout([777]))
    w("core_user_666.dat", m.marshal_encode(c, True))
    for i, extra in enumerate([None, None, None, layout([555])], 1):  # character files
        root = m.Dict([(b"windows", m.Dict([(b"fitting", m.Tuple([m.Long(i), [i, i * 2, 300, 400]]))])),
                       (b"ui", m.Dict([(b"neocomButtonRawData", m.Tuple([m.Long(i), b"char%d" % i])),
                                       (b"chatchannels", m.Tuple([m.Long(i), [b"Local", b"Corp%d" % i]]))]))])
        if extra is not None:
            root[1][1].append((b"slotOrder", extra))
        w("core_char_%d.dat" % i, m.marshal_encode(root, False))
    open(os.path.join(WORK, "notes.txt"), "w").write("unrelated file")
    subprocess.run(["chattr", "+i", os.path.join(WORK, "core_user_555.dat")], check=True)
    # make the real file the newest so it is option [0]
    os.utime(os.path.join(WORK, "core_user_111.dat"), (2e9, 2e9))
    os.utime(os.path.join(WORK, "core_char_1.dat"), (2e9, 2e9))


def main():
    # ---- 1. dry run changes nothing
    build_folder()
    before = snapshot(WORK)
    rc, out = run(WORK, ["--users", "--dry-run"], "0\n")
    check("dry run: nothing changed", snapshot(WORK) == before)
    check("dry run: no backup folder", not any(f.startswith("eanm_sync_backup") for f in os.listdir(WORK)))
    check("dry run: plan lists kept layout", "module rack layout (2 ships)" in out and "weapon groups (1 ship)" in out, out)
    check("dry run: damaged file reported", "core_user_444.dat  SKIPPED: can't decode it" in out, out)
    check("dry run: read-only file reported", "core_user_555.dat  SKIPPED: file is read-only" in out, out)
    print(out)

    # ---- 3. real account sync
    orig = {f: open(os.path.join(WORK, f), "rb").read() for f in os.listdir(WORK) if f.endswith(".dat")}
    rc, out = run(WORK, ["--users"], "0\ny\n")
    print(out)
    check("sync: exit code 1 because two files were skipped", rc == 1, str(rc))
    backups = [f for f in os.listdir(WORK) if f.startswith("eanm_sync_backup")]
    check("sync: one backup folder", len(backups) == 1, str(backups))
    bdir = os.path.join(WORK, backups[0])
    check("sync: backups are exactly the written targets",
          sorted(os.listdir(bdir)) == ["core_user_222.dat", "core_user_333.dat", "core_user_666.dat"], str(os.listdir(bdir)))
    for f in os.listdir(bdir):
        check("sync: backup of %s is byte-identical to original" % f, open(os.path.join(bdir, f), "rb").read() == orig[f])
    check("sync: source untouched", open(os.path.join(WORK, "core_user_111.dat"), "rb").read() == orig["core_user_111.dat"])
    check("sync: damaged target untouched", open(os.path.join(WORK, "core_user_444.dat"), "rb").read() == orig["core_user_444.dat"])
    check("sync: read-only target untouched", open(os.path.join(WORK, "core_user_555.dat"), "rb").read() == orig["core_user_555.dat"])
    check("sync: character files untouched", all(open(os.path.join(WORK, f), "rb").read() == orig[f] for f in orig if f.startswith("core_char")))
    check("sync: no temp files left", not any(f.endswith(".tmp") for f in os.listdir(WORK)))

    # independent verification with the Rust crate
    s_other, s_rest, s_prot = rust_ui_split(rust_json(os.path.join(WORK, "core_user_111.dat")))
    for f in ["core_user_222.dat", "core_user_333.dat", "core_user_666.dat"]:
        old = os.path.join(bdir, f)
        new = os.path.join(WORK, f)
        o_other, o_rest, o_prot = rust_ui_split(rust_json(new))
        _, _, t_prot = rust_ui_split(rust_json(old))
        check("rust check %s: other sections == source" % f, o_other == s_other)
        check("rust check %s: ui settings == source (incl. showEmptySlots, target-only keys gone)" % f, o_rest == s_rest)
        check("rust check %s: rack layout/weapon groups == its own original" % f, o_prot == t_prot,
              "%s vs %s" % (list(o_prot), list(t_prot)))
    check("222 kept 2-ship layout", len(rust_ui_split(rust_json(os.path.join(WORK, "core_user_222.dat")))[2]["bytes:slotOrder"]["tuple"][1]) == 2)
    check("333 still has no layout", rust_ui_split(rust_json(os.path.join(WORK, "core_user_333.dat")))[2] == {})
    d666 = m.marshal_decode(open(os.path.join(WORK, "core_user_666.dat"), "rb").read())
    check("666 keeps its checksum", d666.had_crc)

    # ---- 4. character sync: exact copies, and a char file holding slotOrder keeps it
    before_chars = {f: open(os.path.join(WORK, f), "rb").read() for f in os.listdir(WORK) if f.startswith("core_char")}
    rc, out = run(WORK, ["--chars"], "0\ny\n")
    print(out)
    src_c = before_chars["core_char_1.dat"]
    check("chars: exit code 0", rc == 0, str(rc))
    check("chars: 2 and 3 are exact byte copies", all(open(os.path.join(WORK, f), "rb").read() == src_c for f in ["core_char_2.dat", "core_char_3.dat"]))
    c4 = rust_ui_split(rust_json(os.path.join(WORK, "core_char_4.dat")))
    check("chars: 4 kept its own slotOrder", c4[2] and len(c4[2]["bytes:slotOrder"]["tuple"][1]) == 1)
    check("chars: 4 got the source's other settings", c4[0] == rust_ui_split(rust_json(os.path.join(WORK, "core_char_1.dat")))[0])

    # ---- 5. refuses to run while EVE is running
    fake = os.path.join(os.path.dirname(WORK), "exefile.exe")
    shutil.copy(shutil.which("sleep"), fake)
    proc = subprocess.Popen([fake, "30"])
    try:
        before = snapshot(WORK)
        rc, out = run(WORK, ["--users"], "0\ny\n")
        check("eve running: refused", rc != 0 and "EVE is running" in out, out)
        check("eve running: nothing changed", snapshot(WORK) == before)
    finally:
        proc.kill(); proc.wait(); os.remove(fake)

    # ---- 6. picking a damaged file as the source
    before = snapshot(WORK)
    users = sorted(sorted(f for f in os.listdir(WORK) if f.startswith("core_user")), key=lambda f: -os.stat(os.path.join(WORK, f)).st_mtime)
    idx = users.index("core_user_444.dat")
    rc, out = run(WORK, ["--users"], "%d\ny\n" % idx)
    check("damaged source: refused, nothing changed", "Can't use core_user_444.dat as the source" in out and snapshot(WORK) == before, out)

    # ---- 7. write-time failures (unit level)
    data_a = open(os.path.join(WORK, "core_user_222.dat"), "rb").read()
    t = os.path.join(WORK, "core_user_333.dat")
    cur = open(t, "rb").read()
    p = m.Plan(m.Path(t), "keep", "", data_a, cur + b"x")      # file "changed" since it was checked
    err = m.write_target(p)
    check("changed-after-check: refused and untouched", err and "changed after it was checked" in err and open(t, "rb").read() == cur, str(err))
    p = m.Plan(m.Path(t), "keep", "", data_a, cur)
    subprocess.run(["chattr", "+i", t], check=True)                 # replace will fail
    err = m.write_target(p)
    subprocess.run(["chattr", "-i", t], check=True)
    check("replace failure: reported, untouched, temp removed",
          err and "couldn't write it" in err and open(t, "rb").read() == cur and not os.path.exists(t + ".eanm_sync.tmp"), str(err))
    err = m.write_target(m.Plan(m.Path(t), "keep", "", data_a, cur))
    check("normal write works", err is None and open(t, "rb").read() == data_a, str(err))

    subprocess.run(["chattr", "-i", os.path.join(WORK, "core_user_555.dat")])
    failed = [n for n, ok in results if not ok]
    print("\n%d checks, %d failed" % (len(results), len(failed)))
    for n in failed:
        print("  FAILED:", n)


main()
