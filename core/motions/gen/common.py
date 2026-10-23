"""Shared helpers for writing motions.

Angle convention (as in the app): degrees, positive turns clockwise on screen.
Facing right, that swings a hanging limb backward, a forward-pointing foot
toe-down, and an upright torso or head forward. Limbs hanging from the torso
are usually written relative to the *world* and converted with `to_local`,
so a lean doesn't tilt the legs.
"""

SIDE = "Person (side)"
FRONT = "Person (front)"

SIDE_REST = {
    "torso": -90, "head": -90,
    **{f"{s} {b}": 90 for s in ("near", "far") for b in ("thigh", "shin", "upper arm", "forearm")},
    "near foot": 0, "far foot": 0,
}
FRONT_REST = {
    "torso": -90, "head": -90,
    **{f"{s} {b}": 90 for s in ("left", "right") for b in ("thigh", "shin", "upper arm", "forearm")},
}


FOUR_LEGS_REST = {
    "body": 0,
    **{f"{s} {e} {b}": 90 for s in ("near", "far") for e in ("front", "back") for b in ("leg", "shin")},
}
# Neutral direction per bone (0 = right, 90 = down) for a figure facing right.
# Bones left out keep the angle they're drawn at (tails, tentacles, blobs).
REST = {SIDE: SIDE_REST, FRONT: FRONT_REST, "Four legs (side)": FOUR_LEGS_REST}


def interp(keys, t):
    """Value of a key list at time t (straight lines between keys)."""
    keys = sorted(keys)
    if t <= keys[0][0]:
        return keys[0][1]
    for (a, va), (b, vb) in zip(keys, keys[1:]):
        if a <= t <= b:
            return va + (vb - va) * (t - a) / (b - a)
    return keys[-1][1]


def shift(keys, by, duration):
    """The same loop, started `by` seconds later (for the other leg or arm)."""
    loop = sorted((round((t + by) % duration, 4), v) for t, v in keys if t < duration)
    start = interp(keys, (duration - by) % duration)
    out = [(0.0, start)] + [k for k in loop if k[0] > 0] + [(duration, start)]
    dedup = []
    for k in out:
        if not dedup or abs(dedup[-1][0] - k[0]) > 1e-3:
            dedup.append(k)
    return dedup


def to_local(world, torso):
    """Angles relative to the world → relative to a torso that leans by `torso`."""
    return [(t, round(v - interp(torso, t), 1)) for t, v in world]


def keys(times, values):
    return list(zip(times, values))


def both(tracks, side_names, values):
    """Give the same keys to both sides (near and far, or left and right)."""
    for s in side_names:
        tracks[s] = values


def side_cycle(duration, torso, thigh_w, shin, foot, arm_w, forearm):
    """Near limbs as given, far limbs half a cycle later (a walk or run)."""
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


# Thigh and shin length as shares of the figure height (Person (side) template).
THIGH, SHIN = 0.20, 0.19


def leg_height(thigh_w, shin):
    """How far below the hip the foot is, for a thigh at `thigh_w` (world) and a knee bent by `shin`."""
    import math
    a = math.radians(thigh_w)
    return THIGH * math.cos(a) + SHIN * math.cos(a + math.radians(shin))


def grounded_bob(times, legs):
    """Bob that keeps the lower foot on the ground: the hip drops by however
    much shorter the longer leg is than when standing straight.
    `legs`: list of (thigh world keys, shin keys) per leg."""
    stand = THIGH + SHIN
    return [(t, round(stand - max(leg_height(interp(th, t), interp(sh, t)) for th, sh in legs), 4)) for t in times]


def cycle_bob(duration, thigh_w, shin, steps=8):
    """`grounded_bob` for a `side_cycle` (far leg half a cycle behind), at
    `steps` evenly spaced times; the last equals the first so it loops."""
    half = duration / 2
    legs = [(thigh_w, shin), (shift(thigh_w, half, duration), shift(shin, half, duration))]
    times = [round(duration * i / steps, 4) for i in range(steps + 1)]
    bob = grounded_bob(times, legs)
    bob[-1] = (duration, bob[0][1])
    return bob


def side_pose(times, torso, near=None, far=None, legs=None, ground=False):
    """A side-view motion from lists of values per key time.

    `near` / `far`: dicts of "upper arm", "forearm" (world for the upper arm).
    `legs`: dict of "thigh" (world), "shin", "foot" for both legs, or
    "near thigh", "far thigh", ... to give each leg its own.
    With `ground`, returns (tracks, bob) where bob keeps the feet on the floor.
    """
    tr = {"torso": keys(times, torso)}
    torso_k = tr["torso"]
    for side, arm in (("near", near or {}), ("far", far or {})):
        if "upper arm" in arm:
            tr[f"{side} upper arm"] = to_local(keys(times, arm["upper arm"]), torso_k)
        if "forearm" in arm:
            tr[f"{side} forearm"] = keys(times, arm["forearm"])
    world = {}
    for name, vals in (legs or {}).items():
        names = [name] if name.startswith(("near ", "far ")) else [f"near {name}", f"far {name}"]
        for n in names:
            v = keys(times, vals)
            world[n] = v
            tr[n] = to_local(v, torso_k) if n.endswith("thigh") else v
    if not ground:
        return tr
    zero = keys(times, [0] * len(times))
    legs_w = [(world.get(f"{s} thigh", zero), world.get(f"{s} shin", zero)) for s in ("near", "far")]
    return tr, grounded_bob(times, legs_w)


def motion(name, template, duration, tracks, bob=None, sway=None, loop=True):
    """One motion, as written to JSON. `bob` (down +) and `sway` (forward +) are
    shares of the figure's height."""
    rest = REST.get(template, {})
    return {"name": name, "template": template, "duration": duration, "loop": loop, "rest": rest,
            "tracks": tracks, "bob": bob or [], "sway": sway or []}
