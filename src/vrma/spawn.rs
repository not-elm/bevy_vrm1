use crate::vrm::humanoid_bone::{HumanoidBoneRegistry, HumanoidBonesAttached};
use crate::vrm::VrmExpression;
use crate::vrma::animation::{VrmAnimationGraph, VrmaAnimationPlayers};
use crate::vrma::gltf::extensions::VrmaExtensions;
use crate::vrma::loader::VrmaAsset;
use crate::vrma::{RetargetTo, Vrma, VrmaDuration, VrmaHandle, VrmaPath};
use bevy::gltf::GltfNode;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use std::time::Duration;

pub struct VrmaSpawnPlugin;

impl Plugin for VrmaSpawnPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, spawn_vrma);
    }
}

#[derive(Component, Deref, Reflect)]
pub struct VrmaExpressionNames(Vec<VrmExpression>);

impl VrmaExpressionNames {
    pub fn new(extensions: &VrmaExtensions) -> Self {
        let Some(expressions) = extensions.vrmc_vrm_animation.expressions.as_ref() else {
            return Self(Vec::default());
        };
        Self(
            expressions
                .preset
                .keys()
                .map(|expression| VrmExpression(expression.clone()))
                .collect(),
        )
    }
}
fn spawn_vrma(
    mut commands: Commands,
    mut animation_graphs: ResMut<Assets<AnimationGraph>>,
    vrma_assets: Res<Assets<VrmaAsset>>,
    node_assets: Res<Assets<GltfNode>>,
    clip_assets: Res<Assets<AnimationClip>>,
    vrma_handles: Query<(Entity, &VrmaHandle, &ChildOf)>,
    complements: Query<Entity, With<HumanoidBonesAttached>>,
    global_transform: Query<&GlobalTransform>,
) {
    for (handle_entity, handle, child_of) in vrma_handles.iter() {
        let vrm_entity = child_of.parent();
        if complements.get(vrm_entity).is_err() {
            continue;
        }
        if !global_transform.contains(vrm_entity) {
            continue;
        }
        let Some(vrma_path) = handle.0.path().map(|path| path.path().to_path_buf()) else {
            continue;
        };
        let Some(name) = handle.0.path().map(|p| p.to_string()) else {
            continue;
        };
        let Some(vrma) = vrma_assets.get(handle.0.id()) else {
            continue;
        };
        commands.entity(handle_entity).remove::<VrmaHandle>();

        let Some(scene_root) = vrma.gltf.scenes.first().cloned() else {
            error!("[VRMA] Not found vrma scene in {name}");
            continue;
        };
        let extensions = match VrmaExtensions::from_gltf(&vrma.gltf) {
            Ok(extensions) => extensions,
            Err(_e) => {
                error!("[VRMA] Not found vrma extensions in {name}:\n{_e}");
                continue;
            }
        };

        commands.entity(handle_entity).insert((
            Vrma,
            Name::new(name),
            VrmaAnimationPlayers::default(),
            RetargetTo(child_of.parent()),
            SceneRoot(scene_root),
            VrmaDuration(obtain_vrma_duration(&clip_assets, &vrma.gltf.animations)),
            VrmaPath(vrma_path),
            VrmAnimationGraph::new(vrma.gltf.animations.to_vec(), &mut animation_graphs),
            VrmaExpressionNames::new(&extensions),
            HumanoidBoneRegistry::new(
                &extensions.vrmc_vrm_animation.humanoid.human_bones,
                &node_assets,
                &vrma.gltf.nodes,
            ),
        ));
    }
}

fn obtain_vrma_duration(
    assets: &Assets<AnimationClip>,
    handles: &[Handle<AnimationClip>],
) -> Duration {
    let duration = handles
        .iter()
        .filter_map(|handle| assets.get(handle))
        .map(|clip| clip.duration() as f64)
        .fold(0., |v1, v2| v2.max(v1));
    Duration::from_secs_f64(duration)
}
