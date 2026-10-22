"""Generate the ready-made motions (core/motions/*.json).

Run from anywhere: python core/motions/generate.py

Angle convention (as in the app): degrees, positive turns clockwise on screen.
Facing right, that swings a hanging limb backward, a forward-pointing foot
toe-down, and an upright torso or head forward. Limbs hanging from the torso
are written here relative to the *world* and converted to local by
subtracting the torso angle, so a lean doesn't tilt the legs.
"""
import json, os, sys

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))

SIDE_REST = {
    "torso": -90, "head": -90,
    **{f"{s} {b}": 90 for s in ("near", "far") for b in ("thigh", "shin", "upper arm", "forearm")},
    "near foot": 0, "far foot": 0,
}
FRONT_REST = {
    "torso": -90, "head": -90,
    **{f"{s} {b}": 90 for s in ("left", "right") for b in ("thigh", "shin", "upper arm", "forearm")},
}


def interp(keys, t):
    keys = sorted(keys)
    if t <= keys[0][0]:
        return keys[0][1]
    for (a, va), (b, vb) in zip(keys, keys[1:]):
        if a <= t <= b:
            return va + (vb - va) * (t - a) / (b - a)
    return keys[-1][1]


def shift(keys, by, duration):
    """The same loop, started `by` seconds later (for the other leg/arm)."""
    loop = [(round((t + by) % duration, 4), v) for t, v in keys if t < duration]
    loop.sort()
    start = interp([(t, v) for t, v in keys], (duration - by) % duration)
    out = [(0.0, start)] + [k for k in loop if k[0] > 0] + [(duration, start)]
    dedup = []
    for k in out:
        if not dedup or abs(dedup[-1][0] - k[0]) > 1e-3:
            dedup.append(k)
    return dedup


def to_local(world, torso):
    return [(t, round(v - interp(torso, t), 1)) for t, v in world]


def cycle(name, template, duration, rest, tracks, bob=None, sway=None):
    data = {"name": name, "template": template, "duration": duration, "rest": rest,
            "tracks": {k: [[round(t, 4), round(v, 1)] for t, v in keys] for k, keys in tracks.items()}}
    if bob:
        data["bob"] = [[round(t, 4), round(v, 4)] for t, v in bob]
    if sway:
        data["sway"] = [[round(t, 4), round(v, 4)] for t, v in sway]
    return data


def side_legs_arms(duration, torso, thigh_w, shin, foot, arm_w, forearm):
    """Near limbs as given, far limbs half a cycle later; arms swing opposite to legs."""
    half = duration / 2
    t = {"torso": torso}
    t["near thigh"] = to_local(thigh_w, torso)
    t["far thigh"] = to_local(shift(thigh_w, half, duration), torso)
    t["near shin"], t["far shin"] = shin, shift(shin, half, duration)
    t["near foot"], t["far foot"] = foot, shift(foot, half, duration)
    t["near upper arm"] = to_local(arm_w, torso)
    t["far upper arm"] = to_local(shift(arm_w, half, duration), torso)
    t["near forearm"], t["far forearm"] = forearm, shift(forearm, half, duration)
    return t


def walk():
    d = 1.0
    torso = [(0, 5), (d, 5)]
    thigh = [(0, -25), (0.25, -2), (0.5, 20), (0.75, -20), (d, -25)]
    shin = [(0, 2), (0.12, 15), (0.25, 5), (0.5, 15), (0.62, 55), (0.75, 60), (0.88, 20), (d, 2)]
    foot = [(0, -15), (0.12, 0), (0.25, 0), (0.5, 20), (0.62, 25), (0.75, 0), (0.88, -10), (d, -15)]
    arm = [(0, 20), (0.5, -20), (d, 20)]
    forearm = [(0, -10), (0.5, -30), (d, -10)]
    tr = side_legs_arms(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -3), (d, -3)]
    bob = [(0, 0), (0.1, 0.012), (0.25, -0.012), (0.5, 0), (0.6, 0.012), (0.75, -0.012), (d, 0)]
    return cycle("Walk", "Person (side)", d, SIDE_REST, tr, bob)


def run():
    d = 0.6
    torso = [(0, 12), (d, 12)]
    k = lambda f: f * d
    thigh = [(0, -30), (k(0.15), 0), (k(0.35), 35), (k(0.5), 10), (k(0.75), -55), (d, -30)]
    shin = [(0, 10), (k(0.15), 35), (k(0.35), 15), (k(0.5), 90), (k(0.65), 110), (k(0.8), 50), (d, 10)]
    foot = [(0, -10), (k(0.15), 5), (k(0.35), 30), (k(0.5), 20), (k(0.75), -10), (d, -10)]
    arm = [(0, 40), (k(0.5), -45), (d, 40)]
    forearm = [(0, -70), (k(0.5), -100), (d, -70)]
    tr = side_legs_arms(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -8), (d, -8)]
    bob = [(0, 0.01), (k(0.15), 0.03), (k(0.35), -0.02), (k(0.5), 0.01), (k(0.65), 0.03), (k(0.85), -0.02), (d, 0.01)]
    return cycle("Run", "Person (side)", d, SIDE_REST, tr, bob)


def idle_side():
    d = 2.0
    torso = [(0, 0), (1, -2), (d, 0)]
    tr = {"torso": torso, "head": [(0, 0), (1, 3), (d, 0)]}
    for s in ("near", "far"):
        tr[f"{s} thigh"] = to_local([(0, 0), (1, 0), (d, 0)], torso)
        tr[f"{s} upper arm"] = to_local([(0, 2), (1, 5), (d, 2)], torso)
        tr[f"{s} forearm"] = [(0, -8), (1, -12), (d, -8)]
    return cycle("Idle", "Person (side)", d, SIDE_REST, tr, [(0, 0), (1, 0.006), (d, 0)])


def jump_side():
    d = 1.2
    times = [0, 0.25, 0.4, 0.6, 0.85, 0.95, d]
    torso = list(zip(times, [0, 25, 5, 10, 15, 20, 0]))
    thigh_w = list(zip(times, [0, -50, 5, -40, -20, -45, 0]))
    shin = list(zip(times, [0, 90, 0, 70, 30, 85, 0]))
    foot = list(zip(times, [0, -40, 30, -20, -10, -40, 0]))
    arm_w = list(zip(times, [0, 50, -140, -100, -30, 20, 0]))
    forearm = list(zip(times, [0, -10, -20, -40, -30, -20, 0]))
    tr = {"torso": torso, "head": list(zip(times, [0, -15, -5, 0, -10, -15, 0]))}
    for s in ("near", "far"):
        tr[f"{s} thigh"] = to_local(thigh_w, torso)
        tr[f"{s} shin"], tr[f"{s} foot"] = shin, foot
        tr[f"{s} upper arm"] = to_local(arm_w, torso)
        tr[f"{s} forearm"] = forearm
    bob = list(zip(times, [0, 0.12, -0.1, -0.35, 0, 0.1, 0]))
    return cycle("Jump", "Person (side)", d, SIDE_REST, tr, bob)


def punch():
    d = 0.8
    times = [0, 0.2, 0.3, 0.4, d]
    torso = list(zip(times, [3, 0, 12, 12, 3]))
    tr = {"torso": torso, "head": list(zip(times, [-3, 0, -10, -10, -3]))}
    tr["near upper arm"] = to_local(list(zip(times, [-30, -20, -90, -90, -30])), torso)
    tr["near forearm"] = list(zip(times, [-110, -120, -5, -5, -110]))
    tr["far upper arm"] = to_local(list(zip(times, [-20, -20, -10, -10, -20])), torso)
    tr["far forearm"] = list(zip(times, [-120, -120, -130, -130, -120]))
    tr["near thigh"] = to_local([(t, -15) for t in times], torso)
    tr["far thigh"] = to_local([(t, 15) for t in times], torso)
    for s in ("near", "far"):
        tr[f"{s} shin"] = list(zip(times, [10, 12, 8, 8, 10]))
    return cycle("Punch", "Person (side)", d, SIDE_REST, tr)


def idle_front():
    d = 2.0
    tr = {"torso": [(0, 0), (d, 0)], "head": [(0, 0), (1, 2), (d, 0)]}
    tr["left upper arm"] = [(0, 3), (1, 6), (d, 3)]
    tr["right upper arm"] = [(0, -3), (1, -6), (d, -3)]
    tr["left forearm"] = [(0, -3), (1, -6), (d, -3)]
    tr["right forearm"] = [(0, 3), (1, 6), (d, 3)]
    return cycle("Idle", "Person (front)", d, FRONT_REST, tr, [(0, 0), (1, 0.006), (d, 0)])


def wave():
    d = 1.6
    tr = {"head": [(0, 0), (0.3, 5), (1.3, 5), (d, 0)]}
    tr["right upper arm"] = [(0, 0), (0.3, -140), (1.3, -140), (d, 0)]
    tr["right forearm"] = [(0, 0), (0.3, -40), (0.5, 0), (0.7, -40), (0.9, 0), (1.1, -40), (1.3, -40), (d, 0)]
    tr["left upper arm"] = [(0, 0), (d, 0)]
    return cycle("Wave", "Person (front)", d, FRONT_REST, tr)


def jump_front():
    d = 1.2
    times = [0, 0.25, 0.4, 0.6, 0.85, 0.95, d]
    tr = {"torso": [(t, 0) for t in times]}
    tr["left thigh"] = list(zip(times, [0, 25, 0, 30, 10, 20, 0]))
    tr["left shin"] = list(zip(times, [0, -45, 0, -50, -15, -40, 0]))
    tr["right thigh"] = [(t, -v) for t, v in tr["left thigh"]]
    tr["right shin"] = [(t, -v) for t, v in tr["left shin"]]
    tr["left upper arm"] = list(zip(times, [0, 10, 150, 120, 40, 15, 0]))
    tr["right upper arm"] = [(t, -v) for t, v in tr["left upper arm"]]
    tr["left forearm"] = list(zip(times, [0, 0, 10, 20, 10, 0, 0]))
    tr["right forearm"] = [(t, -v) for t, v in tr["left forearm"]]
    bob = list(zip(times, [0, 0.08, -0.1, -0.35, 0, 0.07, 0]))
    return cycle("Jump", "Person (front)", d, FRONT_REST, tr, bob)


files = {
    "side_walk": walk(), "side_run": run(), "side_idle": idle_side(), "side_jump": jump_side(),
    "side_punch": punch(), "front_idle": idle_front(), "front_wave": wave(), "front_jump": jump_front(),
}
os.makedirs(OUT, exist_ok=True)
for name, data in files.items():
    lines = ["{"]
    lines.append(f'  "name": {json.dumps(data["name"])},')
    lines.append(f'  "template": {json.dumps(data["template"])},')
    lines.append(f'  "duration": {data["duration"]},')
    lines.append(f'  "rest": {json.dumps(data["rest"])},')
    lines.append('  "tracks": {')
    items = list(data["tracks"].items())
    for i, (k, v) in enumerate(items):
        lines.append(f'    {json.dumps(k)}: {json.dumps(v)}' + ("," if i + 1 < len(items) else ""))
    extra = [k for k in ("bob", "sway") if k in data]
    lines.append("  }" + ("," if extra else ""))
    for i, k in enumerate(extra):
        lines.append(f'  "{k}": {json.dumps(data[k])}' + ("," if i + 1 < len(extra) else ""))
    lines.append("}")
    with open(os.path.join(OUT, name + ".json"), "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
print("wrote", len(files))
