"""Move: getting from place to place (all in place, for game sprites)."""
from .common import *


def walk():
    d = 1.0
    torso = [(0, 5), (d, 5)]
    thigh = [(0, -25), (0.25, -2), (0.5, 20), (0.75, -20), (d, -25)]
    shin = [(0, 2), (0.12, 15), (0.25, 5), (0.5, 15), (0.62, 55), (0.75, 60), (0.88, 20), (d, 2)]
    foot = [(0, -15), (0.12, 0), (0.25, 0), (0.5, 20), (0.62, 25), (0.75, 0), (0.88, -10), (d, -15)]
    arm = [(0, 20), (0.5, -20), (d, 20)]
    forearm = [(0, -10), (0.5, -30), (d, -10)]
    tr = side_cycle(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -3), (d, -3)]
    bob = [(0, 0), (0.1, 0.012), (0.25, -0.012), (0.5, 0), (0.6, 0.012), (0.75, -0.012), (d, 0)]
    return motion("Walk", SIDE, d, tr, bob)


def run():
    d = 0.6
    torso = [(0, 12), (d, 12)]
    k = lambda f: f * d
    thigh = [(0, -30), (k(0.15), 0), (k(0.35), 35), (k(0.5), 10), (k(0.75), -55), (d, -30)]
    shin = [(0, 10), (k(0.15), 35), (k(0.35), 15), (k(0.5), 90), (k(0.65), 110), (k(0.8), 50), (d, 10)]
    foot = [(0, -10), (k(0.15), 5), (k(0.35), 30), (k(0.5), 20), (k(0.75), -10), (d, -10)]
    arm = [(0, 40), (k(0.5), -45), (d, 40)]
    forearm = [(0, -70), (k(0.5), -100), (d, -70)]
    tr = side_cycle(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -8), (d, -8)]
    bob = [(0, 0.01), (k(0.15), 0.03), (k(0.35), -0.02), (k(0.5), 0.01), (k(0.65), 0.03), (k(0.85), -0.02), (d, 0.01)]
    return motion("Run", SIDE, d, tr, bob)


def crouch_walk():
    """Walking low, knees bent, body forward."""
    d = 1.2
    k = lambda f: round(f * d, 4)
    torso = [(0, 30), (d, 30)]
    thigh = [(0, -65), (k(0.25), -45), (k(0.5), -25), (k(0.75), -60), (d, -65)]
    shin = [(0, 75), (k(0.25), 90), (k(0.5), 100), (k(0.62), 115), (k(0.75), 110), (d, 75)]
    foot = [(0, -10), (k(0.25), -40), (k(0.5), -40), (k(0.62), -20), (k(0.75), -30), (d, -10)]
    arm = [(0, -15), (k(0.5), -45), (d, -15)]
    forearm = [(0, -50), (k(0.5), -60), (d, -50)]
    tr = side_cycle(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -25), (d, -25)]
    return motion("Crouch-walk", SIDE, d, tr, cycle_bob(d, thigh, shin))


def sneak():
    """Tiptoeing: slow, high steps, hands held up."""
    d = 1.6
    k = lambda f: round(f * d, 4)
    torso = [(0, 10), (d, 10)]
    thigh = [(0, -30), (k(0.25), -5), (k(0.5), 15), (k(0.7), -55), (d, -30)]
    shin = [(0, 10), (k(0.25), 15), (k(0.5), 25), (k(0.7), 100), (k(0.85), 50), (d, 10)]
    foot = [(0, 10), (k(0.25), 20), (k(0.5), 30), (k(0.7), 25), (d, 10)]
    arm = [(0, -30), (k(0.5), -40), (d, -30)]
    forearm = [(0, -110), (k(0.5), -100), (d, -110)]
    tr = side_cycle(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -10), (d, -10)]
    return motion("Sneak", SIDE, d, tr, cycle_bob(d, thigh, shin))


def push():
    """Leaning into something heavy and pushing it along."""
    d = 1.6
    k = lambda f: round(f * d, 4)
    torso = [(0, 35), (d, 35)]
    thigh = [(0, -40), (k(0.25), -20), (k(0.5), 15), (k(0.75), -35), (d, -40)]
    shin = [(0, 30), (k(0.25), 25), (k(0.5), 20), (k(0.62), 60), (k(0.75), 70), (d, 30)]
    foot = [(0, -20), (k(0.25), -10), (k(0.5), 30), (k(0.75), 0), (d, -20)]
    arm = [(0, -80), (k(0.5), -85), (d, -80)]
    forearm = [(0, -35), (k(0.5), -30), (d, -35)]
    tr = side_cycle(d, torso, thigh, shin, foot, arm, forearm)
    tr["head"] = [(0, -30), (d, -30)]
    return motion("Push", SIDE, d, tr, cycle_bob(d, thigh, shin))


def dash():
    """A quick burst forward: wind up, stretch out, recover. Plays once."""
    times = [0, 0.1, 0.2, 0.4, 0.6]
    tr, bob = side_pose(
        times,
        torso=[0, 15, 40, 40, 0],
        near={"upper arm": [0, -20, 60, 60, 0], "forearm": [0, -30, -20, -20, 0]},
        far={"upper arm": [0, 20, 70, 70, 0], "forearm": [0, -30, -15, -15, 0]},
        legs={"near thigh": [0, -30, -60, -60, 0], "far thigh": [0, 10, 40, 40, 0],
              "near shin": [0, 50, 40, 40, 0], "far shin": [0, 40, 30, 30, 0],
              "near foot": [0, -20, -20, -20, 0], "far foot": [0, 10, 30, 30, 0]},
        ground=True,
    )
    tr["head"] = keys(times, [0, -10, -30, -30, 0])
    return motion("Dash", SIDE, 0.6, tr, bob, loop=False)


from . import creatures

MOTIONS = [walk(), run(), crouch_walk(), sneak(), push(), dash()] + creatures.MOVE
