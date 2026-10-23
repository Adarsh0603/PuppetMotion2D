//! Motions for any skeleton: made from its structure (which bone hangs from
//! which), not from bone names, so monsters, tails and blobs get them too.
//!
//! Every motion is small turns from the pose as drawn, so it looks right
//! whichever way the drawing faces, and it loops: the last keys equal the first.

use crate::motion::{Ease, Key, Motion};
use crate::rig::clamp_to_limit;
use crate::skeleton::Skeleton;
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnyRig {
    /// A slow rise and fall, with the limbs easing out a little.
    Breathe,
    /// The body rocks side to side; what hangs from it rocks back a little.
    Sway,
    /// A wave travelling down every chain, growing toward the tips (tails, tentacles).
    Wiggle,
    /// A quick tremble.
    Shake,
    /// A small jump in place.
    Hop,
}

impl AnyRig {
    pub const ALL: [AnyRig; 5] = [AnyRig::Breathe, AnyRig::Sway, AnyRig::Wiggle, AnyRig::Shake, AnyRig::Hop];

    pub fn name(self) -> &'static str {
        match self {
            AnyRig::Breathe => "Breathe",
            AnyRig::Sway => "Sway",
            AnyRig::Wiggle => "Wiggle",
            AnyRig::Shake => "Shake",
            AnyRig::Hop => "Hop",
        }
    }

    /// The motion for `skel`. `figure_h` is the character's height in pixels
    /// (for the whole-figure moves); keys land on whole frames at `fps`.
    pub fn motion(self, skel: &Skeleton, figure_h: f32, fps: u32) -> Motion {
        let duration = match self {
            AnyRig::Breathe | AnyRig::Sway => 2.0,
            AnyRig::Wiggle => 1.5,
            AnyRig::Shake => 0.5,
            AnyRig::Hop => 0.8,
        };
        let mut m = Motion { duration, fps, ..Motion::default() };
        let t = duration;
        // Each sibling turns the other way from the one before, so a pair
        // (both arms, both legs) moves in mirror rather than in step.
        let mut sibling = vec![0usize; skel.parts.len()];
        for (i, part) in skel.parts.iter().enumerate() {
            sibling[i] = skel.parts[..i].iter().filter(|p| p.parent == part.parent).count();
        }
        let side = |i: usize| if sibling[i].is_multiple_of(2) { 1.0 } else { -1.0 };
        for (i, depth) in skel.tree_order() {
            let keys = match (self, depth) {
                (AnyRig::Breathe, 1) => wave(t, side(i) * 2.0, 0.5),
                (AnyRig::Sway, 0) => wave(t, 4.0, 0.0),
                (AnyRig::Sway, 1) => wave(t, -2.0, 0.0),
                (AnyRig::Wiggle, d) => wave(t, (5.0 + 4.0 * d as f32).min(20.0), 0.15 * d as f32),
                (AnyRig::Shake, 0) => shake(t, 3.0),
                (AnyRig::Hop, 1) => hop(t, side(i) * 6.0),
                _ => continue,
            };
            let keys = keys.into_iter().map(|k| Key { value: clamp_to_limit(skel, i, k.value.to_radians()), ..k });
            m.tracks.insert(skel.parts[i].name.clone(), keys.collect());
        }
        // Whole-figure moves, as shares of the height (down is positive).
        let h = figure_h.max(1.0);
        match self {
            AnyRig::Breathe => m.offset_y = wave(t, 0.008 * h, 0.5),
            AnyRig::Shake => m.offset_x = shake(t, 0.015 * h),
            AnyRig::Hop => m.offset_y = hop(t, -0.18 * h),
            _ => {}
        }
        snap(&mut m);
        m
    }
}

/// One cycle of `amp · cos(2π (t/T − delay))` over `t`: the peak comes
/// `delay` cycles in. Peaks half a cycle apart need only three Smooth keys
/// (smoothstep from peak to peak is very close to a cosine); other delays
/// take Linear samples, which loop without a pause at the seam.
fn wave(t: f32, amp: f32, delay: f32) -> Vec<Key> {
    let half_steps = (delay * 2.0).round();
    if (delay * 2.0 - half_steps).abs() < 1e-4 {
        let a = if half_steps as i32 % 2 == 0 { amp } else { -amp };
        return vec![key(0.0, a, Ease::Smooth), key(t / 2.0, -a, Ease::Smooth), key(t, a, Ease::Smooth)];
    }
    (0..=WAVE_SAMPLES)
        .map(|k| {
            let u = k as f32 / WAVE_SAMPLES as f32;
            key(u * t, amp * (TAU * (u - delay)).cos(), Ease::Linear)
        })
        .collect()
}

/// Samples per cycle for waves whose peaks don't land on the seam.
const WAVE_SAMPLES: u32 = 12;

/// Back and forth twice, quickly, settling at rest.
fn shake(t: f32, amp: f32) -> Vec<Key> {
    [0.0, amp, -amp, amp, -amp, 0.0].iter().enumerate().map(|(k, &v)| key(t * k as f32 / 5.0, v, Ease::Linear)).collect()
}

/// Crouch, spring up to `amp`, come down and settle.
fn hop(t: f32, amp: f32) -> Vec<Key> {
    vec![
        key(0.0, 0.0, Ease::Smooth),
        key(0.15 * t, -0.15 * amp, Ease::Smooth),
        key(0.5 * t, amp, Ease::Smooth),
        key(0.85 * t, -0.1 * amp, Ease::Smooth),
        key(t, 0.0, Ease::Smooth),
    ]
}

fn key(t: f32, value: f32, ease: Ease) -> Key {
    Key { t, value, ease }
}

/// Move every key to the nearest frame, keeping the last of any that land together.
fn snap(m: &mut Motion) {
    let grid = m.clone();
    let fix = |keys: &mut Vec<Key>| {
        for k in keys.iter_mut() {
            k.t = grid.snap(k.t);
        }
        keys.dedup_by(|later, kept| {
            let same = (later.t - kept.t).abs() < crate::motion::KEY_EPSILON;
            if same {
                *kept = *later;
            }
            same
        });
    };
    m.tracks.values_mut().for_each(fix);
    fix(&mut m.offset_x);
    fix(&mut m.offset_y);
    m.duration = grid.snap(m.duration);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;

    /// A straight chain of `n` bones hanging down from the first.
    fn chain(n: usize) -> Skeleton {
        let mut s = Skeleton::empty();
        let mut prev = s.add_joint([50.0, 0.0]);
        for k in 1..=n {
            let j = s.add_joint([50.0, 20.0 * k as f32]);
            s.add_part(prev, j);
            prev = j;
        }
        s
    }

    #[test]
    fn every_motion_fits_any_skeleton_and_loops() {
        for skel in [Skeleton::person_front(RectI::new(0, 0, 100, 200)), chain(1), chain(3)] {
            for kind in AnyRig::ALL {
                let m = kind.motion(&skel, 200.0, 24);
                assert!(!m.is_empty(), "{} moves something on {} bones", kind.name(), skel.parts.len());
                let (a, b) = (m.sample(0.0, &skel), m.sample(m.duration, &skel));
                for i in 0..skel.parts.len() {
                    assert!((a.angle(i) - b.angle(i)).abs() < 1e-4, "{} loops", kind.name());
                    assert_eq!(clamp_to_limit(&skel, i, a.angle(i)), a.angle(i), "{} keeps to limits", kind.name());
                }
                assert!((a.offset[0] - b.offset[0]).abs() < 1e-4 && (a.offset[1] - b.offset[1]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn wiggle_grows_and_lags_down_a_chain() {
        let skel = chain(3);
        let m = AnyRig::Wiggle.motion(&skel, 200.0, 24);
        let peak = |name: &str| m.tracks[name].iter().map(|k| k.value.abs()).fold(0.0, f32::max);
        // The earliest key at the track's highest value (the loop repeats it at the end).
        let first_peak = |name: &str| {
            let top = m.tracks[name].iter().map(|k| k.value).fold(f32::MIN, f32::max);
            m.tracks[name].iter().find(|k| k.value == top).unwrap().t
        };
        let names: Vec<String> = skel.parts.iter().map(|p| p.name.clone()).collect();
        assert!(peak(&names[0]) < peak(&names[1]) && peak(&names[1]) < peak(&names[2]));
        assert!(first_peak(&names[0]) < first_peak(&names[1]) && first_peak(&names[1]) < first_peak(&names[2]));
    }

    #[test]
    fn waves_on_the_seam_need_three_keys() {
        assert_eq!(wave(2.0, 4.0, 0.0).len(), 3);
        assert_eq!(wave(2.0, 4.0, 0.5)[0].value, -4.0);
        assert_eq!(wave(2.0, 4.0, 0.15).len(), WAVE_SAMPLES as usize + 1);
    }
}
