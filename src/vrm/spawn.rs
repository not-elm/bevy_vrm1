use crate::vrm::expressions::VrmExpressionRegistry;
use crate::vrm::gltf::extensions::VrmExtensions;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::loader::{VrmAsset, VrmHandle};
use crate::vrm::mtoon::VrmcMaterialRegistry;
use crate::vrm::spring_bone::registry::*;
use crate::vrm::{Vrm, VrmPath};
use bevy::app::{App, Update};
use bevy::asset::Assets;
use bevy::gltf::GltfNode;
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
            Err(_e) => {
                error!("Failed to load VRM extensions: {_e}");
                continue;
            }
        };

        let mut cmd = commands.entity(vrm_handle_entity);
        cmd.insert((
            Vrm,
            Name::new(extensions.name().unwrap_or_else(|| "VRM".to_string())),
            SceneRoot(scene.clone()),
            VrmcMaterialRegistry::new(&vrm.gltf, vrm.images.clone()),
            VrmExpressionRegistry::new(&extensions, &node_assets, &vrm.gltf.nodes),
            HumanoidBoneRegistry::new(
                &extensions.vrmc_vrm.humanoid.human_bones,
                &node_assets,
                &vrm.gltf.nodes,
            ),
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

        if let Some(look_at) = extensions.vrmc_vrm.look_at.clone() {
            cmd.insert(look_at);
        }

        if let Some(vrm_path) = handle.0.path() {
            #[cfg(feature = "develop")]
            {
                if let Some(vrm_name) = vrm_path.path().file_stem() {
                    output_vrm_materials(vrm_name, &vrm.gltf);
                    output_vrm_extensions(vrm_name, &extensions);
                }
            }
            cmd.insert(VrmPath::new(vrm_path.path()));
        }
    }
}

#[cfg(feature = "develop")]
fn output_vrm_materials(
    vrm_name: &std::ffi::OsStr,
    gltf: &Gltf,
) {
    let name = vrm_name.to_str().unwrap();
    std::fs::write(
        format!("./develop/{name}_materials.json"),
        serde_json::to_string_pretty(&gltf.source.as_ref().unwrap().as_json().materials).unwrap(),
    )
    .unwrap();
}

#[cfg(feature = "develop")]
fn output_vrm_extensions(
    vrm_name: &std::ffi::OsStr,
    extensions: &VrmExtensions,
) {
    let name = vrm_name.to_str().unwrap();
    std::fs::write(
        format!("{name}.json"),
        serde_json::to_string_pretty(extensions).unwrap(),
    )
    .unwrap();
}
