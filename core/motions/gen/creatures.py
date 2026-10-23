"""Motions for the creature templates: Four legs (side), Tentacles and Blob.
They're listed under the usual categories (move.py and basics.py add them).

Angles as everywhere: degrees, positive turns clockwise on screen. For a fox
facing right, that swings a hanging leg backward and tips the head down.
"""
from .common import *


def loop_keys(duration, values, phase=0.0):
    """Evenly spaced keys over one loop, started `phase` of a loop later
    (values wrap round); the last key equals the first."""
    n = len(values)
    keys = [(round(duration * i / n, 4), values[i]) for i in range(n)] + [(duration, values[0])]
    return shift(keys, phase * duration, duration) if phase else keys


# ---------------------------------------------------------------- four legs

LEGS = [f"{s} {e}" for s in ("near", "far") for e in ("front", "back")]


def gait(duration, phases, reach, lift, body, head, tail, bob):
    """A four-legged gait. `phases`: where each leg is in its stride (0..1),
    by leg name. Each leg reaches forward by `reach`, sweeps back while on
    the ground, then lifts (the lower leg folds by `lift`) and swings forward."""
    up = [-reach, -reach * 0.5, 0, reach * 0.5, reach, reach * 0.3, -reach * 0.6, -reach]
    fold = [0, 0, 0, 0, lift * 0.3, lift, lift * 0.6, 0]
    tr = {"body": loop_keys(duration, body)}
    for leg in LEGS:
        p = phases[leg]
        # Front wrists and hind hocks fold opposite ways.
        sign = 1 if "front" in leg else -1
        tr[f"{leg} leg"] = loop_keys(duration, up, p)
        tr[f"{leg} shin"] = loop_keys(duration, [sign * f for f in fold], p)
    tr["head"] = loop_keys(duration, head)
    for i, name in enumerate(("tail", "tail middle", "tail tip")):
        tr[name] = loop_keys(duration, tail, 0.12 * i)
    return motion_for("Four legs (side)", duration, tr, bob)


def fox_walk():
    """Four beats: near back, near front, far back, far front."""
    return {**gait(1.2, {"near back": 0.0, "near front": 0.25, "far back": 0.5, "far front": 0.75},
                   reach=22, lift=45, body=[0, 1, 0, -1], head=[0, 3, 0, 3], tail=[-6, 0, 6, 0],
                   bob=loop_keys(1.2, [0, 0.008, 0, 0.008])), "name": "Walk"}


def fox_run():
    """A gallop: the hind legs push off together, then the front legs reach."""
    d = 0.55
    return {**gait(d, {"near back": 0.0, "far back": 0.08, "near front": 0.5, "far front": 0.58},
                   reach=40, lift=70, body=[-6, 0, 6, 0], head=[6, 0, -6, 0], tail=[-15, -8, 0, -8],
                   bob=loop_keys(d, [0.02, -0.03, 0.02, -0.03])), "name": "Run"}


def fox_idle():
    """Standing: breathing, looking about, a slow tail wag."""
    d = 2.4
    tr = {
        "body": loop_keys(d, [0, -1, 0, 1]),
        "head": loop_keys(d, [0, -6, -6, 0, 4, 4]),
        **{name: loop_keys(d, [10, 0, -10, 0], 0.1 * i) for i, name in enumerate(("tail", "tail middle", "tail tip"))},
    }
    return {**motion_for("Four legs (side)", d, tr, loop_keys(d, [0, 0.004, 0, 0.004])), "name": "Idle"}


# ---------------------------------------------------------------- tentacles

# Tentacles 1 and 3 hang to the left, 2 and 4 to the right, 5 straight down.
# A negative turn brings a left tentacle in toward the middle; the inner pair
# closes half as far, so they don't cross the middle one.
INWARD = {1: -1, 2: 1, 3: -0.5, 4: 0.5, 5: 0}
SEGMENTS = ("", " middle", " tip")


def tentacles(duration, spread, wave, lag):
    """Each tentacle closes in by `spread` degrees and opens again (all together),
    plus a sideways `wave` that runs down each one, `lag` of a loop per segment."""
    tr = {}
    for k, inward in INWARD.items():
        for s, seg in enumerate(SEGMENTS):
            close = [inward * spread * (1 + 0.5 * s) * f for f in (1, 0.4, -0.3, 0.4)]
            sway = [wave * (1 + 0.6 * s) * f for f in (0, 1, 0, -1)]
            tr[f"tentacle {k}{seg}"] = loop_keys(duration, [c + w for c, w in zip(close, sway)], lag * s)
    return tr


def octopus_swim():
    """Pulls the tentacles in and pushes off: the head rises with each stroke."""
    d = 1.2
    tr = tentacles(d, spread=14, wave=4, lag=0.1)
    tr["head"] = loop_keys(d, [0, 2, 0, -2])
    return {**motion_for("Tentacles", d, tr, loop_keys(d, [0, -0.05, -0.02, 0.02])), "name": "Swim"}


def octopus_float():
    """Hanging in the water, tentacles drifting."""
    d = 2.6
    tr = tentacles(d, spread=4, wave=8, lag=0.15)
    tr["head"] = loop_keys(d, [0, 3, 0, -3])
    return {**motion_for("Tentacles", d, tr, loop_keys(d, [0, -0.015, 0, 0.015])), "name": "Float"}


# ---------------------------------------------------------------- blob

def blob_bounce():
    """Squats, springs up, lands, and wobbles to a stop."""
    d = 0.9
    k = lambda f: round(f * d, 4)
    tr = {
        "body": [(0, 0), (k(0.2), 0), (k(0.5), 0), (k(0.7), 3), (k(0.85), -2), (d, 0)],
        "top": [(0, 0), (k(0.15), 4), (k(0.35), -6), (k(0.6), 0), (k(0.72), -10), (k(0.86), 7), (d, 0)],
    }
    bob = [(0, 0), (k(0.15), 0.03), (k(0.45), -0.3), (k(0.7), 0.02), (k(0.85), -0.01), (d, 0)]
    return {**motion_for("Blob", d, tr, bob), "name": "Bounce"}


def blob_jiggle():
    """Sitting still, wobbling."""
    d = 1.6
    tr = {"body": loop_keys(d, [2, 0, -2, 0]), "top": loop_keys(d, [6, 0, -6, 0], 0.15)}
    return {**motion_for("Blob", d, tr, loop_keys(d, [0, 0.01, 0, 0.01])), "name": "Jiggle"}


def motion_for(template, duration, tracks, bob):
    return motion("", template, duration, tracks, bob)


MOVE = [fox_walk(), fox_run(), octopus_swim(), blob_bounce()]
BASICS = [fox_idle(), octopus_float(), blob_jiggle()]
