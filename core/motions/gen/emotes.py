"""Emotes: greetings and feelings."""
from .common import *


def wave_side():
    d = 1.6
    times = [0, 0.3, 0.5, 0.7, 0.9, 1.1, 1.3, d]
    tr = side_pose(
        times,
        torso=[0, -3, -3, -3, -3, -3, -3, 0],
        near={"upper arm": [0, -155, -155, -155, -155, -155, -155, 0],
              "forearm": [0, -50, -10, -50, -10, -50, -50, 0]},
        far={"upper arm": [0] * 8, "forearm": [-5] * 8},
    )
    tr["head"] = keys(times, [0, -5, -5, -5, -5, -5, -5, 0])
    return motion("Wave", SIDE, d, tr)


def wave_front():
    d = 1.6
    tr = {"head": [(0, 0), (0.3, 5), (1.3, 5), (d, 0)]}
    tr["right upper arm"] = [(0, 0), (0.3, -140), (1.3, -140), (d, 0)]
    tr["right forearm"] = [(0, 0), (0.3, -40), (0.5, 0), (0.7, -40), (0.9, 0), (1.1, -40), (1.3, -40), (d, 0)]
    tr["left upper arm"] = [(0, 0), (d, 0)]
    return motion("Wave", FRONT, d, tr)


def victory_side():
    """Fist pumps with little hops."""
    d = 1.0
    times = [0, 0.25, 0.5, 0.75, d]
    tr = side_pose(
        times,
        torso=[-5, -8, -5, -8, -5],
        near={"upper arm": [-150, -170, -150, -170, -150], "forearm": [-60, -10, -60, -10, -60]},
        far={"upper arm": [10, 5, 10, 5, 10], "forearm": [-100, -110, -100, -110, -100]},
        legs={"shin": [15, 0, 15, 0, 15], "thigh": [-8, 0, -8, 0, -8]},
    )
    tr["head"] = keys(times, [-10, -15, -10, -15, -10])
    return motion("Victory", SIDE, d, tr, bob=keys(times, [0.01, -0.04, 0.01, -0.04, 0.01]))


def victory_front():
    """Both arms up in a V, bouncing."""
    d = 1.0
    times = [0, 0.25, 0.5, 0.75, d]
    tr = {"torso": keys(times, [0] * 5), "head": keys(times, [0, 3, 0, -3, 0])}
    tr["left upper arm"] = keys(times, [140, 150, 140, 150, 140])
    tr["left forearm"] = keys(times, [10, 0, 10, 0, 10])
    tr["left thigh"] = keys(times, [5, 0, 5, 0, 5])
    tr["left shin"] = keys(times, [-10, 0, -10, 0, -10])
    for part in ("upper arm", "forearm", "thigh", "shin"):
        tr[f"right {part}"] = [(t, -v) for t, v in tr[f"left {part}"]]
    return motion("Victory", FRONT, d, tr, bob=keys(times, [0.01, -0.04, 0.01, -0.04, 0.01]))


MOTIONS = [wave_front(), wave_side(), victory_side(), victory_front()]
