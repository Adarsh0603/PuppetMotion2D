//! Skeleton templates: a ready-made set of joints and bones saved as JSON.
//!
//! Joint positions are fractions of the character's bounding box (0,0 its
//! top-left, 1,1 its bottom-right), so one template fits a drawing of any
//! size. Bones name their joints and parent bone instead of using numbers,
//! so the files are easy to read and edit by hand.

use crate::geom::RectI;
use crate::skeleton::{Joint, SkelPart, Skeleton};
use serde::{Deserialize, Serialize};

/// The templates that come with the app, in menu order.
const BUILTIN: [&str; 2] = [include_str!("../templates/person_front.json"), include_str!("../templates/person_side.json")];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    pub joints: Vec<TemplateJoint>,
    pub bones: Vec<TemplateBone>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateJoint {
    pub name: String,
    /// Position as fractions of the character's bounding box.
    pub at: [f32; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateBone {
    pub name: String,
    /// The joint it turns around (where it attaches to its parent).
    pub from: String,
    /// Its free end.
    pub to: String,
    /// The bone it hangs from; none for the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Allowed rotation `[min, max]` in degrees from the way it's drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<[f32; 2]>,
    /// Place in the painting order, lowest at the back.
    #[serde(default)]
    pub layer: i32,
}

/// The templates that come with the app.
pub fn builtin() -> Vec<Template> {
    BUILTIN.iter().map(|json| Template::from_json(json).expect("built-in template is valid")).collect()
}

impl Template {
    /// Read a template, checking that every joint and parent a bone names exists.
    pub fn from_json(text: &str) -> Result<Template, String> {
        let t: Template = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if t.name.trim().is_empty() {
            return Err("the template has no name".into());
        }
        let joint = |n: &str| t.joints.iter().any(|j| j.name == n);
        for b in &t.bones {
            for end in [&b.from, &b.to] {
                if !joint(end) {
                    return Err(format!("bone \"{}\" uses a joint \"{end}\" that isn't in the template", b.name));
                }
            }
            if let Some(p) = &b.parent
                && (p == &b.name || !t.bones.iter().any(|o| &o.name == p))
            {
                return Err(format!("bone \"{}\" hangs from \"{p}\", which isn't another bone in the template", b.name));
            }
        }
        Ok(t)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("templates always serialize")
    }

    /// The skeleton, scaled into `bbox` (usually the character's bounding box).
    pub fn fit(&self, bbox: RectI) -> Skeleton {
        let at = |[u, v]: [f32; 2]| [bbox.x as f32 + u * bbox.w as f32, bbox.y as f32 + v * bbox.h as f32];
        let joints: Vec<Joint> = self.joints.iter().map(|j| Joint { name: j.name.clone(), pos: at(j.at) }).collect();
        let joint = |n: &str| self.joints.iter().position(|j| j.name == n);
        let bone = |n: &str| self.bones.iter().position(|b| b.name == n);
        let parts = self
            .bones
            .iter()
            .filter_map(|b| {
                Some(SkelPart {
                    name: b.name.clone(),
                    pivot: joint(&b.from)?,
                    tip: joint(&b.to)?,
                    parent: b.parent.as_deref().and_then(bone),
                    limit: b.limit.map(|[lo, hi]| [lo.to_radians(), hi.to_radians()]),
                    layer: b.layer,
                })
            })
            .collect();
        Skeleton { joints, parts }
    }

    /// A template from a skeleton placed on a character whose bounding box is `bbox`.
    pub fn from_skeleton(name: &str, skel: &Skeleton, bbox: RectI) -> Template {
        let round = |v: f32, places: f32| (v * places).round() / places;
        let (w, h) = (bbox.w.max(1) as f32, bbox.h.max(1) as f32);
        let joints = skel
            .joints
            .iter()
            .map(|j| TemplateJoint {
                name: j.name.clone(),
                at: [round((j.pos[0] - bbox.x as f32) / w, 1e4), round((j.pos[1] - bbox.y as f32) / h, 1e4)],
            })
            .collect();
        let bones = skel
            .parts
            .iter()
            .map(|p| TemplateBone {
                name: p.name.clone(),
                from: skel.joints[p.pivot].name.clone(),
                to: skel.joints[p.tip].name.clone(),
                parent: p.parent.map(|q| skel.parts[q].name.clone()),
                limit: p.limit.map(|[lo, hi]| [round(lo.to_degrees(), 10.0), round(hi.to_degrees(), 10.0)]),
                layer: p.layer,
            })
            .collect();
        Template { name: name.trim().to_string(), joints, bones }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_load_and_fit() {
        let all = builtin();
        assert_eq!(all.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Person (front)", "Person (side)"]);
        let bbox = RectI::new(10, 20, 100, 200);
        for t in &all {
            let s = t.fit(bbox);
            assert_eq!(s.parts.len(), t.bones.len(), "{}: every bone fits", t.name);
            assert_eq!(s.parts.iter().filter(|p| p.parent.is_none()).count(), 1, "{}: one root", t.name);
            for j in &s.joints {
                assert!(j.pos[0] >= 10.0 && j.pos[0] <= 110.0 && j.pos[1] >= 20.0 && j.pos[1] <= 220.0);
            }
        }
    }

    #[test]
    fn side_view_has_far_limbs_behind_the_body() {
        let s = builtin()[1].fit(RectI::new(0, 0, 100, 200));
        let order = s.paint_order();
        let at = |n: &str| order.iter().position(|&p| s.parts[p].name == n).unwrap();
        for far in ["far upper arm", "far forearm", "far thigh", "far shin", "far foot"] {
            assert!(at(far) < at("torso"), "{far} is behind the torso");
        }
        for near in ["near upper arm", "near forearm", "near thigh", "near shin", "near foot"] {
            assert!(at(near) > at("torso"), "{near} is in front of the torso");
        }
    }

    #[test]
    fn a_saved_skeleton_comes_back_the_same() {
        let bbox = RectI::new(40, 10, 80, 160);
        let mut skel = builtin()[1].fit(bbox);
        skel.parts[2].layer = 7;
        let saved = Template::from_skeleton("  My monster ", &skel, bbox);
        assert_eq!(saved.name, "My monster");
        let back = Template::from_json(&saved.to_json()).unwrap().fit(bbox);
        assert_eq!(back.parts, skel.parts);
        for (a, b) in back.joints.iter().zip(&skel.joints) {
            assert!((a.pos[0] - b.pos[0]).abs() < 0.05 && (a.pos[1] - b.pos[1]).abs() < 0.05);
        }
        // Fits a drawing of another size by the same fractions.
        let big = saved.fit(RectI::new(0, 0, 800, 1600));
        let [u, v] = saved.joints[0].at;
        assert_eq!(big.joints[0].pos, [u * 800.0, v * 1600.0]);
    }

    #[test]
    fn broken_templates_are_refused_with_a_reason() {
        let bad = r#"{"name": "x", "joints": [{"name": "a", "at": [0, 0]}],
                      "bones": [{"name": "b", "from": "a", "to": "missing"}]}"#;
        assert!(Template::from_json(bad).unwrap_err().contains("missing"));
        assert!(Template::from_json("not json").is_err());
    }
}
