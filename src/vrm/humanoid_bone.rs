//! This module handles humanoid bones.
//! Refer to [here](https://docs.unity3d.com/ja/2019.4/ScriptReference/HumanBodyBones.html) for the list of humanoid bones.
//!
//! After the VRM(A) is loaded, marker components are inserted for each bone.
//! For example, the entity of the hips bone will have [`Hips`] inserted.
//! Additionally, a component that holds the entity will be inserted into the VRM(A) entity.
//!
//! The setup of these is done after all bones have been spawned, so there may be a slight delay.

mod bones;

use crate::prelude::*;
use crate::vrm::VrmBone;
use crate::vrm::gltf::extensions::VrmNode;
use crate::vrm::humanoid_bone::bones::BonesPlugin;
use bevy::app::{App, Plugin};
use bevy::asset::{Assets, Handle};
use bevy::gltf::GltfNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

pub mod prelude {
    pub use crate::vrm::humanoid_bone::bones::*;
}

#[derive(Component, Deref, Reflect, Default)]
pub(crate) struct HumanoidBoneRegistry(HashMap<VrmBone, Name>);

impl HumanoidBoneRegistry {
    pub fn new(
        bones: &HashMap<String, VrmNode>,
        node_assets: &Assets<GltfNode>,
        nodes: &[Handle<GltfNode>],
    ) -> Self {
        Self(
            bones
                .iter()
                .filter_map(|(name, target_node)| {
                    let node_handle = nodes.get(target_node.node)?;
                    let node = node_assets.get(node_handle)?;
                    Some((VrmBone(name.clone()), Name::new(node.name.clone())))
                })
                .collect(),
        )
    }
}

#[cfg(test)]
impl HumanoidBoneRegistry {
    pub(crate) fn from_pairs(pairs: impl IntoIterator<Item = (VrmBone, Name)>) -> Self {
        Self(pairs.into_iter().collect())
    }
}

pub(super) struct VrmHumanoidBonePlugin;

impl Plugin for VrmHumanoidBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<HumanoidBoneRegistry>();
        register_bone_components(app);
    }
}

macro_rules! insert_bone {
    (
        $commands: expr,
        $vrm_entity: expr,
        $bone_entity: expr,
        $bone_name: expr,
        $($bone: ident),+$(,)?
    ) => {

        match $bone_name.0.to_uppercase(){
            $(
                x if x == stringify!($bone).to_uppercase() => {
                    paste::paste!{
                        $commands.entity($vrm_entity).insert([<$bone BoneEntity>]($bone_entity));
                    }
                    $commands.entity($bone_entity).insert($bone);
                }
            )+
            _ => {

            }
        }
    };
}

pub(crate) fn insert_bone_holders(
    root: Entity,
    commands: &mut Commands,
    bones: &[(VrmBone, Entity)],
) {
    for (bone, bone_entity) in bones {
        let bone_entity = *bone_entity;
        insert_bone!(
            commands,
            root,
            bone_entity,
            bone,
            Hips,
            RightRingProximal,
            RightThumbDistal,
            RightRingIntermediate,
            RightUpperArm,
            LeftIndexProximal,
            LeftUpperLeg,
            LeftFoot,
            LeftIndexDistal,
            LeftThumbMetacarpal,
            RightLowerArm,
            LeftMiddleDistal,
            RightUpperLeg,
            LeftToes,
            LeftThumbDistal,
            RightShoulder,
            RightThumbMetacarpal,
            Spine,
            LeftLowerLeg,
            LeftShoulder,
            LeftUpperArm,
            UpperChest,
            RightToes,
            RightIndexDistal,
            LeftMiddleProximal,
            LeftRingProximal,
            LeftRingDistal,
            LeftThumbProximal,
            LeftIndexIntermediate,
            LeftLittleProximal,
            LeftLittleDistal,
            RightHand,
            RightLittleProximal,
            LeftRingIntermediate,
            RightIndexIntermediate,
            Chest,
            LeftHand,
            RightLittleIntermediate,
            RightFoot,
            RightLowerLeg,
            LeftLittleIntermediate,
            LeftLowerArm,
            RightLittleDistal,
            RightMiddleIntermediate,
            RightMiddleProximal,
            RightThumbProximal,
            Neck,
            Jaw,
            Head,
            LeftEye,
            RightEye,
            LeftMiddleIntermediate,
            RightRingDistal,
            RightIndexProximal,
            RightMiddleDistal,
        );
    }
}

/// `BonesPlugin` is unique, and both `VrmPlugin` and `VrmGltfPlugin` register it.
pub(crate) fn register_bone_components(app: &mut App) {
    if !app.is_plugin_added::<BonesPlugin>() {
        app.add_plugins(BonesPlugin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_bone_components_is_idempotent() {
        let mut app = App::new();
        register_bone_components(&mut app);
        register_bone_components(&mut app);
        assert!(app.is_plugin_added::<BonesPlugin>());
    }
}
