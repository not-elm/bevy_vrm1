mod bones;

use crate::system_param::child_searcher::ChildSearcher;
use crate::vrm::gltf::extensions::VrmNode;
use crate::vrm::{
    BoneRestGlobalTransform, BoneRestTransform, VrmBone,
};
use bevy::app::{App, Plugin, Update};
use bevy::asset::{Assets, Handle};
use bevy::gltf::GltfNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use crate::vrm::humanoid_bone::bones::BonesPlugin;
use crate::prelude::*;

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

pub(super) struct VrmHumanoidBonePlugin;

impl Plugin for VrmHumanoidBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<HumanoidBonesAttached>()
            .register_type::<HumanoidBoneRegistry>()
            .add_plugins(BonesPlugin)
            .add_systems(Update, setup_bones);
    }
}

#[derive(Component, Reflect, Serialize, Deserialize)]
pub struct HumanoidBonesAttached;

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

fn setup_bones(
    mut commands: Commands,
    searcher: ChildSearcher,
    vrm: Query<(Entity, &HumanoidBoneRegistry), Without<HumanoidBonesAttached>>,
    transforms: Query<(&Transform, &GlobalTransform)>,
) {
    for (vrm_entity, humanoid_bones) in vrm.iter() {
        if !searcher.has_been_spawned_all_bones(vrm_entity, humanoid_bones) {
            continue;
        }

        for (bone, name) in humanoid_bones.iter() {
            let Some(bone_entity) = searcher.find_from_name(vrm_entity, name.as_str()) else {
                continue;
            };
            let Ok((tf, gtf)) = transforms.get(bone_entity) else {
                continue;
            };
            commands.entity(bone_entity).insert((
                bone.clone(),
                BoneRestTransform(*tf),
                BoneRestGlobalTransform(*gtf),
            ));
            insert_bone!(
                commands,
                vrm_entity,
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
            commands.entity(vrm_entity).insert(HumanoidBonesAttached);
        }
    }
}
