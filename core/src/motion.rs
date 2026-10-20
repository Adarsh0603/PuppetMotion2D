//! Motion: how the pose changes over time.
//!
//! A motion is a set of tracks, one per part, each a list of keys (a time and
//! an angle). Between keys the angle is blended, so you only set the poses
//! that matter and the in-betweens are filled in. Tracks are looked up by part
//! *name*, not number, so a motion survives adding or removing bones and can
//! later be applied to another character built from the same template.

use crate::rig::Pose;
use crate::skeleton::Skeleton;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Keys closer together than this (seconds) count as the same moment.
pub const KEY_EPSILON: f32 = 1.0 / 120.0;

/// Frame rate of a new motion, and of projects saved before it was stored.
pub const DEFAULT_FPS: u32 = 24;

/// Highest frame rate allowed: frames stay further apart than `KEY_EPSILON`.
pub const MAX_FPS: u32 = 60;

/// How a value travels from one key to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Ease {
    /// Constant speed.
    Linear,
    /// Starts slow, speeds up, ends slow (ease in/out).
    #[default]
    Smooth,
    /// Holds this key's value until the next key, then jumps.
    Hold,
}

impl Ease {
    /// Map progress `u` in [0, 1] between two keys to blend amount.
    fn apply(self, u: f32) -> f32 {
        match self {
            Ease::Linear => u,
            Ease::Smooth => u * u * (3.0 - 2.0 * u),
            Ease::Hold => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Time in seconds.
    pub t: f32,
    pub value: f32,
    /// How to move from this key to the next one.
    #[serde(default)]
    pub ease: Ease,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Motion {
    /// Length in seconds (what playback loops over).
    pub duration: f32,
    /// Frames per second: the grid the timeline snaps to, and the export's
    /// default. Keys are still stored in seconds, so changing it moves nothing.
    pub fps: u32,
    /// Part name → angle keys (radians, as in `Pose`), sorted by time.
    pub tracks: BTreeMap<String, Vec<Key>>,
    /// Whole-figure movement keys (pixels).
    pub offset_x: Vec<Key>,
    pub offset_y: Vec<Key>,
}

impl Default for Motion {
    fn default() -> Self {
        Motion { duration: 3.0, fps: DEFAULT_FPS, tracks: BTreeMap::new(), offset_x: Vec::new(), offset_y: Vec::new() }
    }
}

impl Motion {
    /// Length of one frame in seconds.
    pub fn frame_len(&self) -> f32 {
        1.0 / self.fps.max(1) as f32
    }

    /// Number of the frame nearest to time `t` (frame 0 starts at 0 s).
    pub fn frame_at(&self, t: f32) -> i64 {
        (t * self.fps.max(1) as f32).round() as i64
    }

    /// `t` moved to the nearest whole frame.
    pub fn snap(&self, t: f32) -> f32 {
        self.frame_at(t) as f32 / self.fps.max(1) as f32
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.values().all(Vec::is_empty) && self.offset_x.is_empty() && self.offset_y.is_empty()
    }

    /// The pose at time `t` for `skel`. Parts without a track stay as drawn.
    pub fn sample(&self, t: f32, skel: &Skeleton) -> Pose {
        let mut pose = Pose::rest(skel.parts.len());
        for (i, part) in skel.parts.iter().enumerate() {
            if let Some(v) = self.tracks.get(&part.name).and_then(|k| sample_keys(k, t, true)) {
                pose.set_angle(i, v);
            }
        }
        pose.offset = [
            sample_keys(&self.offset_x, t, false).unwrap_or(0.0),
            sample_keys(&self.offset_y, t, false).unwrap_or(0.0),
        ];
        pose
    }

    /// Write keys at time `t` from `pose`. With `changed_from`, only parts
    /// (and the offset) whose values differ from that earlier pose get keys;
    /// with None, every part is keyed.
    pub fn set_keys(&mut self, t: f32, skel: &Skeleton, pose: &Pose, changed_from: Option<&Pose>) {
        for (i, part) in skel.parts.iter().enumerate() {
            let changed = changed_from.is_none_or(|prev| (prev.angle(i) - pose.angle(i)).abs() > 1e-5);
            if changed {
                set_key(self.tracks.entry(part.name.clone()).or_default(), t, pose.angle(i));
            }
        }
        let moved = changed_from.is_none_or(|prev| prev.offset != pose.offset);
        if moved {
            set_key(&mut self.offset_x, t, pose.offset[0]);
            set_key(&mut self.offset_y, t, pose.offset[1]);
        }
        self.duration = self.duration.max(t);
    }

    /// Every distinct key time, sorted.
    pub fn key_times(&self) -> Vec<f32> {
        let mut times: Vec<f32> = self.all_tracks().flat_map(|k| k.iter().map(|k| k.t)).collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < KEY_EPSILON);
        times
    }

    /// Delete every key at time `t`.
    pub fn remove_keys_at(&mut self, t: f32) {
        for keys in self.tracks.values_mut().chain([&mut self.offset_x, &mut self.offset_y]) {
            keys.retain(|k| (k.t - t).abs() >= KEY_EPSILON);
        }
        self.tracks.retain(|_, k| !k.is_empty());
    }

    /// Move a part's keys to its new name (when a bone is renamed).
    pub fn rename_track(&mut self, old: &str, new: &str) {
        if old != new
            && let Some(keys) = self.tracks.remove(old)
        {
            self.tracks.insert(new.to_string(), keys);
        }
    }

    fn all_tracks(&self) -> impl Iterator<Item = &Vec<Key>> {
        self.tracks.values().chain([&self.offset_x, &self.offset_y])
    }

    pub fn track(&self, id: &TrackId) -> Option<&Vec<Key>> {
        match id {
            TrackId::Part(name) => self.tracks.get(name),
            TrackId::OffsetX => Some(&self.offset_x),
            TrackId::OffsetY => Some(&self.offset_y),
        }
    }

    fn track_mut(&mut self, id: &TrackId) -> &mut Vec<Key> {
        match id {
            TrackId::Part(name) => self.tracks.entry(name.clone()).or_default(),
            TrackId::OffsetX => &mut self.offset_x,
            TrackId::OffsetY => &mut self.offset_y,
        }
    }

    /// Every key at time `t`, across all tracks.
    pub fn keys_at(&self, t: f32) -> Vec<KeyRef> {
        let mut out = Vec::new();
        for (name, keys) in &self.tracks {
            if keys.iter().any(|k| (k.t - t).abs() < KEY_EPSILON) {
                out.push(KeyRef { track: TrackId::Part(name.clone()), t });
            }
        }
        for id in [TrackId::OffsetX, TrackId::OffsetY] {
            if self.track(&id).is_some_and(|k| k.iter().any(|k| (k.t - t).abs() < KEY_EPSILON)) {
                out.push(KeyRef { track: id, t });
            }
        }
        out
    }

    pub fn has_key(&self, r: &KeyRef) -> bool {
        self.track(&r.track).is_some_and(|k| k.iter().any(|k| (k.t - r.t).abs() < KEY_EPSILON))
    }

    /// Copy the referenced keys, with times relative to the earliest one.
    pub fn copy(&self, sel: &[KeyRef]) -> Clip {
        let mut keys: Vec<(TrackId, Key)> = sel
            .iter()
            .filter_map(|r| {
                let k = self.track(&r.track)?.iter().find(|k| (k.t - r.t).abs() < KEY_EPSILON)?;
                Some((r.track.clone(), *k))
            })
            .collect();
        let start = keys.iter().map(|(_, k)| k.t).fold(f32::INFINITY, f32::min);
        for (_, k) in &mut keys {
            k.t -= start;
        }
        keys.dedup_by(|a, b| a.0 == b.0 && (a.1.t - b.1.t).abs() < KEY_EPSILON);
        Clip { keys }
    }

    /// Paste a clip so its first key lands at `at`. Keys already at those
    /// times are replaced. Returns references to the pasted keys.
    pub fn paste(&mut self, clip: &Clip, at: f32) -> Vec<KeyRef> {
        let mut pasted = Vec::new();
        for (id, k) in &clip.keys {
            let key = Key { t: (at + k.t).max(0.0), ..*k };
            insert_key(self.track_mut(id), key);
            self.duration = self.duration.max(key.t);
            pasted.push(KeyRef { track: id.clone(), t: key.t });
        }
        pasted
    }

    /// The key a reference points at.
    pub fn key(&self, r: &KeyRef) -> Option<&Key> {
        self.track(&r.track)?.iter().find(|k| (k.t - r.t).abs() < KEY_EPSILON)
    }

    /// Move the referenced keys `dt` seconds later (earlier if negative),
    /// never before 0. Keys they land on that aren't being moved are
    /// replaced. Returns references to the moved keys.
    pub fn move_keys(&mut self, sel: &[KeyRef], dt: f32) -> Vec<KeyRef> {
        let picked: Vec<(TrackId, Key)> = sel.iter().filter_map(|r| Some((r.track.clone(), *self.key(r)?))).collect();
        let earliest = picked.iter().map(|(_, k)| k.t).fold(f32::INFINITY, f32::min);
        let dt = if earliest.is_finite() { dt.max(-earliest) } else { dt };
        // Lift them all out first, so keys moving past each other don't collide.
        for (id, k) in &picked {
            self.track_mut(id).retain(|o| (o.t - k.t).abs() >= KEY_EPSILON);
        }
        let mut moved = Vec::with_capacity(picked.len());
        for (id, k) in picked {
            let key = Key { t: k.t + dt, ..k };
            insert_key(self.track_mut(&id), key);
            self.duration = self.duration.max(key.t);
            moved.push(KeyRef { track: id, t: key.t });
        }
        self.tracks.retain(|_, k| !k.is_empty());
        moved
    }

    /// Set how each referenced key travels to the next one.
    pub fn set_ease(&mut self, sel: &[KeyRef], ease: Ease) {
        for r in sel {
            if let Some(k) = self.track_mut(&r.track).iter_mut().find(|k| (k.t - r.t).abs() < KEY_EPSILON) {
                k.ease = ease;
            }
        }
        self.tracks.retain(|_, k| !k.is_empty());
    }

    /// Delete the referenced keys.
    pub fn remove(&mut self, sel: &[KeyRef]) {
        for r in sel {
            if let Some(keys) = match &r.track {
                TrackId::Part(name) => self.tracks.get_mut(name),
                TrackId::OffsetX => Some(&mut self.offset_x),
                TrackId::OffsetY => Some(&mut self.offset_y),
            } {
                keys.retain(|k| (k.t - r.t).abs() >= KEY_EPSILON);
            }
        }
        self.tracks.retain(|_, k| !k.is_empty());
    }
}

/// Which track a key lives on.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TrackId {
    /// A part's angle, by part name.
    Part(String),
    /// Whole-figure movement, horizontal and vertical.
    OffsetX,
    OffsetY,
}

/// Points at one key: its track and its time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyRef {
    pub track: TrackId,
    pub t: f32,
}

/// Copied keys, ready to paste. Times are relative to the earliest key.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Clip {
    pub keys: Vec<(TrackId, Key)>,
}

/// Marks clipboard text as copied keys (so pasting other text is ignored).
const CLIP_TAG: &str = "PuppetMotion2D-keys:";

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Text form for the system clipboard, so keys can be pasted into another project.
    pub fn to_clipboard_text(&self) -> String {
        format!("{CLIP_TAG}{}", serde_json::to_string(self).unwrap_or_default())
    }

    /// Read clipboard text written by `to_clipboard_text`; None for anything else.
    pub fn from_clipboard_text(text: &str) -> Option<Clip> {
        serde_json::from_str(text.trim().strip_prefix(CLIP_TAG)?).ok()
    }
}

/// Insert a key at `t`, or update the value of a key already there.
fn set_key(keys: &mut Vec<Key>, t: f32, value: f32) {
    if let Some(k) = keys.iter_mut().find(|k| (k.t - t).abs() < KEY_EPSILON) {
        k.value = value;
        return;
    }
    insert_key(keys, Key { t, value, ease: Ease::default() });
}

/// Insert `key` in time order, replacing any key at the same moment.
fn insert_key(keys: &mut Vec<Key>, key: Key) {
    keys.retain(|k| (k.t - key.t).abs() >= KEY_EPSILON);
    let at = keys.partition_point(|k| k.t < key.t);
    keys.insert(at, key);
}

/// Value of a sorted key list at time `t`; None if there are no keys. Before
/// the first key and after the last the value holds. `angle` blends the short
/// way round so 170° → -170° passes through 180°, not 0°.
fn sample_keys(keys: &[Key], t: f32, angle: bool) -> Option<f32> {
    let first = keys.first()?;
    let last = keys.last()?;
    if t <= first.t {
        return Some(first.value);
    }
    if t >= last.t {
        return Some(last.value);
    }
    let i = keys.partition_point(|k| k.t <= t) - 1;
    let (a, b) = (keys[i], keys[i + 1]);
    let u = a.ease.apply((t - a.t) / (b.t - a.t));
    let mut delta = b.value - a.value;
    if angle {
        use std::f32::consts::{PI, TAU};
        delta = (delta + PI).rem_euclid(TAU) - PI;
    }
    Some(a.value + delta * u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::RectI;

    fn key(t: f32, value: f32, ease: Ease) -> Key {
        Key { t, value, ease }
    }

    #[test]
    fn moving_keys_shifts_replaces_and_stops_at_zero() {
        let arm = TrackId::Part("arm".into());
        let mut m = Motion::default();
        m.tracks.insert("arm".into(), vec![key(0.5, 1.0, Ease::Smooth), key(1.0, 2.0, Ease::Hold), key(2.0, 3.0, Ease::Linear)]);
        let r = |t: f32| KeyRef { track: arm.clone(), t };

        // Two keys move together, keeping their gap and values.
        let moved = m.move_keys(&[r(0.5), r(1.0)], 0.5);
        assert_eq!(moved, vec![r(1.0), r(1.5)]);
        let arm_keys = &m.tracks["arm"];
        assert_eq!(arm_keys.iter().map(|k| (k.t, k.value)).collect::<Vec<_>>(), vec![(1.0, 1.0), (1.5, 2.0), (2.0, 3.0)]);
        assert_eq!(arm_keys[1].ease, Ease::Hold, "ease travels with the key");

        // Landing on an unselected key replaces it.
        m.move_keys(&[r(1.5)], 0.5);
        assert_eq!(m.tracks["arm"].iter().map(|k| (k.t, k.value)).collect::<Vec<_>>(), vec![(1.0, 1.0), (2.0, 2.0)]);

        // Can't go before 0; past the end grows the length.
        assert_eq!(m.move_keys(&[r(1.0)], -5.0), vec![r(0.0)]);
        m.move_keys(&[r(2.0)], 3.0);
        assert_eq!(m.duration, 5.0);

        m.set_ease(&[r(0.0)], Ease::Linear);
        assert_eq!(m.key(&r(0.0)).unwrap().ease, Ease::Linear);
    }

    #[test]
    fn frames_snap_and_count() {
        let m = Motion { fps: 24, ..Motion::default() };
        assert_eq!(m.frame_at(0.5), 12);
        assert_eq!(m.snap(0.51), 0.5);
        assert_eq!(m.snap(0.49), 0.5);
        assert!((m.frame_len() - 1.0 / 24.0).abs() < 1e-7);
        // Projects saved before fps existed open at the default.
        let old: Motion = serde_json::from_str(r#"{"duration": 2.0, "tracks": {}}"#).unwrap();
        assert_eq!(old.fps, DEFAULT_FPS);
    }

    #[test]
    fn blending_between_keys() {
        let lin = [key(0.0, 0.0, Ease::Linear), key(2.0, 1.0, Ease::Linear)];
        assert_eq!(sample_keys(&lin, 0.0, false), Some(0.0));
        assert_eq!(sample_keys(&lin, 1.0, false), Some(0.5));
        assert_eq!(sample_keys(&lin, 2.0, false), Some(1.0));
        assert_eq!(sample_keys(&lin, -1.0, false), Some(0.0), "holds before the first key");
        assert_eq!(sample_keys(&lin, 9.0, false), Some(1.0), "holds after the last key");

        let smooth = [key(0.0, 0.0, Ease::Smooth), key(2.0, 1.0, Ease::Smooth)];
        assert!((sample_keys(&smooth, 1.0, false).unwrap() - 0.5).abs() < 1e-6, "midpoint at the middle");
        assert!(sample_keys(&smooth, 0.2, false).unwrap() < 0.1, "slow start");

        let hold = [key(0.0, 0.0, Ease::Hold), key(2.0, 1.0, Ease::Hold)];
        assert_eq!(sample_keys(&hold, 1.9, false), Some(0.0));
        assert!(sample_keys(&[], 1.0, false).is_none());
    }

    #[test]
    fn angles_take_the_short_way() {
        let a = 170f32.to_radians();
        let b = -170f32.to_radians();
        let keys = [key(0.0, a, Ease::Linear), key(1.0, b, Ease::Linear)];
        let mid = sample_keys(&keys, 0.5, true).unwrap();
        assert!((mid.abs() - std::f32::consts::PI).abs() < 1e-4, "passes through 180°, got {}", mid.to_degrees());
    }

    #[test]
    fn keys_follow_part_names() {
        let mut s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let mut m = Motion::default();
        let arm = s.parts.iter().position(|p| p.name == "left forearm").unwrap();
        let mut pose = Pose::rest(s.parts.len());
        pose.set_angle(arm, 1.0);
        m.set_keys(1.0, &s, &pose, Some(&Pose::rest(s.parts.len())));
        assert_eq!(m.tracks.len(), 1, "only the changed part is keyed");
        assert!(m.offset_x.is_empty());

        // Remove the head (part 0): indices shift, the forearm keeps its motion.
        s.remove_part(0);
        let arm = s.parts.iter().position(|p| p.name == "left forearm").unwrap();
        assert_eq!(m.sample(1.0, &s).angle(arm), 1.0);

        m.rename_track("left forearm", "left lower arm");
        s.parts[arm].name = "left lower arm".into();
        assert_eq!(m.sample(1.0, &s).angle(arm), 1.0);
    }

    #[test]
    fn copy_paste_and_delete_selected_keys() {
        let s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let mut m = Motion::default();
        let mut pose = Pose::rest(s.parts.len());
        pose.set_angle(3, 0.4);
        m.set_keys(1.0, &s, &pose, Some(&Pose::rest(s.parts.len()))); // forearm at 1.0
        pose.set_angle(3, -0.4);
        m.set_keys(1.5, &s, &pose, None); // every part at 1.5

        // Copy the forearm's two keys and paste them at 2.0: same spacing.
        let arm = TrackId::Part(s.parts[3].name.clone());
        let sel = [KeyRef { track: arm.clone(), t: 1.0 }, KeyRef { track: arm.clone(), t: 1.5 }];
        let clip = m.copy(&sel);
        assert_eq!(clip.keys.iter().map(|(_, k)| k.t).collect::<Vec<_>>(), vec![0.0, 0.5]);
        let pasted = m.paste(&clip, 2.0);
        assert!(pasted.iter().all(|r| m.has_key(r)));
        let a = m.sample(2.0, &s).angle(3);
        assert!((a - 0.4).abs() < 1e-6 && (m.sample(2.5, &s).angle(3) + 0.4).abs() < 1e-6);
        assert_eq!(m.duration, 3.0, "unchanged: 2.5 is within the length");

        // Clipboard text round-trips; other text is ignored.
        assert_eq!(Clip::from_clipboard_text(&clip.to_clipboard_text()), Some(clip));
        assert_eq!(Clip::from_clipboard_text("hello"), None);

        // Select everything at 1.5 and delete it.
        let at = m.keys_at(1.5);
        assert!(at.len() > s.parts.len(), "all parts plus the position");
        m.remove(&at);
        assert_eq!(m.key_times(), vec![1.0, 2.0, 2.5]);
    }

    #[test]
    fn set_replace_and_remove_keys() {
        let s = Skeleton::person_front(RectI::new(0, 0, 100, 200));
        let mut m = Motion::default();
        let mut pose = Pose::rest(s.parts.len());
        m.set_keys(0.0, &s, &pose, None);
        assert_eq!(m.tracks.len(), s.parts.len(), "key all");
        pose.set_angle(2, 0.5);
        pose.offset = [3.0, 0.0];
        m.set_keys(1.0, &s, &pose, None);
        m.set_keys(1.0 + KEY_EPSILON / 2.0, &s, &pose, None); // same moment: replaces
        assert_eq!(m.key_times(), vec![0.0, 1.0]);
        assert_eq!(m.sample(1.0, &s).offset, [3.0, 0.0]);
        m.set_keys(4.0, &s, &pose, None);
        assert_eq!(m.duration, 4.0, "length grows to fit keys");
        m.remove_keys_at(1.0);
        assert_eq!(m.key_times(), vec![0.0, 4.0]);
        assert!(!m.is_empty());
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<Motion>(&json).unwrap(), m);
    }
}
