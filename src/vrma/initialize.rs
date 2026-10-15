//! This module turns a loaded [`VrmaHandle`] into a playable source rig: it
//! instantiates the `.vrma` scene, keeps the pristine clip, and stamps the
//! source rest pose. It also tells the animation graph to rebuild whenever the
//! avatar it is parented to finishes an attach — including a rig replacement,
//! where the graph, the retarget and the playback are restored onto the new
//! skeleton.

use crate::error::vrm_error;
use crate::system_param::prelude::*;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::{Initialized, RestGlobalTransform, RestTransform, RestWorldTransform, Vrm};
use crate::vrma::animation::animation_graph::RequestUpdateAnimationGraph;
use crate::vrma::animation::expressions::VrmaExpressionRegistry;
use crate::vrma::gltf::extensions::VrmaExtensions;
use crate::vrma::loader::VrmaAsset;
use crate::vrma::{
    RetargetSource, VrmAnimationClipHandle, VrmAnimationSourceClip, Vrma, VrmaDuration, VrmaHandle,
    VrmaPath, VrmaPlaybackState,
};
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

/// Every complete attach of a rig is observed through `Insert<Initialized>` on
/// the entity carrying [`Vrm`], and asks for the animation graph to be rebuilt.
///
/// On a pre-existing VRMA the graph build sees a rig generation its working
/// clip was not baked for, clones the pristine
/// [`VrmAnimationSourceClip`](crate::vrma::VrmAnimationSourceClip) again,
/// retargets it for the new rig and restores the recorded playback. The first
/// attach of a fresh avatar reaches this too, but its VRMA children are usually
/// still loading and `trigger_loaded` requests the graph again once they are
/// ready.
///
/// `Initialized` is also stamped on a `.vrma`'s own source-rig entity by
/// [`request_initialize_vrma`]; the [`Vrm`] filter excludes those.
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
        let Some(source_clip_handle) = vrma.gltf.animations.first() else {
            vrm_error!("[VRMA] Not found vrma animations in {name}");
            continue;
        };
        let Some(animation_clip) = clip_assets.get(source_clip_handle).cloned() else {
            vrm_error!("[VRMA] Not found animation clip for {name}");
            continue;
        };
        let working_clip_handle = clip_assets.add(animation_clip);
        commands.entity(handle_entity).insert((
            Vrma,
            Name::new(name),
            // The pristine source stays for the entity's whole life so a rig
            // replacement can rebuild the working clip from it; the working
            // clone is what the retarget and bake mutate.
            VrmAnimationSourceClip(source_clip_handle.clone()),
            VrmAnimationClipHandle(working_clip_handle),
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

/// Stamps [`Initialized`] on a `.vrma` entity once its source rig has spawned,
/// and captures the source rig's rest pose.
///
/// A `.vrm` is initialized while it loads, so the avatar root already carries the
/// marker and the pipeline writes its rest transforms into the scene. A `.vrma`
/// still builds its bones through the scene it instantiates and nothing writes
/// those components, so without this the VRMA retarget finds every source bone
/// without a `(RestTransform, RestGlobalTransform)` pair, silently moves no curve
/// and no `.vrma` ever plays.
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
        // The application placement, captured together with the bones' rests:
        // VRM and VRMA may initialize under different world transforms.
        if let Ok((_, global)) = transforms.get(root) {
            commands.entity(root).insert(RestWorldTransform(*global));
        }
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
        }
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
    Duration::from_secs_f64(duration)
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

    /// The regression that stopped every `.vrma` from playing: nothing stamped
    /// `Initialized` on a `.vrma` entity and nothing captured its source rig's
    /// rest transforms, so `trigger_loaded` never fired and the retarget found
    /// every source bone without a `(RestTransform, RestGlobalTransform)` pair.
    #[test]
    fn a_vrma_is_initialized_and_its_rests_are_captured_once_its_bones_are_spawned() {
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

        // The source rig has not arrived yet: the marker must not be stamped.
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
    }
}
