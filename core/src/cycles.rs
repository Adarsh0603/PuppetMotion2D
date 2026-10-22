//! Ready-made motions (walk, run, idle, …) that can be put on any character
//! rigged from the template they were made for.
//!
//! A cycle's angles are relative to a *neutral* pose, given in the file as each
//! bone's direction (`rest`), not to the way the character happens to be
//! drawn. So a drawing with an arm held up still walks with its arms swinging
//! at its sides: the difference between the drawn and the neutral direction is
//! added to every key. Tracks match bones by name, like copied keys.

use crate::motion::{Ease, Key, Motion};
use crate::rig::clamp_to_limit;
use crate::skeleton::Skeleton;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

/// The cycles that come with the app, in menu order.
const BUILTIN: [&str; 8] = [
    include_str!("../motions/side_idle.json"),
    include_str!("../motions/side_walk.json"),
    include_str!("../motions/side_run.json"),
    include_str!("../motions/side_jump.json"),
    include_str!("../motions/side_punch.json"),
    include_str!("../motions/front_idle.json"),
    include_str!("../motions/front_wave.json"),
    include_str!("../motions/front_jump.json"),
];

/// Share of a cycle's tracks the skeleton must have for the cycle to be offered.
const MATCH_SHARE: f32 = 0.75;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cycle {
    pub name: String,
    /// The template it was made for (a hint; matching goes by bone names).
    #[serde(default)]
    pub template: String,
    /// Length in seconds; the last keys usually equal the first so it loops.
    pub duration: f32,
    /// Neutral direction of each bone in degrees (0 = right, 90 = down), for a
    /// figure facing right. Bones not listed keep the angle they're drawn at.
    #[serde(default)]
    pub rest: BTreeMap<String, f32>,
    /// Bone name → keys, as degrees turned from the neutral direction relative
    /// to the parent (positive turns clockwise on screen, as in `Pose`).
    pub tracks: BTreeMap<String, Vec<CycleKey>>,
    /// Whole-figure up/down movement, as a share of the figure's height (down positive).
    #[serde(default)]
    pub bob: Vec<CycleKey>,
    /// Whole-figure forward/back movement, as a share of the figure's height (forward positive).
    #[serde(default)]
    pub sway: Vec<CycleKey>,
}

/// `[time in seconds, value]`, or `[time, value, "Linear" | "Hold" | "Smooth"]`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CycleKey(pub f32, pub f32, #[serde(default, skip_serializing_if = "is_smooth")] pub Ease);

fn is_smooth(e: &Ease) -> bool {
    *e == Ease::Smooth
}

/// The cycles that come with the app.
pub fn builtin_cycles() -> Vec<Cycle> {
    BUILTIN.iter().map(|json| Cycle::from_json(json).expect("built-in cycle is valid")).collect()
}

impl Cycle {
    /// Read a cycle, checking that it has a length, keys inside it, and sensible numbers.
    pub fn from_json(text: &str) -> Result<Cycle, String> {
        let c: Cycle = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if c.name.trim().is_empty() {
            return Err("the motion has no name".into());
        }
        if !(c.duration.is_finite() && c.duration > 0.0) {
            return Err("the motion's length must be more than 0 seconds".into());
        }
        let tracks = c.tracks.iter().map(|(n, k)| (n.as_str(), k)).chain([("bob", &c.bob), ("sway", &c.sway)]);
        for (name, keys) in tracks {
            for k in keys {
                if !(k.0.is_finite() && k.1.is_finite()) || k.0 < 0.0 || k.0 > c.duration + 1e-3 {
                    return Err(format!("\"{name}\" has a key at {} s, outside the motion's {} s", k.0, c.duration));
                }
            }
        }
        Ok(c)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("cycles always serialize")
    }

    /// Whether `skel` has enough of the bones this cycle moves to be worth offering.
    pub fn matches(&self, skel: &Skeleton) -> bool {
        let named: Vec<&String> = self.tracks.iter().filter(|(_, k)| !k.is_empty()).map(|(n, _)| n).collect();
        let found = named.iter().filter(|n| skel.parts.iter().any(|p| &&p.name == *n)).count();
        found > 0 && found as f32 >= MATCH_SHARE * named.len() as f32
    }

    /// The cycle as keys for `skel`. `figure_h` is the character's height in
    /// pixels (for the whole-figure moves); keys land on whole frames at `fps`.
    /// `facing_left` mirrors it, for side views drawn facing left.
    pub fn to_motion(&self, skel: &Skeleton, figure_h: f32, fps: u32, facing_left: bool) -> Motion {
        let mut motion = Motion { duration: self.duration, fps, ..Motion::default() };
        let grid = motion.clone();
        let snap = |t: f32| grid.snap(t.clamp(0.0, self.duration));
        let flip = if facing_left { -1.0 } else { 1.0 };
        let fix = self.corrections(skel, facing_left);
        for (i, part) in skel.parts.iter().enumerate() {
            let Some(keys) = self.tracks.get(&part.name) else { continue };
            let keys = keys.iter().map(|k| {
                let angle = wrap(flip * k.1.to_radians() + fix[i]);
                Key { t: snap(k.0), value: clamp_to_limit(skel, i, angle), ease: k.2 }
            });
            let keys = dedup(keys.collect());
            if !keys.is_empty() {
                motion.tracks.insert(part.name.clone(), keys);
            }
        }
        let offset = |keys: &[CycleKey], scale: f32| {
            dedup(keys.iter().map(|k| Key { t: snap(k.0), value: k.1 * scale, ease: k.2 }).collect())
        };
        motion.offset_y = offset(&self.bob, figure_h);
        motion.offset_x = offset(&self.sway, flip * figure_h);
        motion
    }

    /// For each part, the local angle that turns it from the way it's drawn to
    /// its neutral direction. A part with no neutral direction keeps its drawn
    /// angle to its parent.
    fn corrections(&self, skel: &Skeleton, facing_left: bool) -> Vec<f32> {
        let mut world = vec![0.0f32; skel.parts.len()];
        let mut local = vec![0.0f32; skel.parts.len()];
        for p in skel.draw_order() {
            let parent = skel.parts[p].parent.map_or(0.0, |q| world[q]);
            world[p] = match self.rest.get(&skel.parts[p].name) {
                Some(&deg) => {
                    let neutral = if facing_left { PI - deg.to_radians() } else { deg.to_radians() };
                    let (pivot, tip) = skel.bone(p);
                    let drawn = (tip[1] - pivot[1]).atan2(tip[0] - pivot[0]);
                    wrap(neutral - drawn)
                }
                None => parent,
            };
            local[p] = wrap(world[p] - parent);
        }
        local
    }
}

/// Whether a side-view character faces left, judged by its feet (bones named
/// "… foot" pointing from ankle to toe). False when it has none, e.g. a front view.
pub fn faces_left(skel: &Skeleton) -> bool {
    let feet: Vec<f32> = (0..skel.parts.len())
        .filter(|&p| skel.parts[p].name.ends_with("foot"))
        .map(|p| {
            let (ankle, toe) = skel.bone(p);
            toe[0] - ankle[0]
        })
        .collect();
    !feet.is_empty() && feet.iter().sum::<f32>() < 0.0
}

/// `a` in (-π, π].
fn wrap(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w <= -PI { w + TAU } else { w }
}

/// Keys sorted by time, keeping the last of any that snapped onto the same frame.
fn dedup(mut keys: Vec<Key>) -> Vec<Key> {
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut out: Vec<Key> = Vec::with_capacity(keys.len());
    for k in keys {
        match out.last_mut() {
            Some(last) if (last.t - k.t).abs() < crate::motion::KEY_EPSILON => *last = k,
            _ => out.push(k),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;
    use crate::template::builtin;

    fn template(name: &str) -> crate::template::Template {
        builtin().into_iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn builtins_load_loop_and_match_their_template() {
        let all = builtin_cycles();
        assert_eq!(all.len(), BUILTIN.len());
        for c in &all {
            let skel = template(&c.template).fit(RectI::new(0, 0, 100, 200));
            assert!(c.matches(&skel), "{} matches {}", c.name, c.template);
            for (name, keys) in c.tracks.iter().chain([(&"bob".to_string(), &c.bob)]) {
                let (Some(first), Some(last)) = (keys.first(), keys.last()) else { continue };
                assert_eq!(first.0, 0.0, "{} / {name} starts at 0", c.name);
                assert!((last.0 - c.duration).abs() < 1e-3, "{} / {name} ends at the end", c.name);
                assert!((first.1 - last.1).abs() < 1e-3, "{} / {name} loops", c.name);
            }
        }
    }

    #[test]
    fn side_cycles_need_a_side_skeleton() {
        let front = template("Person (front)").fit(RectI::new(0, 0, 100, 200));
        let walk = builtin_cycles().into_iter().find(|c| c.name == "Walk").unwrap();
        assert!(!walk.matches(&front));
    }

    #[test]
    fn keys_fit_the_skeleton_limits_and_frames() {
        for c in builtin_cycles() {
            let skel = template(&c.template).fit(RectI::new(0, 0, 100, 200));
            let m = c.to_motion(&skel, 200.0, 24, false);
            assert_eq!(m.duration, c.duration);
            for (i, part) in skel.parts.iter().enumerate() {
                for k in m.tracks.get(&part.name).into_iter().flatten() {
                    assert_eq!(clamp_to_limit(&skel, i, k.value), k.value, "{} / {} within limit", c.name, part.name);
                    assert!((k.t * 24.0 - (k.t * 24.0).round()).abs() < 1e-4, "on a frame");
                }
            }
        }
    }

    /// A one-bone arm hanging straight down: its neutral direction.
    fn arm(tip: [f32; 2]) -> Skeleton {
        let t = r#"{"name": "arm", "joints": [{"name": "s", "at": [0, 0]}, {"name": "h", "at": [0, 1]}],
                    "bones": [{"name": "arm", "from": "s", "to": "h"}]}"#;
        let mut skel = crate::template::Template::from_json(t).unwrap().fit(RectI::new(0, 0, 100, 100));
        skel.joints[1].pos = tip;
        skel
    }

    fn arm_cycle(rest: f32, keys: &str) -> Cycle {
        Cycle::from_json(&format!(
            r#"{{"name": "c", "duration": 1, "rest": {{"arm": {rest}}}, "tracks": {{"arm": {keys}}}, "sway": [[0, 0.1]]}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_drawing_off_neutral_is_brought_back() {
        let c = arm_cycle(90.0, "[[0, 0], [1, 30]]");
        // Drawn hanging down: keys are used as they are.
        let m = c.to_motion(&arm([0.0, 100.0]), 100.0, 24, false);
        assert!(m.tracks["arm"][0].value.abs() < 1e-5);
        assert!((m.tracks["arm"][1].value - 30f32.to_radians()).abs() < 1e-5);
        // Drawn pointing right (held out): turned 90° clockwise back down first.
        let m = c.to_motion(&arm([100.0, 0.0]), 100.0, 24, false);
        assert!((m.tracks["arm"][0].value - 90f32.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn facing_left_mirrors_angles_and_sway() {
        let c = arm_cycle(45.0, "[[0, 20]]");
        // Neutral down-right; mirrored, down-left. Drawn down-left, so no correction.
        let m = c.to_motion(&arm([-100.0, 100.0]), 100.0, 24, true);
        assert!((m.tracks["arm"][0].value + 20f32.to_radians()).abs() < 1e-5);
        assert!((m.offset_x[0].value + 10.0).abs() < 1e-4);
    }

    #[test]
    fn facing_is_read_from_the_feet() {
        let bbox = RectI::new(0, 0, 100, 200);
        let side = template("Person (side)");
        assert!(!faces_left(&side.fit(bbox)));
        assert!(faces_left(&side.mirrored().fit(bbox)));
        assert!(!faces_left(&template("Person (front)").fit(bbox)));
    }

    #[test]
    fn broken_cycles_are_refused_with_a_reason() {
        assert!(Cycle::from_json(r#"{"name": "x", "duration": 0, "tracks": {}}"#).unwrap_err().contains("length"));
        let late = r#"{"name": "x", "duration": 1, "tracks": {"arm": [[2, 0]]}}"#;
        assert!(Cycle::from_json(late).unwrap_err().contains("arm"));
        let eased = r#"{"name": "x", "duration": 1, "tracks": {"arm": [[0, 0, "Hold"], [1, 5]]}}"#;
        assert_eq!(Cycle::from_json(eased).unwrap().tracks["arm"][0].2, Ease::Hold);
    }
}
