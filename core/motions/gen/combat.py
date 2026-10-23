"""Combat: attacks and defence (attacks play once and end where they start).

A weapon drawn in the hand (sword, bow, wand) moves with the forearm.
"""
from .common import *

# Fighting stance legs, used by most attacks.
STANCE = {"near thigh": -15, "far thigh": 15, "near shin": 10, "far shin": 12}


def stance_legs(n, **override):
    legs = {k: [v] * n for k, v in STANCE.items()}
    legs.update(override)
    return legs


def punch():
    d = 0.8
    times = [0, 0.2, 0.3, 0.4, d]
    tr = side_pose(
        times,
        torso=[3, 0, 12, 12, 3],
        near={"upper arm": [-30, -20, -90, -90, -30], "forearm": [-110, -120, -5, -5, -110]},
        far={"upper arm": [-20, -20, -10, -10, -20], "forearm": [-120, -120, -130, -130, -120]},
        legs={"near thigh": [-15] * 5, "far thigh": [15] * 5, "shin": [10, 12, 8, 8, 10]},
    )
    tr["head"] = keys(times, [-3, 0, -10, -10, -3])
    return motion("Punch", SIDE, d, tr, loop=False)


def kick():
    d = 0.8
    times = [0, 0.15, 0.3, 0.42, 0.6, d]
    tr, bob = side_pose(
        times,
        torso=[3, -5, -15, -15, -5, 3],
        near={"upper arm": [-30, -40, -20, -20, -35, -30], "forearm": [-110, -100, -90, -90, -100, -110]},
        far={"upper arm": [-20, 10, 30, 30, 0, -20], "forearm": [-120, -90, -60, -60, -90, -120]},
        legs={"near thigh": [-15, -85, -85, -85, -60, -15], "near shin": [10, 120, 5, 5, 90, 10],
              "near foot": [0, 20, 25, 25, 10, 0], "far thigh": [15, 5, 10, 10, 5, 15], "far shin": [12, 10, 5, 5, 10, 12]},
        ground=True,
    )
    tr["head"] = keys(times, [-3, 5, 10, 10, 5, -3])
    return motion("Kick", SIDE, d, tr, bob, loop=False)


def slash():
    """An overhead sword swing with the near arm."""
    d = 0.8
    times = [0, 0.25, 0.38, 0.48, 0.8]
    tr, bob = side_pose(
        times,
        torso=[5, -8, 20, 25, 5],
        near={"upper arm": [-40, -170, -60, 10, -40], "forearm": [-60, -70, -10, -20, -60]},
        far={"upper arm": [-20, 10, -30, -40, -20], "forearm": [-100, -90, -60, -50, -100]},
        legs=stance_legs(5, **{"near thigh": [-15, -10, -30, -30, -15], "near shin": [10, 5, 25, 25, 10]}),
        ground=True,
    )
    tr["head"] = keys(times, [-5, 5, -15, -15, -5])
    return motion("Sword slash", SIDE, d, tr, bob, loop=False)


def block():
    """Guard up, forearms in front of the face (a loop to hold)."""
    d = 1.2
    times = [0, 0.6, d]
    tr, bob = side_pose(
        times,
        torso=[8, 10, 8],
        near={"upper arm": [-45, -47, -45], "forearm": [-120, -122, -120]},
        far={"upper arm": [-35, -37, -35], "forearm": [-125, -127, -125]},
        legs=stance_legs(3, **{"near shin": [20, 24, 20], "far shin": [22, 26, 22]}),
        ground=True,
    )
    tr["head"] = keys(times, [10, 12, 10])
    return motion("Block", SIDE, d, tr, bob)


def throw():
    """An overarm throw with the near arm."""
    d = 0.9
    times = [0, 0.3, 0.45, 0.6, d]
    tr, bob = side_pose(
        times,
        torso=[3, -12, 20, 28, 3],
        near={"upper arm": [-20, 140, 260, 310, 340], "forearm": [-60, -100, -10, -15, -60]},
        far={"upper arm": [-20, -70, -10, 20, -20], "forearm": [-90, -20, -60, -80, -90]},
        legs=stance_legs(5, **{"near thigh": [-15, -15, -30, -30, -15], "near shin": [10, 5, 25, 25, 10],
                               "far thigh": [15, 20, 25, 25, 15]}),
        ground=True,
    )
    tr["head"] = keys(times, [-3, 5, -10, -10, -3])
    return motion("Throw", SIDE, d, tr, bob, loop=False)


def cast_side():
    """Gather power, then push both hands forward."""
    d = 1.1
    times = [0, 0.35, 0.55, 0.8, d]
    arm = {"upper arm": [0, -30, -90, -90, 0], "forearm": [0, -110, -5, -5, 0]}
    far = {"upper arm": [0, -25, -85, -85, 0], "forearm": [0, -115, -10, -10, 0]}
    tr, bob = side_pose(
        times,
        torso=[0, -10, 15, 15, 0],
        near=arm,
        far=far,
        legs=stance_legs(5, **{"near thigh": [0, -10, -25, -25, 0], "far thigh": [0, 10, 20, 20, 0],
                               "near shin": [0, 10, 20, 20, 0], "far shin": [0, 10, 10, 10, 0]}),
        ground=True,
    )
    tr["head"] = keys(times, [0, 5, -10, -10, 0])
    return motion("Cast spell", SIDE, d, tr, bob, loop=False)


def shoot():
    """Draw a bow (held in the far hand) and loose an arrow."""
    d = 1.2
    times = [0, 0.3, 0.6, 0.68, 0.9, d]
    tr, bob = side_pose(
        times,
        torso=[0, -3, -3, -3, -3, 0],
        far={"upper arm": [0, -90, -90, -90, -90, 0], "forearm": [0, -5, -5, -5, -5, 0]},
        near={"upper arm": [0, -85, -60, -95, -80, 0], "forearm": [0, -60, -150, -40, -60, 0]},
        legs=stance_legs(6, **{"near thigh": [0, -15, -15, -15, -15, 0], "far thigh": [0, 15, 15, 15, 15, 0],
                               "near shin": [0, 8, 8, 8, 8, 0], "far shin": [0, 8, 8, 8, 8, 0]}),
        ground=True,
    )
    tr["head"] = keys(times, [0, -5, -5, -5, -5, 0])
    return motion("Shoot", SIDE, d, tr, bob, loop=False)


def cast_front():
    """Arms sweep up from the sides, hold, and come down."""
    d = 1.1
    times = [0, 0.4, 0.75, d]
    tr = {"torso": keys(times, [0] * 4), "head": keys(times, [0] * 4)}
    tr["left upper arm"] = keys(times, [0, 165, 160, 0])
    tr["left forearm"] = keys(times, [0, 10, 15, 0])
    for part in ("upper arm", "forearm"):
        tr[f"right {part}"] = [(t, -v) for t, v in tr[f"left {part}"]]
    return motion("Cast spell", FRONT, d, tr, bob=keys(times, [0, -0.02, -0.02, 0]), loop=False)


MOTIONS = [punch(), kick(), slash(), block(), throw(), cast_side(), cast_front(), shoot()]
