//! UI-independent core: image ops and cutouts. Must never depend on UI crates.

pub mod background;
pub mod components;
pub mod cutout;
pub mod geom;
pub mod history;
pub mod mask;
pub mod parts;
pub mod skeleton;

pub use cutout::{Params, foreground_mask, part_mask};
pub use geom::RectI;
pub use history::History;
pub use mask::Mask;
pub use parts::{Axis, PartBox, PartMasks, part_masks};
pub use skeleton::{Joint, SkelPart, Skeleton, SkeletonMasks, skeleton_masks};
