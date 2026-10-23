"""Basics: standing, breathing, small everyday motions."""
from .common import *


def idle_side():
    d = 2.0
    torso = [(0, 0), (1, -2), (d, 0)]
    tr = {"torso": torso, "head": [(0, 0), (1, 3), (d, 0)]}
    for s in ("near", "far"):
        tr[f"{s} thigh"] = to_local([(0, 0), (1, 0), (d, 0)], torso)
        tr[f"{s} upper arm"] = to_local([(0, 2), (1, 5), (d, 2)], torso)
        tr[f"{s} forearm"] = [(0, -8), (1, -12), (d, -8)]
    return motion("Idle", SIDE, d, tr, bob=[(0, 0), (1, 0.006), (d, 0)])


def crouch_side():
    """Crouched low and breathing (a crouch idle)."""
    d = 1.6
    times = [0, 0.8, d]
    tr, bob = side_pose(
        times,
        torso=[30, 33, 30],
        near={"upper arm": [-35, -38, -35], "forearm": [-45, -50, -45]},
        far={"upper arm": [-25, -28, -25], "forearm": [-50, -55, -50]},
        legs={"near thigh": [-55, -57, -55], "far thigh": [-35, -37, -35],
              "near shin": [95, 98, 95], "far shin": [100, 103, 100],
              "near foot": [-40, -40, -40], "far foot": [-40, -40, -40]},
        ground=True,
    )
    tr["head"] = keys(times, [-25, -27, -25])
    return motion("Crouch", SIDE, d, tr, bob)


def talk_side():
    d = 2.0
    times = [0, 0.4, 0.8, 1.2, 1.6, d]
    tr = side_pose(
        times,
        torso=[2, 3, 2, 4, 2, 2],
        near={"upper arm": [-15, -30, -20, -35, -15, -15], "forearm": [-60, -95, -70, -100, -60, -60]},
        far={"upper arm": [5, 5, 5, 5, 5, 5], "forearm": [-15, -15, -15, -15, -15, -15]},
    )
    tr["head"] = keys(times, [0, -6, 3, -5, 2, 0])
    return motion("Talk", SIDE, d, tr)


def sit_side():
    """Sitting on a seat at knee height, breathing."""
    d = 2.0
    times = [0, 1, d]
    tr, bob = side_pose(
        times,
        torso=[-3, -5, -3],
        near={"upper arm": [-20, -22, -20], "forearm": [-70, -72, -70]},
        far={"upper arm": [-15, -17, -15], "forearm": [-75, -77, -75]},
        legs={"thigh": [-88, -88, -88], "shin": [88, 88, 88], "foot": [0, 0, 0]},
        ground=True,
    )
    tr["head"] = keys(times, [3, 0, 3])
    return motion("Sit", SIDE, d, tr, bob)


def doze_side():
    """Dozing off standing up: the head sinks slowly, then jerks awake."""
    d = 3.0
    times = [0, 2.2, 2.45, d]
    tr = side_pose(
        times,
        torso=[2, 8, 0, 2],
        near={"upper arm": [3, 0, 8, 3], "forearm": [-8, -5, -15, -8]},
        far={"upper arm": [3, 0, 8, 3], "forearm": [-8, -5, -15, -8]},
    )
    tr["head"] = keys(times, [5, 35, -5, 5])
    return motion("Doze", SIDE, d, tr, bob=keys(times, [0, 0.01, -0.005, 0]))


def idle_front():
    d = 2.0
    tr = {"torso": [(0, 0), (d, 0)], "head": [(0, 0), (1, 2), (d, 0)]}
    tr["left upper arm"] = [(0, 3), (1, 6), (d, 3)]
    tr["right upper arm"] = [(0, -3), (1, -6), (d, -3)]
    tr["left forearm"] = [(0, -3), (1, -6), (d, -3)]
    tr["right forearm"] = [(0, 3), (1, 6), (d, 3)]
    return motion("Idle", FRONT, d, tr, bob=[(0, 0), (1, 0.006), (d, 0)])


def talk_front():
    d = 2.0
    times = [0, 0.4, 0.8, 1.2, 1.6, d]
    tr = {"torso": keys(times, [0] * 6), "head": keys(times, [0, 4, -2, 5, -1, 0])}
    tr["right upper arm"] = keys(times, [-10, -25, -15, -30, -10, -10])
    tr["right forearm"] = keys(times, [-40, -80, -55, -90, -40, -40])
    tr["left upper arm"] = keys(times, [3] * 6)
    tr["left forearm"] = keys(times, [-3] * 6)
    return motion("Talk", FRONT, d, tr)


from . import creatures

MOTIONS = [idle_side(), idle_front(), crouch_side(), talk_side(), talk_front(), sit_side(), doze_side()] + creatures.BASICS
