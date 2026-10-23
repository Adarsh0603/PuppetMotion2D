//! Draws the sample characters (flat cartoons on transparent backgrounds) and
//! the templates that fit them. Run from the repository root:
//!
//!     cargo run --release -p anim-core --example make_samples
//!
//! It writes `app/assets/samples/*.png` and `core/templates/{four_legs_side,
//! tentacles,blob}.json`. Shapes are distance functions, so outlines stay smooth.
use image::{Rgba, RgbaImage};

type Sdf = Box<dyn Fn([f32; 2]) -> f32>;

fn len(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}
fn circle(c: [f32; 2], r: f32) -> Sdf {
    Box::new(move |p| len([p[0] - c[0], p[1] - c[1]]) - r)
}
/// Rotated ellipse (approximate distance).
fn ellipse(c: [f32; 2], rx: f32, ry: f32, deg: f32) -> Sdf {
    let (s, co) = deg.to_radians().sin_cos();
    Box::new(move |p| {
        let d = [p[0] - c[0], p[1] - c[1]];
        let q = [d[0] * co + d[1] * s, -d[0] * s + d[1] * co];
        let k = len([q[0] / rx, q[1] / ry]);
        (k - 1.0) * rx.min(ry)
    })
}
/// A limb from `a` (radius `ra`) to `b` (radius `rb`).
fn limb(a: [f32; 2], b: [f32; 2], ra: f32, rb: f32) -> Sdf {
    Box::new(move |p| {
        let pa = [p[0] - a[0], p[1] - a[1]];
        let ba = [b[0] - a[0], b[1] - a[1]];
        let h = ((pa[0] * ba[0] + pa[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1])).clamp(0.0, 1.0);
        len([pa[0] - ba[0] * h, pa[1] - ba[1] * h]) - (ra + (rb - ra) * h)
    })
}
/// A chain of limbs through `pts`, tapering from `r0` to `r1`.
fn chain(pts: Vec<[f32; 2]>, r0: f32, r1: f32) -> Sdf {
    let n = pts.len() - 1;
    let parts: Vec<Sdf> = (0..n)
        .map(|i| {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            limb(pts[i], pts[i + 1], r0 + (r1 - r0) * t0, r0 + (r1 - r0) * t1)
        })
        .collect();
    union(parts)
}
fn rbox(c: [f32; 2], hw: f32, hh: f32, r: f32, deg: f32) -> Sdf {
    let (s, co) = deg.to_radians().sin_cos();
    Box::new(move |p| {
        let d = [p[0] - c[0], p[1] - c[1]];
        let q = [(d[0] * co + d[1] * s).abs() - hw + r, (-d[0] * s + d[1] * co).abs() - hh + r];
        len([q[0].max(0.0), q[1].max(0.0)]) + q[0].max(q[1]).min(0.0) - r
    })
}
fn tri(a: [f32; 2], b: [f32; 2], c: [f32; 2], round: f32) -> Sdf {
    Box::new(move |p| {
        let pts = [a, b, c];
        let mut d = f32::MAX;
        let mut inside = true;
        let mut sign = 0.0;
        for i in 0..3 {
            let (e0, e1) = (pts[i], pts[(i + 1) % 3]);
            let e = [e1[0] - e0[0], e1[1] - e0[1]];
            let w = [p[0] - e0[0], p[1] - e0[1]];
            let h = ((w[0] * e[0] + w[1] * e[1]) / (e[0] * e[0] + e[1] * e[1])).clamp(0.0, 1.0);
            d = d.min(len([w[0] - e[0] * h, w[1] - e[1] * h]));
            let cross = e[0] * w[1] - e[1] * w[0];
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                inside = false;
            }
        }
        (if inside { -d } else { d }) - round
    })
}
fn union(parts: Vec<Sdf>) -> Sdf {
    Box::new(move |p| parts.iter().map(|f| f(p)).fold(f32::MAX, f32::min))
}
fn smooth_union(a: Sdf, b: Sdf, k: f32) -> Sdf {
    Box::new(move |p| {
        let (x, y) = (a(p), b(p));
        let h = (0.5 + 0.5 * (y - x) / k).clamp(0.0, 1.0);
        y + (x - y) * h - k * h * (1.0 - h)
    })
}
fn intersect(a: Sdf, b: Sdf) -> Sdf {
    Box::new(move |p| a(p).max(b(p)))
}

struct Layer {
    sdf: Sdf,
    fill: [u8; 3],
    outline: f32,
}
fn layer(sdf: Sdf, fill: u32, outline: f32) -> Layer {
    Layer { sdf, fill: [(fill >> 16) as u8, (fill >> 8) as u8, fill as u8], outline }
}

const INK: [f32; 3] = [0x2b as f32, 0x22 as f32, 0x30 as f32];

fn render(size: [u32; 2], layers: &[Layer]) -> RgbaImage {
    let mut img = RgbaImage::new(size[0], size[1]);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let p = [x as f32 + 0.5, y as f32 + 0.5];
        let (mut c, mut a) = ([0.0f32; 3], 0.0f32);
        let mut over = |col: [f32; 3], cov: f32| {
            if cov <= 0.0 {
                return;
            }
            let na = cov + a * (1.0 - cov);
            for i in 0..3 {
                c[i] = (col[i] * cov + c[i] * a * (1.0 - cov)) / na;
            }
            a = na;
        };
        for l in layers {
            let d = (l.sdf)(p);
            if l.outline > 0.0 {
                over(INK, (0.5 - (d - l.outline)).clamp(0.0, 1.0));
            }
            over(l.fill.map(|v| v as f32), (0.5 - d).clamp(0.0, 1.0));
        }
        *px = Rgba([c[0] as u8, c[1] as u8, c[2] as u8, (a * 255.0).round() as u8]);
    }
    img
}

fn eye(c: [f32; 2], r: f32, look: [f32; 2]) -> Vec<Layer> {
    vec![
        layer(circle(c, r), 0xffffff, 3.0),
        layer(circle([c[0] + look[0] * r * 0.35, c[1] + look[1] * r * 0.35], r * 0.55), 0x2b2230, 0.0),
        layer(circle([c[0] + look[0] * r * 0.35 - r * 0.2, c[1] + look[1] * r * 0.35 - r * 0.2], r * 0.18), 0xffffff, 0.0),
    ]
}

/// A front-facing adventurer in an A-pose: no limb crosses the body.
fn hero() -> Vec<Layer> {
    let o = 5.0;
    let skin = 0xf2c49b;
    let mut v = Vec::new();
    for side in [-1.0f32, 1.0] {
        let x = |d: f32| 300.0 + side * d;
        v.push(layer(union(vec![limb([x(22.0), 372.0], [x(38.0), 450.0], 24.0, 20.0), limb([x(38.0), 450.0], [x(42.0), 525.0], 20.0, 17.0)]), 0x5c4836, o));
        v.push(layer(rbox([x(50.0), 538.0], 30.0, 14.0, 10.0, 0.0), 0x3a2f27, o));
        v.push(layer(union(vec![limb([x(48.0), 238.0], [x(88.0), 305.0], 18.0, 16.0), limb([x(88.0), 305.0], [x(112.0), 365.0], 16.0, 14.0)]), 0x4a8fd6, o));
        v.push(layer(circle([x(116.0), 377.0], 16.0), skin, o));
    }
    v.extend([
        layer(rbox([300.0, 300.0], 58.0, 88.0, 26.0, 0.0), 0x3f7cc0, o),
        layer(rbox([300.0, 355.0], 60.0, 10.0, 4.0, 0.0), 0x5a3d26, 3.0),
        layer(rbox([300.0, 355.0], 11.0, 9.0, 3.0, 0.0), 0xe7c35a, 2.5),
        layer(rbox([300.0, 200.0], 14.0, 16.0, 6.0, 0.0), skin, o),
        layer(circle([300.0, 140.0], 62.0), skin, o),
        layer(intersect(circle([300.0, 135.0], 68.0), circle([300.0, 40.0], 105.0)), 0x6b3b22, o),
        layer(ellipse([300.0, 160.0], 9.0, 8.0, 0.0), 0xe8a982, 0.0),
        layer(intersect(ellipse([300.0, 178.0], 16.0, 10.0, 0.0), rbox([300.0, 186.0], 20.0, 8.0, 0.0, 0.0)), 0x2b2230, 0.0),
    ]);
    v.extend(eye([276.0, 140.0], 11.0, [0.0, 0.2]));
    v.extend(eye([324.0, 140.0], 11.0, [0.0, 0.2]));
    v
}

/// A fox seen from the side, facing right: four legs and a big tail.
fn fox() -> Vec<Layer> {
    let o = 5.0;
    let (fur, fur_far, cream) = (0xe8843a, 0xc96a28, 0xfbeedd);
    let mut v = vec![
        // Far legs.
        layer(union(vec![limb([378.0, 320.0], [372.0, 390.0], 16.0, 13.0), limb([372.0, 390.0], [378.0, 445.0], 13.0, 11.0)]), fur_far, o),
        layer(ellipse([386.0, 452.0], 18.0, 10.0, 0.0), 0x3a2a24, o),
        layer(union(vec![limb([200.0, 315.0], [178.0, 385.0], 20.0, 14.0), limb([178.0, 385.0], [190.0, 445.0], 14.0, 11.0)]), fur_far, o),
        layer(ellipse([198.0, 452.0], 18.0, 10.0, 0.0), 0x3a2a24, o),
        // Tail.
        layer(chain(vec![[175.0, 290.0], [120.0, 270.0], [85.0, 225.0], [78.0, 165.0]], 22.0, 30.0), fur, o),
        layer(intersect(chain(vec![[175.0, 290.0], [120.0, 270.0], [85.0, 225.0], [78.0, 165.0]], 22.0, 30.0), circle([78.0, 160.0], 40.0)), cream, 0.0),
        // Body.
        layer(ellipse([290.0, 300.0], 125.0, 55.0, -4.0), fur, o),
        layer(intersect(ellipse([290.0, 300.0], 125.0, 55.0, -4.0), ellipse([300.0, 345.0], 95.0, 30.0, 0.0)), cream, 0.0),
        // Near legs.
        layer(union(vec![limb([400.0, 322.0], [404.0, 392.0], 17.0, 14.0), limb([404.0, 392.0], [410.0, 445.0], 14.0, 12.0)]), fur, o),
        layer(ellipse([420.0, 452.0], 19.0, 10.0, 0.0), 0x3a2a24, o),
        layer(union(vec![limb([222.0, 318.0], [205.0, 388.0], 22.0, 15.0), limb([205.0, 388.0], [220.0, 445.0], 15.0, 12.0)]), fur, o),
        layer(ellipse([229.0, 452.0], 19.0, 10.0, 0.0), 0x3a2a24, o),
        // Head: ears, face, snout.
        layer(tri([418.0, 200.0], [440.0, 125.0], [462.0, 195.0], 4.0), fur_far, o),
        layer(tri([450.0, 195.0], [482.0, 128.0], [494.0, 205.0], 4.0), fur, o),
        layer(tri([462.0, 190.0], [480.0, 148.0], [486.0, 196.0], 0.0), 0x3a2a24, 0.0),
        layer(smooth_union(circle([450.0, 240.0], 52.0), ellipse([505.0, 258.0], 42.0, 24.0, 8.0), 14.0), fur, o),
        layer(intersect(smooth_union(circle([450.0, 240.0], 52.0), ellipse([505.0, 258.0], 42.0, 24.0, 8.0), 14.0), ellipse([480.0, 280.0], 60.0, 26.0, 8.0)), cream, 0.0),
        layer(circle([543.0, 262.0], 10.0), 0x2b2230, 3.0),
    ];
    v.extend(eye([476.0, 228.0], 10.0, [1.0, 0.0]));
    v
}

/// A boneless slime: one blob, big eyes.
fn slime() -> Vec<Layer> {
    let body = || {
        intersect(
            smooth_union(ellipse([300.0, 330.0], 160.0, 150.0, 0.0), rbox([300.0, 452.0], 200.0, 32.0, 30.0, 0.0), 70.0),
            rbox([300.0, 300.0], 260.0, 180.0, 0.0, 0.0),
        )
    };
    let mut v = vec![
        layer(body(), 0x6fd06a, 6.0),
        layer(intersect(body(), ellipse([300.0, 460.0], 210.0, 50.0, 0.0)), 0x58b456, 0.0),
        layer(ellipse([215.0, 245.0], 38.0, 22.0, -30.0), 0xc8f3c2, 0.0),
        layer(circle([255.0, 210.0], 9.0), 0xe6fae2, 0.0),
    ];
    v.extend(eye([250.0, 320.0], 30.0, [0.3, 0.2]));
    v.extend(eye([350.0, 320.0], 30.0, [0.3, 0.2]));
    v.push(layer(intersect(ellipse([300.0, 385.0], 30.0, 20.0, 0.0), rbox([300.0, 400.0], 40.0, 15.0, 0.0, 0.0)), 0x2b2230, 0.0));
    v.push(layer(ellipse([220.0, 375.0], 16.0, 10.0, 0.0), 0xf29aa0, 0.0));
    v.push(layer(ellipse([380.0, 375.0], 16.0, 10.0, 0.0), 0xf29aa0, 0.0));
    v
}

/// The octopus's tentacles as points along their middles, and whether each is behind.
fn octopus_arms() -> Vec<(Vec<[f32; 2]>, bool)> {
    // (start x, heading in degrees with 90 = straight down, curl, far, segments)
    let arms = [
        (232.0, 128.0, 1.0, true, 18),
        (368.0, 52.0, -1.0, true, 18),
        (262.0, 108.0, 0.6, false, 22),
        (338.0, 72.0, -0.6, false, 22),
        (300.0, 90.0, 0.45, false, 22),
    ];
    arms
        .iter()
        .map(|&(x, heading, curl, far, n)| {
            let (mut p, mut a) = ([x, 300.0], f32::to_radians(heading));
            let mut pts = vec![p];
            for k in 1..=n {
                let t = k as f32 / n as f32;
                a += curl * 0.22 * t * t;
                p = [p[0] + a.cos() * 11.0, p[1] + a.sin() * 11.0];
                pts.push(p);
            }
            (pts, far)
        })
        .collect()
}

/// An octopus: a round head and five curling tentacles, spread apart.
fn octopus() -> Vec<Layer> {
    let o = 5.0;
    let (skin, skin_far) = (0xa36fd6, 0x8a58bd);
    let arms = octopus_arms();
    let mut v = Vec::new();
    for (pts, far) in &arms {
        v.push(layer(chain(pts.clone(), 30.0, 9.0), if *far { skin_far } else { skin }, o));
    }
    v.push(layer(ellipse([300.0, 210.0], 140.0, 125.0, 0.0), skin, o));
    v.push(layer(ellipse([250.0, 140.0], 34.0, 20.0, -25.0), 0xc9a3ee, 0.0));
    v.push(layer(circle([228.0, 225.0], 9.0), 0x8a58bd, 0.0));
    v.push(layer(circle([375.0, 240.0], 12.0), 0x8a58bd, 0.0));
    v.extend(eye([255.0, 255.0], 26.0, [0.0, 0.3]));
    v.extend(eye([345.0, 255.0], 26.0, [0.0, 0.3]));
    v.push(layer(limb([290.0, 305.0], [310.0, 305.0], 4.0, 4.0), 0x2b2230, 0.0));
    v
}

/// Builds a skeleton in drawing coordinates, bone by bone.
struct Rig {
    skel: anim_core::Skeleton,
}
impl Rig {
    fn new() -> Rig {
        Rig { skel: anim_core::Skeleton::empty() }
    }
    fn joint(&mut self, name: &str, pos: [f32; 2]) -> usize {
        if let Some(i) = self.skel.joints.iter().position(|j| j.name == name) {
            return i;
        }
        self.skel.joints.push(anim_core::Joint { name: name.into(), pos });
        self.skel.joints.len() - 1
    }
    /// A bone from joint `from` to joint `to` (made if new), hanging from `parent`.
    fn bone(&mut self, name: &str, from: (&str, [f32; 2]), to: (&str, [f32; 2]), parent: Option<&str>, limit: f32) {
        let pivot = self.joint(from.0, from.1);
        let tip = self.joint(to.0, to.1);
        let parent = parent.map(|p| self.skel.parts.iter().position(|b| b.name == p).expect(p));
        let layer = self.skel.parts.len() as i32;
        let limit = Some([-limit.to_radians(), limit.to_radians()]);
        self.skel.parts.push(anim_core::SkelPart { name: name.into(), pivot, tip, parent, limit, layer });
    }
}

fn fox_rig() -> Rig {
    let mut r = Rig::new();
    let legs = |r: &mut Rig, side: &str, end: &str, top: [f32; 2], knee: [f32; 2], paw: [f32; 2]| {
        let (leg, shin) = (format!("{side} {end} leg"), format!("{side} {end} shin"));
        let (t, k, p) = (format!("{side} {end} hip"), format!("{side} {end} knee"), format!("{side} {end} paw"));
        r.bone(&leg, (&t, top), (&k, knee), Some("body"), 60.0);
        r.bone(&shin, (&k, knee), (&p, paw), Some(&leg), 60.0);
    };
    // The body goes first so the legs can hang from it; layers are fixed below.
    // Legs start at the body's lower edge, so they don't carry the belly with them.
    r.bone("body", ("hips", [215.0, 300.0]), ("shoulders", [400.0, 295.0]), None, 30.0);
    legs(&mut r, "far", "back", [196.0, 340.0], [178.0, 385.0], [192.0, 452.0]);
    legs(&mut r, "far", "front", [376.0, 345.0], [372.0, 390.0], [382.0, 452.0]);
    r.bone("tail", ("tail base", [180.0, 290.0]), ("tail bend", [118.0, 268.0]), Some("body"), 60.0);
    r.bone("tail middle", ("tail bend", [118.0, 268.0]), ("tail curve", [86.0, 222.0]), Some("tail"), 60.0);
    r.bone("tail tip", ("tail curve", [86.0, 222.0]), ("tail end", [79.0, 140.0]), Some("tail middle"), 60.0);
    legs(&mut r, "near", "back", [216.0, 342.0], [205.0, 388.0], [224.0, 452.0]);
    legs(&mut r, "near", "front", [401.0, 347.0], [404.0, 392.0], [416.0, 452.0]);
    r.bone("head", ("neck", [400.0, 285.0]), ("head top", [478.0, 200.0]), Some("body"), 40.0);
    // Painting order, back to front: far legs, tail, body, near legs, head.
    let order = ["far back leg", "far back shin", "far front leg", "far front shin", "tail", "tail middle", "tail tip", "body",
        "near back leg", "near back shin", "near front leg", "near front shin", "head"];
    for (i, name) in order.iter().enumerate() {
        r.skel.parts.iter_mut().find(|b| b.name == *name).unwrap().layer = i as i32;
    }
    r
}

fn octopus_rig() -> Rig {
    let mut r = Rig::new();
    r.bone("head", ("head base", [300.0, 300.0]), ("head top", [300.0, 95.0]), None, 30.0);
    for (i, (pts, _)) in octopus_arms().iter().enumerate() {
        // Start at the head's edge, so the tentacles don't take its pixels.
        let pts = &pts[3..];
        let n = pts.len() - 1;
        let at = |k: usize| pts[k * n / 3];
        let names = [format!("tentacle {}", i + 1), format!("tentacle {} middle", i + 1), format!("tentacle {} tip", i + 1)];
        let joints = [format!("tentacle {} base", i + 1), format!("tentacle {} bend", i + 1), format!("tentacle {} curl", i + 1), format!("tentacle {} end", i + 1)];
        for k in 0..3 {
            let parent = if k == 0 { "head".to_string() } else { names[k - 1].clone() };
            r.bone(&names[k], (&joints[k], at(k)), (&joints[k + 1], at(k + 1)), Some(&parent), 70.0);
        }
    }
    // The head is drawn over every tentacle.
    let head = r.skel.parts.iter().position(|b| b.name == "head").unwrap();
    r.skel.parts[head].layer = r.skel.parts.len() as i32;
    r
}

fn blob_rig() -> Rig {
    let mut r = Rig::new();
    r.bone("body", ("bottom", [300.0, 478.0]), ("middle", [300.0, 330.0]), None, 30.0);
    r.bone("top", ("middle", [300.0, 330.0]), ("top", [300.0, 190.0]), Some("body"), 40.0);
    r
}

fn main() {
    let drawings: [(&str, Vec<Layer>); 4] = [("hero", hero()), ("fox", fox()), ("slime", slime()), ("octopus", octopus())];
    for (name, layers) in &drawings {
        render([600, 600], layers).save(format!("app/assets/samples/{name}.png")).unwrap();
    }
    // Templates, as fractions of the box around the drawing they were made on.
    for (file, template, drawing, rig) in [
        ("four_legs_side", "Four legs (side)", "fox", fox_rig()),
        ("tentacles", "Tentacles", "octopus", octopus_rig()),
        ("blob", "Blob", "slime", blob_rig()),
    ] {
        let img = image::open(format!("app/assets/samples/{drawing}.png")).unwrap().to_rgba8();
        let bbox = anim_core::foreground_mask(&img, &anim_core::auto_params(&img)).bbox().unwrap();
        let t = anim_core::Template::from_skeleton(template, &rig.skel, bbox);
        std::fs::write(format!("core/templates/{file}.json"), t.to_json()).unwrap();
    }
}
