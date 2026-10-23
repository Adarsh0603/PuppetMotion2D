"""Write the ready-made motions: one JSON file per motion in a folder per
category (core/motions/<category>/<view>_<name>.json). The app finds every
JSON file under core/motions/ when it's built.

    python core/motions/generate.py

Each category is a module in gen/ with a MOTIONS list (in menu order). To add
a motion, write a function there and put it in MOTIONS; to delete one, take
it out. Then run this. Files this script wrote before but no longer makes are
removed; hand-written JSON files (without "source") are left alone.
"""
import importlib, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

# Module name → category shown in the app, in menu order.
CATEGORIES = {
    "basics": "Basics",
    "move": "Move",
    "jump": "Jump",
    "combat": "Combat",
    "reactions": "Reactions",
    "emotes": "Emotes",
}
SOURCE = "generate.py"


def rounded(keys, places):
    return [[round(t, 4), round(v, places)] for t, v in keys]


def to_json(m):
    order = ["name", "category", "order", "template", "duration", "loop", "source"]
    lines = ["{"] + [f"  {json.dumps(k)}: {json.dumps(m[k])}," for k in order]
    lines.append(f'  "rest": {json.dumps(m["rest"])},')
    lines.append('  "tracks": {')
    items = list(m["tracks"].items())
    for i, (k, v) in enumerate(items):
        lines.append(f"    {json.dumps(k)}: {json.dumps(rounded(v, 1))}" + ("," if i + 1 < len(items) else ""))
    extra = [k for k in ("bob", "sway") if m[k]]
    lines.append("  }" + ("," if extra else ""))
    for i, k in enumerate(extra):
        lines.append(f"  {json.dumps(k)}: {json.dumps(rounded(m[k], 4))}" + ("," if i + 1 < len(extra) else ""))
    lines.append("}")
    return "\n".join(lines) + "\n"


def slug(text):
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_")


def main():
    written = set()
    for module, category in CATEGORIES.items():
        try:
            mod = importlib.import_module(f"gen.{module}")
        except ModuleNotFoundError:
            continue
        folder = os.path.join(HERE, module)
        os.makedirs(folder, exist_ok=True)
        for i, m in enumerate(mod.MOTIONS):
            m = {**m, "category": category, "order": i, "source": SOURCE}
            view = {"Person (side)": "side", "Person (front)": "front"}.get(m["template"]) or slug(m["template"])
            path = os.path.join(folder, f"{view}_{slug(m['name'])}.json")
            if path in written:
                sys.exit(f"two motions would both be written to {path}")
            with open(path, "w", newline="\n") as f:
                f.write(to_json(m))
            written.add(path)

    removed = 0
    for root, _, files in os.walk(HERE):
        for name in files:
            path = os.path.join(root, name)
            if not name.endswith(".json") or path in written:
                continue
            with open(path) as f:
                if json.load(f).get("source") == SOURCE:
                    os.remove(path)
                    removed += 1
    print(f"wrote {len(written)} motions, removed {removed} old ones")


main()
