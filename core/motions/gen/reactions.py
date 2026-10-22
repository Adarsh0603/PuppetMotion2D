"""Reactions: getting hit and going down (both play once)."""
from .common import *


def hurt_side():
    """A hit from the front: the body snaps back, then recovers."""
    d = 0.55
    times = [0, 0.08, 0.25, d]
    tr, bob = side_pose(
        times,
        torso=[0, -25, -12, 0],
        near={"upper arm": [0, -35, -15, 0], "forearm": [0, -40, -20, 0]},
        far={"upper arm": [0, -25, -10, 0], "forearm": [0, -50, -25, 0]},
        legs={"near thigh": [0, -20, -10, 0], "far thigh": [0, 10, 5, 0], "near shin": [0, 25, 12, 0], "far shin": [0, 15, 8, 0]},
        ground=True,
    )
    tr["head"] = keys(times, [0, -12, -5, 0])
    return motion("Hurt", SIDE, d, tr, bob, sway=keys(times, [0, -0.03, -0.015, 0]), loop=False)


def death_side():
    """Staggers back, drops to the knees and slumps forward."""
    d = 1.6
    times = [0, 0.15, 0.45, 0.75, 1.1, d]
    tr, bob = side_pose(
        times,
        torso=[0, -25, -10, 10, 50, 55],
        near={"upper arm": [0, -40, -20, 10, 50, 55], "forearm": [0, -40, -30, -20, -10, -10]},
        far={"upper arm": [0, -30, -10, 15, 50, 55], "forearm": [0, -50, -30, -20, -10, -10]},
        legs={"thigh": [0, -15, -30, -10, 0, 0], "shin": [0, 20, 70, 90, 90, 90], "foot": [0, 0, -20, 30, 40, 40]},
        ground=True,
    )
    tr["head"] = keys(times, [0, -25, 0, 15, 35, 40])
    return motion("Death", SIDE, d, tr, bob, loop=False)


def hurt_front():
    d = 0.55
    times = [0, 0.08, 0.25, d]
    tr = {"torso": keys(times, [0, -6, -3, 0]), "head": keys(times, [0, 15, 6, 0])}
    tr["left upper arm"] = keys(times, [0, -15, -6, 0])
    tr["left forearm"] = keys(times, [0, -40, -15, 0])
    tr["right upper arm"] = keys(times, [0, 15, 6, 0])
    tr["right forearm"] = keys(times, [0, 40, 15, 0])
    return motion("Hurt", FRONT, d, tr, bob=keys(times, [0, 0.02, 0.01, 0]), loop=False)


MOTIONS = [hurt_side(), hurt_front(), death_side()]
