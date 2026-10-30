"""Write the list of ship and structure types that is built into the app.

The list comes from CCP's static data export (the "SDE"), the JSON Lines
edition:

    https://developers.eveonline.com/static-data/eve-online-static-data-latest-jsonl.zip

Unzip `types.jsonl`, `groups.jsonl` and `_sde.jsonl` from it into a folder,
then run, from the repository root:

    python tools/make_type_list.py <that folder>

It rewrites crates/core/src/data/bundled_types.tsv: one `id<TAB>name` line
for every published type in a published group of the Ship (6) or Structure
(65) category, sorted by name. Re-run it after a game update adds ships.
"""
import json
import os
import sys

CATEGORIES = {6: "ships", 65: "structures"}
OUT = os.path.join("crates", "core", "src", "data", "bundled_types.tsv")


def rows(path):
    with open(path, encoding="utf-8") as f:
        for line in f:
            yield json.loads(line)


def main():
    folder = sys.argv[1]
    build = next(rows(os.path.join(folder, "_sde.jsonl")))
    groups = {g["_key"]: g for g in rows(os.path.join(folder, "groups.jsonl"))}
    found = []
    counts = {name: 0 for name in CATEGORIES.values()}
    for t in rows(os.path.join(folder, "types.jsonl")):
        group = groups.get(t["groupID"], {})
        category = group.get("categoryID")
        if category in CATEGORIES and t.get("published") and group.get("published"):
            name = " ".join(t["name"]["en"].split())
            assert name and "\t" not in name, t["_key"]
            found.append((name.casefold(), name, t["_key"]))
            counts[CATEGORIES[category]] += 1
    found.sort()
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("# Built by tools/make_type_list.py from CCP's static data, build %s (%s).\n"
                % (build["buildNumber"], build["releaseDate"][:10]))
        f.write("# Published ship and structure types: id<TAB>name, sorted by name.\n")
        for _, name, type_id in found:
            f.write("%d\t%s\n" % (type_id, name))
    print("%s: %s" % (OUT, ", ".join("%d %s" % (n, what) for what, n in counts.items())))


main()
