"""Jump: a whole jump to preview, and its parts for games.

In a game the engine moves the character up and down; the parts only change
the pose: Take-off (once) → Rising (loop while going up) → Falling (loop
while coming down) → Land (once). Each part starts in the pose the one
before it ends in.
"""
from .common import *

# Poses shared by the parts, so they join up: (torso, arms, legs) for side_pose.
# Arms stay up-and-forward rather than straight up: in a single flat side-view
# drawing the arm covers the torso, and pointing it straight up tears the body.
RISING = {"torso": 5, "upper arm": -120, "forearm": -20, "thigh": -35, "shin": 60, "foot": 10}
FALLING = {"torso": 0, "upper arm": -105, "forearm": -30, "thigh": -20, "shin": 20, "foot": -5}


def from_poses(times, poses, ground=False, near_leg=None):
    """side_pose from a list of pose dicts (both sides the same unless `near_leg` adds an offset per key)."""
    col = lambda k: [p[k] for p in poses]
    arm = {"upper arm": col("upper arm"), "forearm": col("forearm")}
    legs = {"shin": col("shin"), "foot": col("foot")}
    if near_leg:
        legs["near thigh"] = [p["thigh"] + o for p, o in zip(poses, near_leg)]
        legs["far thigh"] = [p["thigh"] - o for p, o in zip(poses, near_leg)]
    else:
        legs["thigh"] = col("thigh")
    return side_pose(times, col("torso"), near=dict(arm), far=dict(arm), legs=legs, ground=ground)


STAND = {"torso": 0, "upper arm": 0, "forearm": 0, "thigh": 0, "shin": 0, "foot": 0}
CROUCH = {"torso": 25, "upper arm": 50, "forearm": -10, "thigh": -50, "shin": 90, "foot": -40}
STRETCH = {"torso": 5, "upper arm": -130, "forearm": -10, "thigh": 5, "shin": 0, "foot": 30}
IMPACT = {"torso": 20, "upper arm": 20, "forearm": -20, "thigh": -45, "shin": 85, "foot": -40}


def jump_side():
    d = 1.2
    times = [0, 0.25, 0.4, 0.6, 0.85, 0.95, d]
    tr = side_pose(
        times,
        torso=[0, 25, 5, 10, 15, 20, 0],
        near={"upper arm": [0, 50, -125, -100, -30, 20, 0], "forearm": [0, -10, -20, -40, -30, -20, 0]},
        far={"upper arm": [0, 50, -125, -100, -30, 20, 0], "forearm": [0, -10, -20, -40, -30, -20, 0]},
        legs={"thigh": [0, -50, 5, -40, -20, -45, 0], "shin": [0, 90, 0, 70, 30, 85, 0], "foot": [0, -40, 30, -20, -10, -40, 0]},
    )
    tr["head"] = keys(times, [0, -15, -5, 0, -10, -15, 0])
    return motion("Jump", SIDE, d, tr, bob=keys(times, [0, 0.12, -0.1, -0.35, 0, 0.1, 0]))


def take_off():
    times = [0, 0.15, 0.3, 0.4]
    tr, bob = from_poses(times, [STAND, CROUCH, STRETCH, RISING], ground=True, near_leg=[0, 0, 0, -10])
    # Off the ground from the stretch on: the engine lifts the character.
    bob = [(t, b if t < 0.3 else 0.0) for t, b in bob]
    tr["head"] = keys(times, [0, -15, -10, -5])
    return motion("Take-off", SIDE, 0.4, tr, bob, loop=False)


def rising():
    d = 0.6
    times = [0, 0.3, d]
    wiggle = {**RISING, "shin": RISING["shin"] + 10, "upper arm": RISING["upper arm"] + 8}
    tr = from_poses(times, [RISING, wiggle, RISING], near_leg=[-10, -14, -10])
    tr["head"] = keys(times, [-5, -7, -5])
    return motion("Rising", SIDE, d, tr)


def falling():
    d = 0.5
    times = [0, 0.25, d]
    flail = {**FALLING, "upper arm": FALLING["upper arm"] - 15, "forearm": FALLING["forearm"] - 15}
    tr = from_poses(times, [FALLING, flail, FALLING], near_leg=[-15, -10, -15])
    tr["head"] = keys(times, [5, 8, 5])
    return motion("Falling", SIDE, d, tr)


def land():
    times = [0, 0.08, 0.2, 0.45]
    soften = {**IMPACT, "torso": 12, "thigh": -25, "shin": 45, "foot": -20}
    poses = [FALLING, IMPACT, soften, STAND]
    tr, bob = from_poses(times, poses, ground=True, near_leg=[-15, 0, 0, 0])
    tr["head"] = keys(times, [5, -10, -5, 0])
    return motion("Land", SIDE, 0.45, tr, bob, loop=False)


def jump_front():
    d = 1.2
    times = [0, 0.25, 0.4, 0.6, 0.85, 0.95, d]
    tr = {"torso": [(t, 0) for t in times]}
    tr["left thigh"] = keys(times, [0, 25, 0, 30, 10, 20, 0])
    tr["left shin"] = keys(times, [0, -45, 0, -50, -15, -40, 0])
    tr["left upper arm"] = keys(times, [0, 10, 150, 120, 40, 15, 0])
    tr["left forearm"] = keys(times, [0, 0, 10, 20, 10, 0, 0])
    for part in ("thigh", "shin", "upper arm", "forearm"):
        tr[f"right {part}"] = [(t, -v) for t, v in tr[f"left {part}"]]
    return motion("Jump", FRONT, d, tr, bob=keys(times, [0, 0.08, -0.1, -0.35, 0, 0.07, 0]))


MOTIONS = [jump_side(), jump_front(), take_off(), rising(), falling(), land()]
