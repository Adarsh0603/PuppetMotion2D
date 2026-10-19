//! UI-independent core: image ops and cutouts. Must never depend on UI crates.

pub mod background;
pub mod components;
pub mod cutout;
pub mod geom;
pub mod history;
pub mod mask;
pub mod parts;
pub mod rig;
pub mod skeleton;
pub mod sprites;

pub use cutout::{Params, foreground_mask, part_mask};
pub use geom::RectI;
pub use history::History;
pub use mask::Mask;
pub use parts::{Axis, PartBox, PartMasks, part_masks};
pub use rig::{Affine2, Pose, angle_to_point, posed_joint, world_transforms};
pub use sprites::{Sprite, build_sprites, write_parts};
pub use skeleton::{Joint, SkelPart, Skeleton, SkeletonMasks, skeleton_masks};
