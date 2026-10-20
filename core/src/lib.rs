//! UI-independent core: image ops and cutouts. Must never depend on UI crates.

pub mod background;
pub mod components;
pub mod cutout;
pub mod geom;
pub mod history;
pub mod mask;
pub mod motion;
pub mod project;
pub mod rig;
pub mod skeleton;
pub mod sprites;

pub use cutout::{Params, foreground_mask};
pub use geom::RectI;
pub use history::History;
pub use mask::Mask;
pub use motion::{Clip, Ease, Key, KeyRef, Motion, TrackId};
pub use project::{LoadedProject, ProjectData, load_project, save_project};
pub use rig::{
    Affine2, Pose, aim_part, angle_to_point, clamp_to_limit, ik_parent, posed_joint, solve_two_bone, world_transforms,
};
pub use sprites::{Sprite, build_sprites};
pub use skeleton::{Joint, SkelPart, Skeleton, SkeletonMasks, skeleton_masks};
