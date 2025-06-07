use bevy::prelude::SystemSet;

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Ord, PartialOrd, Clone, Copy)]
pub enum VrmSystemSets {
    /// This is used for retargeting VRMA animations.
    Retarget,

    /// This is used for look-at functionality.
    LookAt,

    /// This is used for spring bones.
    SpringBone,
}
