"""Write the list of item groups a probe scan filter can hold.

The list comes from CCP's static data export (the "SDE"), the JSON Lines
edition:

    https://developers.eveonline.com/static-data/eve-online-static-data-latest-jsonl.zip

Unzip `groups.jsonl`, `categories.jsonl` and `_sde.jsonl` from it into a
folder, then run, from the repository root:

    python tools/make_group_list.py <that folder>

It rewrites crates/core/src/data/scan_groups.tsv: one
`id<TAB>category<TAB>published<TAB>name` line for every group in the
categories below, sorted by category and then name. `published` is 1 for a
group the game shows to players and 0 for one it doesn't; the app offers
only the first kind but can name the second when a filter already holds it.
Re-run it after a game update adds kinds of ship.
"""
import json
import os
import sys

# What combat probes can find. This is our reading of the game, not a list
# taken from it, so it may leave out something probes can find.
CATEGORIES = [6, 18, 87, 22, 65, 23, 40, 46]
OUT = os.path.join("crates", "core", "src", "data", "scan_groups.tsv")


def rows(path):
    with open(path, encoding="utf-8") as f:
        for line in f:
            yield json.loads(line)


def clean(name):
    name = " ".join(name.split())
    assert name and "\t" not in name, name
    return name


def main():
    folder = sys.argv[1]
    build = next(rows(os.path.join(folder, "_sde.jsonl")))
    categories = {c["_key"]: clean(c["name"]["en"])
                  for c in rows(os.path.join(folder, "categories.jsonl"))}
    found = []
    for group in rows(os.path.join(folder, "groups.jsonl")):
        category = group.get("categoryID")
        if category in CATEGORIES:
            name = clean(group["name"]["en"])
            found.append((CATEGORIES.index(category), name.casefold(), group["_key"],
                          categories[category], 1 if group.get("published") else 0, name))
    found.sort()
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("# Built by tools/make_group_list.py from CCP's static data, build %s (%s).\n"
                % (build["buildNumber"], build["releaseDate"][:10]))
        f.write("# Item groups a probe scan filter can hold:"
                " id<TAB>category<TAB>published<TAB>name.\n")
        for _, _, group_id, category, published, name in found:
            f.write("%d\t%s\t%d\t%s\n" % (group_id, category, published, name))
    shown = sum(1 for row in found if row[4])
    print("%s: %d groups, %d of them published" % (OUT, len(found), shown))


main()
