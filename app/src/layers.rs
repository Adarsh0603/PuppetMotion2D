//! A drawing opened as several layer images (one PNG per part): what the
//! document keeps about them, and their cards in the side panel. The work
//! itself (stacking, guessing bones, cutting pieces) is in `anim_core::layers`.

use crate::panel::card;
use crate::state::{AppState, part_color};
use anim_core::{Layer, Layers};
use eframe::egui;

/// A document's layers.
pub struct DocLayers {
    pub set: Layers,
    /// Each layer as a PNG file, kept for saving.
    pub pngs: Vec<Vec<u8>>,
    /// Bones chosen by hand per layer (by name); empty = guessed. Undoable.
    pub chosen: Vec<Vec<String>>,
    /// The bones each layer moves with now (chosen or guessed), updated with the split.
    pub bones: Vec<Vec<usize>>,
    /// Which layer each of the document's pieces came from (updated with them).
    pub piece_layer: Vec<usize>,
}

impl DocLayers {
    /// Layers bottom to top, with hand-chosen bones (by name) for each, if any.
    pub fn new(layers: Vec<Layer>, chosen: Vec<Vec<String>>) -> Result<DocLayers, String> {
        let pngs = layers.iter().map(|l| png_bytes(&l.image)).collect::<Result<Vec<_>, _>>()?;
        let n = layers.len();
        let set = Layers::new(layers)?;
        let mut chosen = chosen;
        chosen.resize(n, Vec::new());
        Ok(DocLayers { set, pngs, chosen, bones: vec![Vec::new(); n], piece_layer: Vec::new() })
    }

    /// The same layers in a new order (`order` lists old positions, bottom to top).
    pub fn reordered(&self, order: &[usize]) -> Result<DocLayers, String> {
        let layers = order.iter().map(|&i| self.set.layers[i].clone()).collect();
        let chosen = order.iter().map(|&i| self.chosen[i].clone()).collect();
        DocLayers::new(layers, chosen)
    }
}

/// An image as PNG file bytes.
pub fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Decode picked image files into layers, ordered by file name (numbers in
/// names count as numbers, so "leg 2" comes before "leg 10"): bottom to top.
pub fn decode(files: Vec<(String, Vec<u8>)>) -> Result<Vec<Layer>, String> {
    let mut files = files;
    files.sort_by(|a, b| natural_cmp(&a.0, &b.0));
    files
        .into_iter()
        .map(|(name, bytes)| {
            let image = image::load_from_memory(&bytes).map_err(|e| format!("Could not read {name}: {e}"))?.to_rgba8();
            let stem = std::path::Path::new(&name).file_stem().unwrap_or_default().to_string_lossy().into_owned();
            Ok(Layer { name: stem, image })
        })
        .collect()
}

/// A name for a character made of these layers: the start their names
/// share ("hero_body", "hero_arm" → "hero"), else "character".
pub fn shared_name(layers: &[Layer]) -> String {
    let first = layers.first().map_or("", |l| l.name.as_str());
    let mut len = first.len();
    for l in layers {
        len = len.min(first.bytes().zip(l.name.bytes()).take_while(|(a, b)| a == b).count());
    }
    let name = first[..len].trim_end_matches(|c: char| !c.is_alphanumeric());
    // A shared number ("01 arm", "02 leg") isn't a name.
    if name.chars().any(char::is_alphabetic) { name.into() } else { "character".into() }
}

/// Compare names with runs of digits taken as numbers.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let num = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s.trim_start_matches('0').to_string()
                };
                let (na, nb) = (num(&mut a), num(&mut b));
                let ord = na.len().cmp(&nb.len()).then(na.cmp(&nb));
                if ord.is_ne() {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord.is_ne() {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Review step: the layers from front (top of the list) to back, with
/// buttons to move one up or down, and to flip the whole order (drawing apps
/// number exported layers from the top or from the bottom).
pub fn order_card(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(ls) = state.doc.as_ref().and_then(|d| d.layers.as_ref()) else { return };
    let n = ls.set.layers.len();
    let names: Vec<String> = ls.set.layers.iter().map(|l| l.name.clone()).collect();
    let mut new_order: Option<Vec<usize>> = None;
    card(ui, &format!("Layers ({n})"), |ui| {
        ui.label(egui::RichText::new("The top of the list is in front. Check that the far arm and leg are below the body.").weak());
        for row in 0..n {
            // Rows go front to back; layers are stored back to front.
            let i = n - 1 - row;
            ui.horizontal(|ui| {
                let mut order: Vec<usize> = (0..n).collect();
                if ui.add_enabled(row > 0, egui::Button::new("▲").small()).on_hover_text("Forward").clicked() {
                    order.swap(i, i + 1);
                    new_order = Some(order.clone());
                }
                if ui.add_enabled(row + 1 < n, egui::Button::new("▼").small()).on_hover_text("Back").clicked() {
                    order.swap(i, i - 1);
                    new_order = Some(order);
                }
                ui.label(&names[i]);
            });
        }
        if ui.button("Reverse order").on_hover_text("If the layers came in upside down").clicked() {
            new_order = Some((0..n).rev().collect());
        }
    });
    if let Some(order) = new_order {
        state.reorder_layers(ui.ctx(), &order);
    }
}

/// What a layer moves with, as shown and picked in Rig.
#[derive(Clone, PartialEq)]
enum MovesWith {
    Guess,
    /// One bone, by name.
    Bone(String),
    /// A bone and every bone below it (a whole leg drawn on one layer).
    Below(String),
}

/// Rig step: each layer's "Moves with" choice, front to back.
pub fn bones_card(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(doc) = &state.doc else { return };
    let Some(ls) = &doc.layers else { return };
    let skel = &doc.skeleton;
    let n = ls.set.layers.len();
    let below = |b: usize| -> Vec<String> {
        std::iter::once(b).chain(skel.descendants(b)).map(|p| skel.parts[p].name.clone()).collect()
    };
    let current = |l: usize| -> MovesWith {
        let c = &ls.chosen[l];
        match c.as_slice() {
            [] => MovesWith::Guess,
            [one] => MovesWith::Bone(one.clone()),
            [first, ..] => match skel.parts.iter().position(|p| &p.name == first) {
                Some(b) if below(b) == *c => MovesWith::Below(first.clone()),
                _ => MovesWith::Bone(first.clone()),
            },
        }
    };
    let names_of = |bones: &[usize]| bones.iter().map(|&b| skel.parts[b].name.as_str()).collect::<Vec<_>>().join(", ");
    let mut change: Option<(usize, Vec<String>)> = None;
    card(ui, "Layers", |ui| {
        ui.label(egui::RichText::new("What each layer moves with. Guess goes by the layer's name and where it's drawn.").weak());
        egui::Grid::new("layer bones").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for row in 0..n {
                let l = n - 1 - row;
                let color = ls.bones[l].first().map_or(egui::Color32::GRAY, |&b| part_color(b));
                ui.label(egui::RichText::new(&ls.set.layers[l].name).color(color));
                let now = current(l);
                let text = match &now {
                    MovesWith::Guess => format!("Guess: {}", names_of(&ls.bones[l])),
                    MovesWith::Bone(b) => b.clone(),
                    MovesWith::Below(b) => format!("{b} and below"),
                };
                egui::ComboBox::from_id_salt(("moves with", l)).selected_text(text).width(200.0).show_ui(ui, |ui| {
                    if ui.selectable_label(now == MovesWith::Guess, "Guess").clicked() {
                        change = Some((l, Vec::new()));
                    }
                    for (b, part) in skel.parts.iter().enumerate() {
                        if ui.selectable_label(now == MovesWith::Bone(part.name.clone()), &part.name).clicked() {
                            change = Some((l, vec![part.name.clone()]));
                        }
                        if skel.children(b).next().is_some() {
                            let label = format!("{} and below", part.name);
                            if ui.selectable_label(now == MovesWith::Below(part.name.clone()), label).clicked() {
                                change = Some((l, below(b)));
                            }
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
    if let Some((l, bones)) = change {
        state.choose_layer_bones(l, bones);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_with_numbers_as_numbers() {
        let mut names = vec!["leg 10", "Leg 2", "arm", "leg 1"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["arm", "leg 1", "Leg 2", "leg 10"]);
    }

    #[test]
    fn shared_name_is_the_common_start() {
        let l = |n: &str| Layer { name: n.into(), image: image::RgbaImage::new(1, 1) };
        assert_eq!(shared_name(&[l("hero_body"), l("hero_arm")]), "hero");
        assert_eq!(shared_name(&[l("body"), l("arm")]), "character");
        assert_eq!(shared_name(&[l("01 body"), l("02 arm")]), "character");
    }
}
