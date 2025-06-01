pub mod expressions;
pub mod gltf;
pub mod humanoid_bone;
pub mod loader;
pub mod look_at;
mod mtoon;
mod spawn;
mod spring_bone;

use crate::new_type;
use crate::vrm::humanoid_bone::VrmHumanoidBonePlugin;
use crate::vrm::loader::{VrmAsset, VrmLoaderPlugin};
use crate::vrm::look_at::LookAtPlugin;
use crate::vrm::spawn::VrmSpawnPlugin;
use crate::vrm::spring_bone::VrmSpringBonePlugin;
use bevy::app::{App, Plugin};
use bevy::asset::AssetApp;
use bevy::prelude::*;
use expressions::VrmExpressionPlugin;
use mtoon::MtoonMaterialPlugin;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

new_type!(
    /// The bone name obtained from `VRMC_vrm::humanoid`.
    name: VrmBone,
    ty: String,
);

new_type!(
    /// The key name of `VRMC_vrm::expressions::preset`.
    name: VrmExpression,
    ty: String,
);

/// A marker component attached to the entity of VRM.
#[derive(Debug, Component, Reflect, Copy, Clone, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct Vrm;

/// The path to the VRM file.
#[derive(Debug, Reflect, Clone, Component, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct VrmPath(pub PathBuf);

impl VrmPath {
    /// Creates a new [`VrmPath`] from the path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }
}

/// The bone's initial transform.
#[derive(Debug, Copy, Clone, Component)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct BoneRestTransform(pub Transform);

/// The bone's initial global transform.
#[derive(Debug, Copy, Clone, Component)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct BoneRestGlobalTransform(pub GlobalTransform);

/// Holds the entity of the hips bone.
#[derive(Debug, Copy, Clone, Component)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct VrmHipsBoneTo(pub Entity);

/// A marker components for left eye bone.
#[derive(Debug, Copy, Clone, Component)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct LeftEye(pub Entity);

/// A marker components for right eye bone.
#[derive(Debug, Copy, Clone, Component)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct RightEye(pub Entity);

/// Holds the entity of the vrm head bone.
#[derive(Debug, Copy, Clone, Component)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct Head(pub Entity);

pub struct VrmPlugin;

impl Plugin for VrmPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_asset::<VrmAsset>().add_plugins((
            VrmLoaderPlugin,
            VrmSpawnPlugin,
            VrmSpringBonePlugin,
            VrmHumanoidBonePlugin,
            VrmExpressionPlugin,
            MtoonMaterialPlugin,
            LookAtPlugin,
        ));

        #[cfg(feature = "reflect")]
        {
            app.register_type::<Vrm>()
                .register_type::<VrmPath>()
                .register_type::<BoneRestTransform>()
                .register_type::<BoneRestGlobalTransform>()
                .register_type::<VrmHipsBoneTo>()
                .register_type::<VrmBone>()
                .register_type::<LeftEye>()
                .register_type::<RightEye>()
                .register_type::<Head>();
        }
    }
}
