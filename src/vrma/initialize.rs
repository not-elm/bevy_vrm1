//! This module inserts [`SceneRoot`] and VRMA-related components from the loaded [`VrmaHandle`].

use crate::error::vrm_error;
use crate::system_param::prelude::*;
use crate::vrm::Initialized;
use crate::vrm::humanoid_bone::{HumanoidBoneRegistry, insert_bone_holders};
use crate::vrm::{RestGlobalTransform, RestTransform, RestWorldTransform, Vrm};
use crate::vrma::animation::animation_graph::RequestUpdateAnimationGraph;
use crate::vrma::animation::expressions::VrmaExpressionRegistry;
use crate::vrma::gltf::extensions::VrmaExtensions;
use crate::vrma::loader::VrmaAsset;
use crate::vrma::{RetargetSource, VrmAnimationSourceClip, VrmaPlaybackState};
use crate::vrma::{VrmAnimationClipHandle, Vrma, VrmaDuration, VrmaHandle, VrmaPath};
use bevy::gltf::GltfNode;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
use std::time::Duration;

pub(super) struct VrmaInitializePlugin;

impl Plugin for VrmaInitializePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(
            Update,
            (spawn_vrma, request_initialize_vrma, trigger_loaded),
        )
        .add_observer(on_vrm_initialized);
    }
}

/// Rebuilds the animation graph whenever a VRM (re)attaches its rig.
fn on_vrm_initialized(
    trigger: On<Insert, Initialized>,
    vrms: Query<(), With<Vrm>>,
    mut commands: Commands,
) {
    let vrm = trigger.event_target();
    if vrms.get(vrm).is_err() {
        return;
    }
    commands.trigger(RequestUpdateAnimationGraph { vrm });
}

fn spawn_vrma(
    mut commands: Commands,
    vrma_assets: Res<Assets<VrmaAsset>>,
    node_assets: Res<Assets<GltfNode>>,
    mut clip_assets: ResMut<Assets<AnimationClip>>,
    mut scene_assets: ResMut<Assets<WorldAsset>>,
    type_registry: Res<AppTypeRegistry>,
    vrma_handles: Query<(Entity, &VrmaHandle, &ChildOf)>,
    vrms: Query<Has<Initialized>>,
) {
    for (handle_entity, handle, child_of) in vrma_handles.iter() {
        if !vrms
            .get(child_of.parent())
            .is_ok_and(|initialized| initialized)
        {
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

        let Some(scene_handle) = vrma.gltf.scenes.first() else {
            vrm_error!("[VRMA] Not found vrma scene in {name}");
            continue;
        };
        // Clone the Scene asset per-VRMA instance to prevent SceneSpawner
        // from grouping instances by shared AssetId and respawning them together.
        let Some(source_scene) = scene_assets.get(scene_handle) else {
            vrm_error!("[VRMA] Not found scene data for {name}");
            continue;
        };
        let Ok(cloned_scene) = source_scene.clone_with(&type_registry) else {
            vrm_error!("[VRMA] Failed to clone scene for {name}");
            continue;
        };
        let scene_root = scene_assets.add(cloned_scene);
        let extensions = match VrmaExtensions::from_gltf(&vrma.gltf) {
            Ok(extensions) => extensions,
            Err(_e) => {
                vrm_error!("[VRMA] Not found vrma extensions in {name}:\n{_e}");
                continue;
            }
        };
        let Some(animation_clip_handle) = vrma.gltf.animations.first() else {
            vrm_error!("[VRMA] Not found vrma animations in {name}");
            continue;
        };
        let Some(animation_clip) = clip_assets.get(animation_clip_handle).cloned() else {
            vrm_error!("[VRMA] Not found animation clip for {name}");
            continue;
        };
        let source_clip = VrmAnimationSourceClip(animation_clip_handle.clone());
        let animation_clip_handle = clip_assets.add(animation_clip);
        commands.entity(handle_entity).insert((
            Vrma,
            Name::new(name),
            source_clip,
            VrmAnimationClipHandle(animation_clip_handle.clone()),
            WorldAssetRoot(scene_root),
            VrmaDuration(obtain_vrma_duration(&clip_assets, &vrma.gltf.animations)),
            VrmaPath(vrma_path),
            VrmaExpressionRegistry::new(&extensions, &node_assets, &vrma.gltf.nodes),
            HumanoidBoneRegistry::new(
                &extensions.vrmc_vrm_animation.humanoid.human_bones,
                &node_assets,
                &vrma.gltf.nodes,
            ),
            VrmaPlaybackState::default(),
        ));
    }
}

/// Captures the source rig's rest pose, puts the bone markers and holders on
/// it, and marks the VRMA [`Initialized`] once its bones spawn.
fn request_initialize_vrma(
    mut commands: Commands,
    models: Query<(Entity, &HumanoidBoneRegistry), (With<Vrma>, Without<Initialized>)>,
    transforms: Query<(&Transform, &GlobalTransform)>,
    searcher: ChildSearcher,
) {
    for (root, registry) in models.iter() {
        if !searcher.has_been_spawned_all_bones(root, registry) {
            continue;
        }
        if let Ok((_, global)) = transforms.get(root) {
            commands.entity(root).insert(RestWorldTransform(*global));
        }
        let mut bones = Vec::new();
        for (bone, name) in registry.iter() {
            let Some(entity) = searcher.find_from_name(root, name) else {
                continue;
            };
            let Ok((tf, gtf)) = transforms.get(entity) else {
                continue;
            };
            commands.entity(entity).insert((
                bone.clone(),
                RestTransform(*tf),
                RestGlobalTransform(*gtf),
                RetargetSource,
            ));
            bones.push((bone.clone(), entity));
        }
        insert_bone_holders(root, &mut commands, &bones);
        commands.entity(root).insert(Initialized);
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
    // A sampler timestamp is any finite float, so a clip can outlast what a
    // `Duration` holds (about 1.8e19 s); `from_secs_f64` would panic on it.
    Duration::try_from_secs_f64(duration).unwrap_or(Duration::MAX)
}

fn trigger_loaded(
    mut commands: Commands,
    vrmas: Query<(Has<Initialized>, Has<VrmaHandle>), Or<(With<Vrma>, With<VrmaHandle>)>>,
    newly_initialized: Query<(Entity, &ChildOf), (Added<Initialized>, With<Vrma>)>,
    vrm_children: Query<&Children>,
    initialized_models: Query<(), With<Initialized>>,
) {
    let mut requested = HashSet::new();
    for (_, child_of) in newly_initialized.iter() {
        let vrm_entity = child_of.parent();
        let all_vrmas_initialized = vrm_children.get(vrm_entity).ok().is_some_and(|children| {
            children.iter().all(|child| match vrmas.get(child) {
                Ok((initialized, pending_load)) => initialized && !pending_load,
                Err(_) => true,
            })
        });
        if all_vrmas_initialized
            && initialized_models.get(vrm_entity).is_ok()
            && requested.insert(vrm_entity)
        {
            // Build one stable graph after every VRMA child is initialized.
            // Rebuilding once per child changes positional node indices and can
            // leave already-started players pointing at another clip.
            commands.trigger(RequestUpdateAnimationGraph { vrm: vrm_entity });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct GraphRequests(Vec<Entity>);

    /// A `.vrma` whose sampler holds a huge timestamp must not abort the app
    /// while its duration is read, before the bake caps its sample count.
    #[test]
    fn a_huge_clip_duration_saturates_instead_of_panicking() {
        let mut clips = Assets::<AnimationClip>::default();
        let mut clip = AnimationClip::default();
        clip.set_duration(f32::MAX);
        let handle = clips.add(clip);

        assert_eq!(obtain_vrma_duration(&clips, &[handle]), Duration::MAX);
    }

    #[test]
    fn waits_for_unloaded_handles_and_requests_one_graph_per_model() {
        let mut app = App::new();
        app.init_resource::<GraphRequests>()
            .add_systems(Update, trigger_loaded)
            .add_observer(
                |event: On<RequestUpdateAnimationGraph>, mut requests: ResMut<GraphRequests>| {
                    requests.0.push(event.vrm);
                },
            );
        let vrm = app.world_mut().spawn(Initialized).id();
        app.world_mut().spawn((Vrma, Initialized, ChildOf(vrm)));
        let pending = app
            .world_mut()
            .spawn((VrmaHandle(Handle::default()), ChildOf(vrm)))
            .id();
        app.update();
        assert!(app.world().resource::<GraphRequests>().0.is_empty());

        app.world_mut()
            .entity_mut(pending)
            .remove::<VrmaHandle>()
            .insert((Vrma, Initialized));
        app.update();
        assert_eq!(app.world().resource::<GraphRequests>().0, vec![vrm]);
        app.update();
        assert_eq!(app.world().resource::<GraphRequests>().0, vec![vrm]);
    }

    #[test]
    fn batches_simultaneously_initialized_clips_per_model() {
        let mut app = App::new();
        app.init_resource::<GraphRequests>()
            .add_systems(Update, trigger_loaded)
            .add_observer(
                |event: On<RequestUpdateAnimationGraph>, mut requests: ResMut<GraphRequests>| {
                    requests.0.push(event.vrm);
                },
            );
        for _ in 0..2 {
            let vrm = app.world_mut().spawn(Initialized).id();
            for _ in 0..3 {
                app.world_mut().spawn((Vrma, Initialized, ChildOf(vrm)));
            }
        }
        app.update();
        let requests = &app.world().resource::<GraphRequests>().0;
        assert_eq!(requests.len(), 2);
        assert_ne!(requests[0], requests[1]);
    }

    #[test]
    fn a_vrma_is_initialized_and_its_rests_are_captured_once_its_bones_are_spawned() {
        use crate::prelude::{Hips, HipsBoneEntity};
        use crate::vrm::VrmBone;

        let mut app = App::new();
        app.add_systems(Update, request_initialize_vrma);
        let registry =
            HumanoidBoneRegistry::from_pairs([(VrmBone("hips".to_owned()), Name::new("hips"))]);
        let vrma = app
            .world_mut()
            .spawn((
                Vrma,
                registry,
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();

        app.update();
        assert!(app.world().get::<Initialized>(vrma).is_none());

        let hips = app
            .world_mut()
            .spawn((
                Name::new("hips"),
                Transform::from_xyz(0.0, 1.0, 0.0),
                GlobalTransform::from_xyz(0.0, 1.0, 0.0),
                ChildOf(vrma),
            ))
            .id();
        app.update();
        assert!(
            app.world().get::<Initialized>(vrma).is_some(),
            "with every bone spawned the VRMA is initialized and the graph can be built"
        );
        assert!(
            app.world().get::<RestTransform>(hips).is_some()
                && app.world().get::<RestGlobalTransform>(hips).is_some(),
            "the source bone's rest pose is captured, or the retarget moves no curve"
        );
        assert_eq!(
            app.world()
                .get::<HipsBoneEntity>(vrma)
                .map(|holder| holder.0),
            Some(hips),
            "the `.vrma` holds its bones, as a `.vrm` does"
        );
        assert!(app.world().get::<Hips>(hips).is_some());
    }
}
