//! UI-independent core: image ops and cutouts. Must never depend on UI crates.

pub mod background;
pub mod components;
pub mod cutout;
pub mod fill;
pub mod geom;
pub mod history;
pub mod mask;
pub mod mesh;
pub mod par;
pub mod motion;
pub mod project;
pub mod rig;
pub mod skeleton;
pub mod sprites;
pub mod template;
pub mod weight_paint;

pub use cutout::{Params, foreground_mask};
pub use geom::RectI;
pub use history::History;
pub use mask::Mask;
pub use mesh::{PartMesh, build_meshes, cell_size, smooth_weights};
pub use motion::{Clip, Ease, Key, KeyRef, Motion, TrackId};
pub use project::{LoadedProject, ProjectData, load_project, project_from_bytes, project_to_bytes, save_project};
pub use rig::{
    Affine2, Pose, aim_part, angle_to_point, clamp_to_limit, ik_parent, posed_joint, solve_two_bone, world_transforms,
};
pub use sprites::{Sprite, build_sprites};
pub use skeleton::{Joint, NO_PART, SkelPart, Skeleton, SkeletonMasks, Toward, distance_transform, masks_from_labels, part_labels, skeleton_masks, skeleton_masks_with_dt};
pub use template::Template;
pub use weight_paint::{WeightPaint, apply_paint};
