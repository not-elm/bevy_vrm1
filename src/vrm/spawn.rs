use crate::vrm::expressions::VrmExpressionRegistry;
use crate::vrm::extensions::VrmExtensions;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::loader::{VrmAsset, VrmHandle};
use crate::vrm::spring_bone::registry::*;
use crate::vrm::{Vrm, VrmPath};
use bevy::app::{App, Update};
use bevy::asset::Assets;
use bevy::gltf::GltfNode;
use bevy::log::error;
use bevy::prelude::*;
use bevy::scene::SceneRoot;

pub struct VrmSpawnPlugin;

impl Plugin for VrmSpawnPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, spawn_vrm);
    }
}

fn spawn_vrm(
    mut commands: Commands,
    node_assets: Res<Assets<GltfNode>>,
    vrm_assets: Res<Assets<VrmAsset>>,
    handles: Query<(Entity, &VrmHandle)>,
) {
    for (vrm_handle_entity, handle) in handles.iter() {
        let Some(vrm) = vrm_assets.get(handle.0.id()) else {
            continue;
        };
        commands.entity(vrm_handle_entity).remove::<VrmHandle>();

        let Some(scene) = vrm.gltf.scenes.first() else {
            continue;
        };
        let extensions = match VrmExtensions::from_gltf(&vrm.gltf) {
            Ok(extensions) => extensions,
            Err(e) => {
                error!("[VRM] {e}");
                continue;
            }
        };

        let mut cmd = commands.entity(vrm_handle_entity);
        cmd.insert((
            Vrm,
            SceneRoot(scene.clone()),
            VrmExpressionRegistry::new(&extensions, &node_assets, &vrm.gltf.nodes),
            HumanoidBoneRegistry::new(
                &extensions.vrmc_vrm.humanoid.human_bones,
                &node_assets,
                &vrm.gltf.nodes,
            ),
            Name::new(extensions.name().unwrap_or_else(|| "VRM".to_string())),
        ));

        if let Some(spring_bone) = extensions.vrmc_spring_bone.as_ref() {
            cmd.insert((
                SpringJointPropsRegistry::new(
                    &spring_bone.all_joints(),
                    &node_assets,
                    &vrm.gltf.nodes,
                ),
                SpringColliderRegistry::new(&spring_bone.colliders, &node_assets, &vrm.gltf.nodes),
                SpringNodeRegistry::new(spring_bone, &node_assets, &vrm.gltf.nodes),
            ));
        }

        if let Some(vrm_path) = handle.0.path() {
            cmd.insert(VrmPath::new(vrm_path.path()));
        }
    }
}
