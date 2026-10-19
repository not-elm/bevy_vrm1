use crate::prelude::{
    ChildSearcher, Initialized, RestGlobalTransform, RestTransform, RestWorldTransform, VrmBone,
    Vrma,
};
use crate::vrm::Vrm;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::instance::VrmGeneration;
use crate::vrma::animation::bake::{bake_rotation_curve, bake_translation_curve};
use crate::vrma::animation::bone_rotation::{
    RetargetRotationTable, compute_rotation_transformations, strip_world_prefix,
};
use crate::vrma::animation::bone_translation::{
    RetargetTranslationTable, compute_hips_transformation,
};
use crate::vrma::animation::expressions::{VrmaExpressionRegistry, retarget_expression_curves};
use crate::vrma::animation::mask::mask_group_for_bone;
use crate::vrma::{
    LoadedVrma, VrmAnimationClipHandle, VrmAnimationGraphState, VrmAnimationNodeIndex,
    VrmAnimationSourceClip, VrmaLoaded, VrmaNeedsRestore, VrmaPlaybackState, VrmaRetargetPending,
    VrmaRetargetedFor,
};
use bevy::animation::{AnimationTargetId, animated_field};
use bevy::app::App;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

/// Marker: VRMA clip needs baking.
#[derive(Component)]
pub(crate) struct NeedsBake;

/// Bakes in `PostUpdate`, so a rig attached in `SpawnScene` is baked before it is evaluated.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VrmaBake;

#[derive(Event)]
pub(crate) struct RequestUpdateAnimationGraph {
    pub(crate) vrm: Entity,
}

#[derive(EntityEvent)]
struct RequestUpdateAnimationClips(Entity);

pub(super) struct VrmaAnimationGraphPlugin;

impl Plugin for VrmaAnimationGraphPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_observer(apply_animation_graph)
            .add_observer(apply_retarget_animation_clips)
            .add_systems(Update, (retry_pending_retargets, reconcile_vrma_membership))
            .add_systems(PostUpdate, apply_bake_clips.in_set(VrmaBake));
    }
}

/// Rebuilds the graph when the VRMA children differ from the clips bound to it.
fn reconcile_vrma_membership(
    mut commands: Commands,
    states: Query<(Entity, Option<&VrmAnimationGraphState>), (With<Vrm>, With<Initialized>)>,
    childrens: Query<&Children>,
    ready_vrmas: Query<(), (With<Vrma>, With<Initialized>)>,
    graphs: Res<Assets<AnimationGraph>>,
) {
    for (vrm, state) in states.iter() {
        let live: HashSet<Entity> = childrens
            .get(vrm)
            .into_iter()
            .flatten()
            .copied()
            .filter(|child| ready_vrmas.get(*child).is_ok())
            .collect();
        let bound: HashSet<Entity> = match state {
            None => HashSet::new(),
            Some(state) => {
                let Some(graph) = graphs.get(&state.graph) else {
                    continue;
                };
                state
                    .nodes
                    .iter()
                    .filter_map(|(entity, node)| {
                        graph
                            .get(*node)
                            .is_some_and(|node| {
                                matches!(node.node_type, AnimationNodeType::Clip(_))
                            })
                            .then_some(*entity)
                    })
                    .collect()
            }
        };
        if live != bound {
            commands.trigger(RequestUpdateAnimationGraph { vrm });
        }
    }
}

/// Rebuilds the graph while a retarget is pending and the rig is reachable.
fn retry_pending_retargets(
    mut commands: Commands,
    child_searcher: ChildSearcher,
    pending: Query<&ChildOf, (With<VrmaRetargetPending>, Without<NeedsBake>)>,
) {
    let mut requested = HashSet::new();
    for child_of in &pending {
        let vrm = child_of.parent();
        if child_searcher.find_root_bone(vrm).is_some() && requested.insert(vrm) {
            commands.trigger(RequestUpdateAnimationGraph { vrm });
        }
    }
}

fn apply_animation_graph(
    trigger: On<RequestUpdateAnimationGraph>,
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut clips: ResMut<Assets<AnimationClip>>,
    childrens: Query<&Children>,
    generations: Query<&VrmGeneration>,
    mut vrmas: Query<
        (
            Option<&VrmAnimationSourceClip>,
            Option<&VrmAnimationClipHandle>,
            Option<&VrmaRetargetedFor>,
            Option<&VrmAnimationNodeIndex>,
            Option<&mut VrmaPlaybackState>,
        ),
        (With<Vrma>, With<Initialized>),
    >,
    mut players: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
    states: Query<&VrmAnimationGraphState>,
    child_searcher: ChildSearcher,
    bones: Query<(&VrmBone, &AnimationTargetId)>,
) {
    let vrm_entity = trigger.vrm;
    let Ok(children) = childrens.get(vrm_entity) else {
        return;
    };
    let existing = states.get(vrm_entity).ok().cloned();
    if existing.is_none() && !children.iter().any(|child| vrmas.get(child).is_ok()) {
        return;
    }
    let generation = generations
        .get(vrm_entity)
        .map_or(0, |generation| generation.0);

    // The graph is reused so every `VrmAnimationNodeIndex` stays valid.
    let graph_handle = match &existing {
        Some(state) => state.graph.clone(),
        None => graphs.add(AnimationGraph::new()),
    };

    let root_bone = child_searcher.find_root_bone(vrm_entity);
    let new_rig = match (existing.as_ref(), root_bone) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(state), Some(root)) => state.root_bone != root,
    };

    // Snapshot the playback of the rig being replaced while it still exists.
    let mut stored_transitions = existing
        .as_ref()
        .map(|state| state.transitions.clone())
        .unwrap_or_default();
    if new_rig
        && let Some(old_root) = existing.as_ref().map(|state| state.root_bone)
        && old_root != Entity::PLACEHOLDER
        && Some(old_root) != root_bone
    {
        if let Ok(old_player) = players.get(old_root) {
            for child in children.iter() {
                let Ok((_, _, _, Some(old_index), Some(mut state))) = vrmas.get_mut(child) else {
                    continue;
                };
                match old_player.animation(old_index.0) {
                    Some(active) => {
                        state.active = true;
                        state.animation = Some(*active);
                    }
                    None => {
                        state.active = false;
                        state.animation = None;
                    }
                }
            }
        }
        if let Ok(old_transitions) = transitions.get(old_root) {
            stored_transitions = (*old_transitions).clone();
        }
    }
    if let Some(root_bone) = root_bone {
        if new_rig {
            if let Ok(mut live) = transitions.get_mut(root_bone) {
                *live = stored_transitions.clone();
            }
        } else if let Ok(live) = transitions.get(root_bone) {
            stored_transitions = live.clone();
        }
    }
    insert_animation_graph_into_root_bone(
        vrm_entity,
        AnimationGraphHandle(graph_handle.clone()),
        &mut commands,
        &child_searcher,
    );

    let Some(mut graph) = graphs.get_mut(&graph_handle) else {
        return;
    };
    register_mask_groups(&mut graph, &bones);
    let root = graph.root;

    let mut nodes = existing
        .as_ref()
        .map(|state| state.nodes.clone())
        .unwrap_or_default();
    let live_children: HashSet<Entity> = children
        .iter()
        .filter(|child| vrmas.get(*child).is_ok())
        .collect();

    // A VRMA that left this VRM retires its node: it is kept so ids never
    // shift, but disconnected, made inert and its clip released.
    for (retired, node) in existing
        .as_ref()
        .map(|state| state.nodes.iter())
        .into_iter()
        .flatten()
    {
        if live_children.contains(retired) || graph.get(*node).is_none() {
            continue;
        }
        graph.remove_edge(root, *node);
        if let Some(node_weight) = graph.get_mut(*node) {
            node_weight.node_type = AnimationNodeType::Blend;
            node_weight.weight = 0.0;
        }
        if let Some(root_bone) = root_bone
            && let Ok(mut player) = players.get_mut(root_bone)
        {
            player.stop(*node);
        }
    }

    for child in children.iter() {
        let Ok((source, working, retargeted, _, _)) = vrmas.get_mut(child) else {
            continue;
        };
        let retarget =
            retargeted.is_none_or(|stamp| stamp.0 != generation || stamp.1 != vrm_entity);
        // A clip retargeted for another rig is re-cloned from the pristine source.
        let stale = retargeted.is_some_and(|stamp| stamp.0 != generation || stamp.1 != vrm_entity);
        let handle = if stale || working.is_none() {
            match source {
                Some(source) => {
                    let Some(pristine) = clips.get(&source.0).cloned() else {
                        continue;
                    };
                    let handle = clips.add(pristine);
                    commands
                        .entity(child)
                        .insert(VrmAnimationClipHandle(handle.clone()));
                    handle
                }
                None => match working {
                    Some(working) => working.0.clone(),
                    None => continue,
                },
            }
        } else {
            let Some(working) = working else {
                continue;
            };
            working.0.clone()
        };

        let owned = nodes
            .get(&child)
            .copied()
            .filter(|node| graph.get(*node).is_some());
        let appended = owned.is_none();
        let node = match owned {
            Some(owned) => owned,
            None => {
                let node = graph.add_clip(handle.clone(), 1.0, root);
                nodes.insert(child, node);
                node
            }
        };
        let was_retired = graph
            .get(node)
            .is_some_and(|node| !matches!(node.node_type, AnimationNodeType::Clip(_)));
        if was_retired {
            graph.add_edge(root, node);
        }
        if let Some(node_weight) = graph.get_mut(node) {
            if was_retired {
                node_weight.weight = 1.0;
            }
            node_weight.node_type = AnimationNodeType::Clip(handle);
        }
        commands.entity(child).insert(VrmAnimationNodeIndex(node));

        if retarget {
            commands.entity(child).insert(VrmaRetargetPending);
            commands.entity(child).trigger(RequestUpdateAnimationClips);
        }
        if new_rig || appended || was_retired {
            commands.entity(child).insert(VrmaNeedsRestore);
        }
    }

    commands.entity(vrm_entity).insert(VrmAnimationGraphState {
        graph: graph_handle.clone(),
        transitions: stored_transitions,
        root_bone: root_bone.unwrap_or(Entity::PLACEHOLDER),
        nodes,
    });
}

/// Puts every humanoid bone's [`AnimationTargetId`] into its `VrmMaskGroup`.
fn register_mask_groups(
    graph: &mut AnimationGraph,
    bones: &Query<(&VrmBone, &AnimationTargetId)>,
) {
    for (bone, target) in bones.iter() {
        graph.add_target_to_mask_group(*target, mask_group_for_bone(&bone.0) as u32);
    }
}

fn insert_animation_graph_into_root_bone(
    vrm: Entity,
    animation_graph_handle: AnimationGraphHandle,
    commands: &mut Commands,
    searcher: &ChildSearcher,
) {
    let Some(root_bone) = searcher.find_root_bone(vrm) else {
        return;
    };
    commands.entity(root_bone).insert(animation_graph_handle);
}

/// Retargets the humanoid bone curves and the expression tracks of one VRMA clip.
fn apply_retarget_animation_clips(
    trigger: On<RequestUpdateAnimationClips>,
    mut commands: Commands,
    mut clips: ResMut<Assets<AnimationClip>>,
    clip_handles: Query<&VrmAnimationClipHandle>,
    parents: Query<&ChildOf>,
    vrms: Query<&HumanoidBoneRegistry>,
    expression_registries: Query<&VrmaExpressionRegistry>,
    bones: Query<(&RestTransform, &RestGlobalTransform, &AnimationTargetId)>,
    model_rests: Query<&RestWorldTransform>,
    source_targets: Query<&AnimationTargetId>,
    generations: Query<&VrmGeneration>,
    searcher: ChildSearcher,
) {
    let vrma_entity = trigger.event_target();
    let Ok(ChildOf(vrm_entity)) = parents.get(vrma_entity) else {
        return;
    };
    let Ok(vrm_animation_clip_handle) = clip_handles.get(vrma_entity) else {
        return;
    };
    let Some(root_bone) = searcher.find_root_bone(*vrm_entity) else {
        return;
    };
    // AnimationClip is already cloned per VRMA during initialization.
    let Some(mut clip) = clips.get_mut(vrm_animation_clip_handle.0.id()) else {
        return;
    };
    if let Ok(registry) = vrms.get(vrma_entity) {
        let transformations = compute_rotation_transformations(
            vrma_entity,
            *vrm_entity,
            root_bone,
            registry,
            &searcher,
            &bones,
            &model_rests,
        );
        for (bone_entity, vrma_entity, transformation) in transformations {
            commands
                .entity(bone_entity)
                .entry::<RetargetRotationTable>()
                .and_modify(move |mut table| {
                    table.0.insert(vrma_entity, transformation);
                })
                .or_insert(RetargetRotationTable(HashMap::from([(
                    vrma_entity,
                    transformation,
                )])));
        }
        replace_bone_animation_clips(
            &mut commands,
            &mut clip,
            vrma_entity,
            *vrm_entity,
            root_bone,
            registry,
            &searcher,
            &bones,
            &model_rests,
        );
    }
    if let Ok(expressions) = expression_registries.get(vrma_entity) {
        retarget_expression_curves(
            &mut clip,
            vrma_entity,
            expressions,
            &searcher,
            &source_targets,
        );
    }
    // Recorded only after the curves were moved, so a failed retarget is retried.
    let generation = generations
        .get(*vrm_entity)
        .map_or(0, |generation| generation.0);
    commands
        .entity(vrma_entity)
        .insert((VrmaRetargetedFor(generation, *vrm_entity), NeedsBake));
    commands.entity(vrma_entity).remove::<VrmaRetargetPending>();
}

fn replace_bone_animation_clips(
    commands: &mut Commands,
    clip: &mut AnimationClip,
    vrma_entity: Entity,
    vrm_entity: Entity,
    root_bone: Entity,
    registry: &HumanoidBoneRegistry,
    searcher: &ChildSearcher,
    bones: &Query<(&RestTransform, &RestGlobalTransform, &AnimationTargetId)>,
    model_rests: &Query<&RestWorldTransform>,
) {
    let src_prefix = model_rests.get(vrma_entity).ok();
    let dist_prefix = model_rests.get(vrm_entity).ok();
    let animation_curves = clip.curves_mut();
    for (bone, name) in registry.iter() {
        let Some(vrma_bone_entity) = searcher.find_from_name(vrma_entity, name) else {
            continue;
        };
        let Some(bone_entity) = searcher.find_by_bone_name(root_bone, bone) else {
            continue;
        };
        let Ok((src_rest_tf, src_rest_gtf, vrma_bone_target)) = bones.get(vrma_bone_entity) else {
            continue;
        };
        let Ok((dist_rest_tf, dist_rest_gtf, bone_target)) = bones.get(bone_entity) else {
            continue;
        };
        if bone.as_str() == "hips" {
            // The hips height scaling must compare model-frame rest
            // positions; world-frame ones would bake in the body's
            // placement at each snapshot moment.
            let src_rest_g = strip_world_prefix(src_rest_gtf, src_prefix);
            let dist_rest_g = strip_world_prefix(dist_rest_gtf, dist_prefix);
            let hips_tf = compute_hips_transformation(
                src_rest_tf,
                src_rest_g.translation,
                dist_rest_tf,
                dist_rest_g.translation,
            );
            commands
                .entity(bone_entity)
                .entry::<RetargetTranslationTable>()
                .and_modify(move |mut table| {
                    table.0.insert(vrma_entity, hips_tf);
                })
                .or_insert(RetargetTranslationTable(HashMap::from([(
                    vrma_entity,
                    hips_tf,
                )])));
        }
        if let Some(curves) = animation_curves.remove(vrma_bone_target) {
            animation_curves.insert(*bone_target, curves);
        }
    }
}

fn apply_bake_clips(world: &mut World) {
    let mut to_bake: Vec<(Entity, Handle<AnimationClip>)> = Vec::new();
    {
        let mut query =
            world.query_filtered::<(Entity, &VrmAnimationClipHandle), With<NeedsBake>>();
        for (entity, clip_handle) in query.iter(world) {
            to_bake.push((entity, clip_handle.0.clone()));
        }
    }
    if to_bake.is_empty() {
        return;
    }

    let rotation_field = animated_field!(Transform::rotation);
    let EvaluatorId::ComponentField(rotation_component) = rotation_field.evaluator_id() else {
        return;
    };
    let rotation_component = *rotation_component;
    let translation_field = animated_field!(Transform::translation);
    let EvaluatorId::ComponentField(translation_component) = translation_field.evaluator_id()
    else {
        return;
    };
    let translation_component = *translation_component;

    for (vrma_entity, clip_handle) in to_bake {
        // Get the clip data first; only remove NeedsBake after successful fetch
        let Some(clip) = world
            .resource::<Assets<AnimationClip>>()
            .get(clip_handle.id())
            .cloned()
        else {
            continue;
        };
        // AnimationTargetIds are shared by instances of the same model.
        // Resolve targets only in the rig that owns this VRMA.
        let Some(vrm_entity) = world
            .get::<ChildOf>(vrma_entity)
            .map(|child_of| child_of.parent())
        else {
            continue;
        };
        let Some(own_rig) = collect_own_rig_targets(world, vrm_entity) else {
            continue;
        };
        world.entity_mut(vrma_entity).remove::<NeedsBake>();

        let mut new_curves: HashMap<AnimationTargetId, Vec<VariableCurve>> = HashMap::new();

        for (target_id, variable_curves) in clip.curves().iter() {
            let bone_entity = own_rig.get(target_id).copied();

            let mut baked_curves = Vec::new();
            for vc in variable_curves.iter() {
                let mut baked = false;
                // Do not apply a source-space bone curve to a destination
                // bone when its per-clip retarget transformation is missing.
                let mut drop_curve = false;

                if let Some(bone) = bone_entity
                    && let EvaluatorId::ComponentField(target) = vc.0.evaluator_id()
                {
                    if *target == rotation_component {
                        let transformation = world
                            .get::<RetargetRotationTable>(bone)
                            .and_then(|table| table.0.get(&vrma_entity).cloned());
                        if transformation.is_none() {
                            #[cfg(feature = "log")]
                            warn!(
                                "[VRMA bake] dropped rotation curve for {target_id:?} \
                                 (vrma {vrma_entity:?}): no RetargetRotationTable entry"
                            );
                            drop_curve = true;
                        }
                        if let Some(transformation) = transformation
                            && let Some(baked_vc) = bake_rotation_curve(vc, &transformation, world)
                        {
                            baked_curves.push(baked_vc);
                            baked = true;
                        }
                    } else if *target == translation_component {
                        let transformation = world
                            .get::<RetargetTranslationTable>(bone)
                            .and_then(|table| table.0.get(&vrma_entity).cloned());
                        if transformation.is_none() {
                            #[cfg(feature = "log")]
                            warn!(
                                "[VRMA bake] dropped translation curve for {target_id:?} \
                                 (vrma {vrma_entity:?}): no RetargetTranslationTable entry"
                            );
                            drop_curve = true;
                        }
                        if let Some(transformation) = transformation
                            && let Some(baked_vc) =
                                bake_translation_curve(vc, &transformation, world)
                        {
                            baked_curves.push(baked_vc);
                            baked = true;
                        }
                    }
                }

                if !baked && !drop_curve {
                    baked_curves.push(vc.clone());
                }
            }
            new_curves.insert(*target_id, baked_curves);
        }

        // Replace the clip's curves with baked ones
        let mut clip_assets = world.resource_mut::<Assets<AnimationClip>>();
        if let Some(mut clip) = clip_assets.get_mut(clip_handle.id()) {
            let curves = clip.curves_mut();
            curves.clear();
            for (target_id, variable_curves) in new_curves {
                for vc in variable_curves {
                    curves.entry(target_id).or_default().push(vc);
                }
            }
        }

        // Emitted once per VRMA, after its clip is baked.
        if world.get::<VrmaLoaded>(vrma_entity).is_none() {
            world.entity_mut(vrma_entity).insert(VrmaLoaded);
            world.trigger(LoadedVrma {
                vrm: vrm_entity,
                vrma: vrma_entity,
            });
        }
    }
}

/// Maps animation targets only within the owning VRM's bone-root subtree.
fn collect_own_rig_targets(
    world: &World,
    vrm_entity: Entity,
) -> Option<HashMap<AnimationTargetId, Entity>> {
    // ChildSearcher is a system param and cannot be used from an exclusive
    // system, so the root bone is found by name here the same way.
    // A managed VRM is searched from its scene root, like `ChildSearcher`.
    let search_root = match world.get::<crate::vrm::instance::VrmInstanceState>(vrm_entity) {
        Some(instance) => instance.scene_root,
        None if world
            .get::<crate::vrm::instance::VrmHandle>(vrm_entity)
            .is_some() =>
        {
            return None;
        }
        None => vrm_entity,
    };
    let root_bone = find_entity_by_name(world, search_root, Vrm::ROOT_BONE)?;
    let mut targets = HashMap::default();
    let mut stack = vec![root_bone];
    while let Some(entity) = stack.pop() {
        if let Some(target) = world.get::<AnimationTargetId>(entity) {
            targets.insert(*target, entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    Some(targets)
}

fn find_entity_by_name(
    world: &World,
    entity: Entity,
    name: &str,
) -> Option<Entity> {
    if world
        .get::<Name>(entity)
        .is_some_and(|n| n.as_str() == name)
    {
        return Some(entity);
    }
    let children = world.get::<Children>(entity)?;
    children
        .iter()
        .find_map(|child| find_entity_by_name(world, child, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{
        Initialized, LoadedVrma, PlayVrma, VrmBone, Vrma, vrm_root_animation_target,
    };
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::extensions::VrmNode;
    use crate::vrm::gltf::handler::bone_target_id;
    use crate::vrm::instance::VrmGeneration;
    use crate::vrma::animation::mask::{VrmMaskGroup, VrmaMask};
    use crate::vrma::animation::play::VrmaAnimationPlayPlugin;
    use crate::vrma::{
        RetargetSource, VrmAnimationGraphState, VrmaPlaybackState, VrmaRetargetPending,
    };
    use bevy::animation::graph::{AnimationMask, AnimationNodeType};
    use bevy::animation::{
        ActiveAnimation, AnimatedBy, AnimationEntityMut, AnimationPlugin, RepeatAnimation,
    };
    use bevy::app::SpawnScene;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::gltf::GltfNode;
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct LoadNotifications(Vec<Entity>);

    fn setup_app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<AnimationClip>>()
            .init_resource::<Assets<AnimationGraph>>()
            .init_resource::<LoadNotifications>()
            .add_plugins(VrmaAnimationGraphPlugin)
            .add_observer(
                |event: On<LoadedVrma>,
                 indices: Query<&VrmAnimationNodeIndex>,
                 mut notifications: ResMut<LoadNotifications>| {
                    assert!(
                        indices.get(event.vrma).is_ok(),
                        "LoadedVrma must allow playing the clip"
                    );
                    notifications.0.push(event.vrma);
                },
            );
        app
    }

    fn spawn_rig(
        world: &mut World,
        rest_rotation: Quat,
    ) -> Entity {
        let prefix = GlobalTransform::from(
            Transform::from_xyz(3.0, 2.0, -1.0).with_rotation(Quat::from_rotation_y(0.3)),
        );
        let vrm = world.spawn((Initialized, RestWorldTransform(prefix))).id();
        let root = world.spawn((Name::new(Vrm::ROOT_BONE), ChildOf(vrm))).id();
        let rest = Transform::from_rotation(rest_rotation);
        world.spawn((
            VrmBone::from("spine"),
            AnimationTargetId::from_name(&Name::new("destination_spine")),
            RestTransform(rest),
            RestGlobalTransform(prefix.mul_transform(rest)),
            ChildOf(root),
        ));
        vrm
    }

    /// The destination avatar as `handler::scene` writes it.
    /// Returns `(vrm, root_bone, spine)`.
    fn spawn_pipeline_rig(
        world: &mut World,
        rest_rotation: Quat,
    ) -> (Entity, Entity, Entity) {
        let prefix = GlobalTransform::from(
            Transform::from_xyz(3.0, 2.0, -1.0).with_rotation(Quat::from_rotation_y(0.3)),
        );
        let vrm = world
            .spawn((Vrm, Initialized, RestWorldTransform(prefix)))
            .id();
        let root_bone = world
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                Transform::default(),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                RetargetSource,
                ChildOf(vrm),
            ))
            .id();
        world
            .entity_mut(vrm)
            .insert((vrm_root_animation_target(), AnimatedBy(root_bone)));
        let rest = Transform::from_rotation(rest_rotation);
        let spine = world
            .spawn((
                VrmBone::from("spine"),
                bone_target_id("spine"),
                AnimatedBy(root_bone),
                RetargetSource,
                Transform::default(),
                RestTransform(rest),
                RestGlobalTransform(prefix.mul_transform(rest)),
                ChildOf(root_bone),
            ))
            .id();
        (vrm, root_bone, spine)
    }

    #[test]
    fn a_hand_built_pipeline_root_is_accepted_by_the_graph_build() -> TestResult {
        let mut app = setup_app();
        let (vrm, root_bone, spine) = spawn_pipeline_rig(app.world_mut(), Quat::IDENTITY);
        let (vrma, clip) = spawn_clip(app.world_mut(), vrm, 0.5);

        request_graph(&mut app, vrm);

        assert!(app.world().get::<AnimationGraphHandle>(root_bone).is_some());
        assert!(app.world().get::<AnimationPlayer>(root_bone).is_some());
        let node = app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .unwrap()
            .clone();
        assert!(matches!(
            app.world()
                .resource::<Assets<AnimationGraph>>()
                .get(&graph)
                .unwrap()
                .graph
                .node_weight(node)
                .unwrap()
                .node_type,
            AnimationNodeType::Clip(_)
        ));
        assert!(app.world().get::<RetargetRotationTable>(spine).is_some());
        let curves = app
            .world()
            .resource::<Assets<AnimationClip>>()
            .get(&clip)
            .unwrap()
            .curves();
        assert!(
            curves.contains_key(&bone_target_id("spine")),
            "the clip must address the avatar by VRM bone name"
        );
        assert_eq!(app.world().resource::<LoadNotifications>().0, vec![vrma]);
        success!()
    }

    #[test]
    fn a_masked_node_does_not_move_a_bone_in_its_group() -> TestResult {
        let mut app = crate::tests::test_app();
        app.add_plugins((AnimationPlugin, VrmaAnimationGraphPlugin));
        let (vrm, root_bone, spine) = spawn_pipeline_rig(app.world_mut(), Quat::IDENTITY);
        let (vrma, _) = spawn_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);

        let node = app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph handle is on the root bone")
            .clone();
        assert_eq!(
            mask_group_for_bone("spine"),
            VrmMaskGroup::UpperBody,
            "the fixture's bone is in the upper body"
        );

        // Masked: the layer must not touch the spine.
        set_node_mask(&mut app, &graph, node, VrmaMask::LOWER_BODY);
        play(&mut app, root_bone, node);
        run_frames(&mut app);
        assert_eq!(
            app.world().get::<Transform>(spine).unwrap().rotation,
            Quat::IDENTITY,
            "a masked node must not move a bone in its mask group"
        );

        // Unmasked: the very same node now drives it.
        set_node_mask(&mut app, &graph, node, VrmaMask::ALL);
        run_frames(&mut app);
        let rotation = app.world().get::<Transform>(spine).unwrap().rotation;
        assert!(
            rotation.angle_between(Quat::from_rotation_x(0.5)) < 0.001,
            "an unmasked node must drive the bone, got {rotation:?}"
        );
        success!()
    }

    /// Sets one clip node's mask-out bitfield.
    fn set_node_mask(
        app: &mut App,
        graph: &AnimationGraphHandle,
        node: AnimationNodeIndex,
        mask: AnimationMask,
    ) {
        let mut graphs = app.world_mut().resource_mut::<Assets<AnimationGraph>>();
        let mut graph = graphs.get_mut(graph).expect("the graph asset exists");
        graph
            .graph
            .node_weight_mut(node)
            .expect("the clip node exists")
            .mask = mask;
    }

    /// Starts a clip without a transition.
    fn play(
        app: &mut App,
        root_bone: Entity,
        node: AnimationNodeIndex,
    ) {
        app.world_mut()
            .entity_mut(root_bone)
            .get_mut::<AnimationPlayer>()
            .expect("the root bone holds the player")
            .play(node);
    }

    /// Three frames, so the evaluation is guaranteed to see the updated graph.
    fn run_frames(app: &mut App) {
        for _ in 0..3 {
            app.update();
        }
    }

    fn spawn_clip(
        world: &mut World,
        vrm: Entity,
        angle: f32,
    ) -> (Entity, Handle<AnimationClip>) {
        let source_target = AnimationTargetId::from_name(&Name::new("source_spine"));
        let mut clip = AnimationClip::default();
        let pose = Quat::from_rotation_x(angle);
        clip.add_curve_to_target(
            source_target,
            AnimatableCurve::new(
                animated_field!(Transform::rotation),
                AnimatableKeyframeCurve::new([(0.0, pose), (1.0, pose)]).unwrap(),
            ),
        );
        let handle = world.resource_mut::<Assets<AnimationClip>>().add(clip);
        let mut nodes = Assets::<GltfNode>::default();
        let node = nodes.add(GltfNode {
            index: 0,
            name: "source_spine".into(),
            children: vec![],
            mesh: None,
            skin: None,
            transform: Transform::default(),
            is_animation_root: false,
            extras: None,
        });
        let registry = HumanoidBoneRegistry::new(
            &HashMap::from([("spine".into(), VrmNode { node: 0 })]),
            &nodes,
            &[node],
        );
        // The source is initialized after a different application placement.
        let prefix = GlobalTransform::from(
            Transform::from_xyz(-2.0, 8.0, 4.0).with_rotation(Quat::from_rotation_y(1.8)),
        );
        let vrma = world
            .spawn((
                Vrma,
                Initialized,
                registry,
                RestWorldTransform(prefix),
                VrmAnimationClipHandle(handle.clone()),
                ChildOf(vrm),
            ))
            .id();
        world.spawn((
            Name::new("source_spine"),
            source_target,
            RestTransform::default(),
            RestGlobalTransform(prefix),
            ChildOf(vrma),
        ));
        (vrma, handle)
    }

    /// Returns `(vrm, spine, prefix)`.
    fn spawn_generation_rig(
        world: &mut World,
        rest_rotation: Quat,
        generation: u64,
    ) -> (Entity, Entity, GlobalTransform) {
        let prefix = GlobalTransform::from(
            Transform::from_xyz(3.0, 2.0, -1.0).with_rotation(Quat::from_rotation_y(0.3)),
        );
        let vrm = world
            .spawn((
                Initialized,
                RestWorldTransform(prefix),
                VrmGeneration(generation),
            ))
            .id();
        let root = world.spawn((Name::new(Vrm::ROOT_BONE), ChildOf(vrm))).id();
        let rest = Transform::from_rotation(rest_rotation);
        let spine = world
            .spawn((
                VrmBone::from("spine"),
                AnimationTargetId::from_name(&Name::new("destination_spine")),
                RestTransform(rest),
                RestGlobalTransform(prefix.mul_transform(rest)),
                ChildOf(root),
            ))
            .id();
        (vrm, spine, prefix)
    }

    /// Returns `(vrma, source, working)`.
    fn spawn_sourced_clip(
        world: &mut World,
        vrm: Entity,
        angle: f32,
    ) -> (Entity, Handle<AnimationClip>, Handle<AnimationClip>) {
        let source_target = AnimationTargetId::from_name(&Name::new("source_spine"));
        let mut clip = AnimationClip::default();
        let pose = Quat::from_rotation_x(angle);
        clip.add_curve_to_target(
            source_target,
            AnimatableCurve::new(
                animated_field!(Transform::rotation),
                AnimatableKeyframeCurve::new([(0.0, pose), (1.0, pose)]).unwrap(),
            ),
        );
        let source = world
            .resource_mut::<Assets<AnimationClip>>()
            .add(clip.clone());
        let working = world.resource_mut::<Assets<AnimationClip>>().add(clip);
        let mut nodes = Assets::<GltfNode>::default();
        let node = nodes.add(GltfNode {
            index: 0,
            name: "source_spine".into(),
            children: vec![],
            mesh: None,
            skin: None,
            transform: Transform::default(),
            is_animation_root: false,
            extras: None,
        });
        let registry = HumanoidBoneRegistry::new(
            &HashMap::from([("spine".into(), VrmNode { node: 0 })]),
            &nodes,
            &[node],
        );
        let prefix = GlobalTransform::from(
            Transform::from_xyz(-2.0, 8.0, 4.0).with_rotation(Quat::from_rotation_y(1.8)),
        );
        let vrma = world
            .spawn((
                Vrma,
                Initialized,
                registry,
                RestWorldTransform(prefix),
                VrmAnimationSourceClip(source.clone()),
                VrmAnimationClipHandle(working.clone()),
                ChildOf(vrm),
            ))
            .id();
        world.spawn((
            Name::new("source_spine"),
            source_target,
            RestTransform::default(),
            RestGlobalTransform(prefix),
            ChildOf(vrma),
        ));
        (vrma, source, working)
    }

    fn request_graph(
        app: &mut App,
        vrm: Entity,
    ) {
        app.world_mut().trigger(RequestUpdateAnimationGraph { vrm });
        app.world_mut().flush();
        app.update();
    }

    fn sample_rotation(
        world: &mut World,
        handle: &Handle<AnimationClip>,
    ) -> Quat {
        let target = AnimationTargetId::from_name(&Name::new("destination_spine"));
        let clip = world
            .resource::<Assets<AnimationClip>>()
            .get(handle)
            .unwrap();
        let curves = clip
            .curves()
            .get(&target)
            .expect("curve must target the destination bone");
        assert_eq!(curves.len(), 1);
        let curve = curves[0].clone();
        let mut evaluator = curve.0.create_evaluator();
        let entity = world.spawn(Transform::default()).id();
        curve
            .0
            .apply(&mut *evaluator, 0.5, 1.0, AnimationNodeIndex::new(0))
            .unwrap();
        let mut query = world.query::<AnimationEntityMut>();
        evaluator
            .commit(query.get_mut(world, entity).unwrap())
            .unwrap();
        let rotation = world.get::<Transform>(entity).unwrap().rotation;
        world.despawn(entity);
        rotation
    }

    #[test]
    fn graph_retargets_every_clip_and_bakes_only_its_own_rig() {
        let mut app = setup_app();
        let rest_a = Quat::from_rotation_z(0.4);
        let rest_b = Quat::from_rotation_z(-0.6);
        let a = spawn_rig(app.world_mut(), rest_a);
        let b = spawn_rig(app.world_mut(), rest_b);
        let (_, clip_a1) = spawn_clip(app.world_mut(), a, 0.5);
        let (_, clip_a2) = spawn_clip(app.world_mut(), a, 0.9);
        let (_, clip_b) = spawn_clip(app.world_mut(), b, 0.5);
        request_graph(&mut app, a);
        request_graph(&mut app, b);
        assert_eq!(app.world().resource::<LoadNotifications>().0.len(), 3);
        for (clip, angle, rest) in [
            (clip_a1, 0.5, rest_a),
            (clip_a2, 0.9, rest_a),
            (clip_b, 0.5, rest_b),
        ] {
            let actual = sample_rotation(app.world_mut(), &clip);
            let expected = Quat::from_rotation_x(angle) * rest;
            assert!(actual.angle_between(expected) < 0.001);
        }
    }

    #[test]
    fn adding_a_clip_does_not_retarget_existing_baked_curves_again() {
        let mut app = setup_app();
        let rest = Quat::from_rotation_z(0.4);
        let vrm = spawn_rig(app.world_mut(), rest);
        let (first, first_clip) = spawn_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);
        let before = sample_rotation(app.world_mut(), &first_clip);
        let first_index = app.world().get::<VrmAnimationNodeIndex>(first).unwrap().0;
        let (second, second_clip) = spawn_clip(app.world_mut(), vrm, 0.9);
        request_graph(&mut app, vrm);
        assert_eq!(
            app.world().resource::<LoadNotifications>().0,
            vec![first, second]
        );
        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(first).unwrap().0,
            first_index
        );
        assert!(sample_rotation(app.world_mut(), &first_clip).angle_between(before) < 0.001);
        let expected = Quat::from_rotation_x(0.9) * rest;
        assert!(sample_rotation(app.world_mut(), &second_clip).angle_between(expected) < 0.001);
    }

    #[test]
    fn a_reloaded_rig_rebakes_from_the_pristine_source() {
        let mut app = setup_app();
        let rest_a = Quat::from_rotation_z(0.4);
        let rest_b = Quat::from_rotation_z(-0.6);
        let (vrm, spine, prefix) = spawn_generation_rig(app.world_mut(), rest_a, 1);
        let (vrma, _source, working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);
        assert!(
            sample_rotation(app.world_mut(), &working)
                .angle_between(Quat::from_rotation_x(0.5) * rest_a)
                < 0.001,
            "the first bake is the first rig's retarget"
        );

        let rest = Transform::from_rotation(rest_b);
        app.world_mut().entity_mut(spine).insert((
            RestTransform(rest),
            RestGlobalTransform(prefix.mul_transform(rest)),
        ));
        app.world_mut().entity_mut(vrm).insert(VrmGeneration(2));
        request_graph(&mut app, vrm);

        let reloaded = app
            .world()
            .get::<VrmAnimationClipHandle>(vrma)
            .expect("the reloaded clip handle")
            .0
            .clone();
        assert_ne!(
            reloaded.id(),
            working.id(),
            "a new generation must re-clone the working clip from the source"
        );
        let actual = sample_rotation(app.world_mut(), &reloaded);
        assert!(
            actual.angle_between(Quat::from_rotation_x(0.5) * rest_b) < 0.001,
            "the reload must bake the new rig's rest only, got {actual:?}"
        );
    }

    #[test]
    fn a_reload_does_not_resignal_loaded_vrma() {
        let mut app = setup_app();
        let (vrm, _spine, _prefix) =
            spawn_generation_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (vrma, _source, _working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);
        assert_eq!(app.world().resource::<LoadNotifications>().0, vec![vrma]);

        app.world_mut().entity_mut(vrm).insert(VrmGeneration(2));
        request_graph(&mut app, vrm);
        assert_eq!(
            app.world().resource::<LoadNotifications>().0,
            vec![vrma],
            "a reattachment must not re-emit LoadedVrma"
        );
    }

    #[test]
    fn a_vrma_that_arrives_after_the_rig_joins_the_current_graph() {
        let mut app = setup_app();
        let (vrm, _spine, _prefix) =
            spawn_generation_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        request_graph(&mut app, vrm);
        assert!(app.world().resource::<LoadNotifications>().0.is_empty());

        let (vrma, _source, working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);

        assert!(
            app.world().get::<VrmAnimationNodeIndex>(vrma).is_some(),
            "the late clip joins the graph"
        );
        assert_eq!(app.world().resource::<LoadNotifications>().0, vec![vrma]);
        let actual = sample_rotation(app.world_mut(), &working);
        assert!(
            actual.angle_between(Quat::from_rotation_x(0.5) * Quat::from_rotation_z(0.4)) < 0.001
        );
    }

    #[test]
    fn a_reload_of_one_avatar_leaves_another_untouched() {
        let mut app = setup_app();
        let (a, _sa, _pa) = spawn_generation_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (b, _sb, _pb) = spawn_generation_rig(app.world_mut(), Quat::from_rotation_z(-0.6), 1);
        let (vrma_a, _src_a, work_a) = spawn_sourced_clip(app.world_mut(), a, 0.5);
        let (vrma_b, _src_b, work_b) = spawn_sourced_clip(app.world_mut(), b, 0.9);
        request_graph(&mut app, a);
        request_graph(&mut app, b);
        let before_b = sample_rotation(app.world_mut(), &work_b);
        let index_b = app.world().get::<VrmAnimationNodeIndex>(vrma_b).unwrap().0;

        app.world_mut().entity_mut(a).insert(VrmGeneration(2));
        request_graph(&mut app, a);

        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(vrma_b).unwrap().0,
            index_b,
            "the other avatar's node index is untouched"
        );
        assert!(
            sample_rotation(app.world_mut(), &work_b).angle_between(before_b) < 0.001,
            "the other avatar's baked clip is untouched"
        );
        let reloaded_a = app
            .world()
            .get::<VrmAnimationClipHandle>(vrma_a)
            .unwrap()
            .0
            .clone();
        assert_ne!(
            reloaded_a.id(),
            work_a.id(),
            "the reloaded avatar did get a new working clip"
        );
    }

    /// An app in which dropped assets are really freed.
    fn setup_tracked_app() -> App {
        let mut app = crate::tests::test_app();
        app.add_plugins((AnimationPlugin, VrmaAnimationGraphPlugin));
        app
    }

    #[test]
    fn repeated_reloads_do_not_grow_derived_assets() {
        let mut app = setup_tracked_app();
        let (vrm, _spine, _prefix) =
            spawn_generation_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (_vrma, _source, _) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);
        // Give the asset events of the first replacement time to drain.
        for _ in 0..3 {
            app.update();
        }
        let clips = app.world().resource::<Assets<AnimationClip>>().len();
        let graphs = app.world().resource::<Assets<AnimationGraph>>().len();
        assert!(clips >= 2, "source and working clips exist");

        for generation in 2..6 {
            app.world_mut()
                .entity_mut(vrm)
                .insert(VrmGeneration(generation));
            request_graph(&mut app, vrm);
            for _ in 0..3 {
                app.update();
            }
        }

        assert_eq!(
            app.world().resource::<Assets<AnimationClip>>().len(),
            clips,
            "working clips must not accumulate across reloads"
        );
        assert_eq!(
            app.world().resource::<Assets<AnimationGraph>>().len(),
            graphs,
            "graphs must not accumulate across reloads"
        );
    }

    /// Returns `(vrm, root_bone, spine, prefix)`.
    fn spawn_live_rig(
        world: &mut World,
        rest_rotation: Quat,
        generation: u64,
    ) -> (Entity, Entity, Entity, GlobalTransform) {
        let prefix = GlobalTransform::from(
            Transform::from_xyz(3.0, 2.0, -1.0).with_rotation(Quat::from_rotation_y(0.3)),
        );
        let vrm = world
            .spawn((
                Vrm,
                Initialized,
                RestWorldTransform(prefix),
                VrmGeneration(generation),
            ))
            .id();
        let root_bone = world
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                Transform::default(),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                RetargetSource,
                ChildOf(vrm),
            ))
            .id();
        world
            .entity_mut(vrm)
            .insert((vrm_root_animation_target(), AnimatedBy(root_bone)));
        let rest = Transform::from_rotation(rest_rotation);
        let spine = world
            .spawn((
                VrmBone::from("spine"),
                AnimationTargetId::from_name(&Name::new("destination_spine")),
                AnimatedBy(root_bone),
                RetargetSource,
                Transform::default(),
                RestTransform(rest),
                RestGlobalTransform(prefix.mul_transform(rest)),
                ChildOf(root_bone),
            ))
            .id();
        (vrm, root_bone, spine, prefix)
    }

    #[test]
    fn an_inactive_clips_mask_survives_a_rebuild() -> TestResult {
        let mut app = crate::tests::test_app();
        app.add_plugins((
            AnimationPlugin,
            VrmaAnimationGraphPlugin,
            VrmaAnimationPlayPlugin,
        ));
        let (vrm, root_bone, _spine, _prefix) =
            spawn_live_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (vrma, _source, _working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);

        request_graph(&mut app, vrm);

        let node = app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph is on the root bone")
            .0
            .clone();
        // The application masks the node while it is not playing.
        app.world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .get_mut(&graph)
            .unwrap()
            .graph
            .node_weight_mut(node)
            .unwrap()
            .mask = 0b101;

        // A second rebuild for the same rig.
        request_graph(&mut app, vrm);

        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0,
            node,
            "the rebuilt clip keeps its node id"
        );
        assert_eq!(
            app.world()
                .resource::<Assets<AnimationGraph>>()
                .get(&graph)
                .unwrap()
                .graph
                .node_weight(node)
                .unwrap()
                .mask,
            0b101,
            "a persistent graph keeps an inactive clip's mask"
        );
        success!()
    }

    #[test]
    fn a_failed_retarget_does_not_record_the_generation() {
        let mut app = setup_app();
        // A VRM with no root bone: the retarget cannot reach a rig.
        let vrm = app.world_mut().spawn(Initialized).id();
        let (vrma, _source, _working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);
        assert!(
            app.world().get::<VrmaRetargetedFor>(vrma).is_none(),
            "the generation is recorded only after a real retarget"
        );
        assert!(
            app.world().get::<NeedsBake>(vrma).is_none(),
            "and nothing is scheduled to bake either"
        );
        assert!(
            app.world().get::<VrmaRetargetPending>(vrma).is_some(),
            "the pending marker keeps the retry alive"
        );
        assert!(
            app.world().get::<VrmaNeedsRestore>(vrma).is_some(),
            "and the restore waits for the retarget"
        );
    }

    #[test]
    fn a_reattach_to_a_hand_built_generation_zero_rig_rebakes() -> TestResult {
        let mut app = setup_app();
        let rest_a = Quat::from_rotation_z(0.4);
        let (vrm, spine, prefix) = spawn_generation_rig(app.world_mut(), rest_a, 1);
        let (vrma, _source, _working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);
        assert!(app.world().get::<VrmaRetargetedFor>(vrma).is_some());

        let rest_b = Quat::from_rotation_z(-0.6);
        app.world_mut().entity_mut(vrm).remove::<VrmGeneration>();
        let rest = Transform::from_rotation(rest_b);
        app.world_mut().entity_mut(spine).insert((
            RestTransform(rest),
            RestGlobalTransform(prefix.mul_transform(rest)),
        ));
        request_graph(&mut app, vrm);

        let reloaded = app
            .world()
            .get::<VrmAnimationClipHandle>(vrma)
            .expect("the reloaded clip handle")
            .0
            .clone();
        let sampled = sample_rotation(app.world_mut(), &reloaded);
        assert!(
            sampled.angle_between(Quat::from_rotation_x(0.5) * rest_b) < 0.001,
            "a generation-0 fallback rig still forces a retarget, got {sampled:?}"
        );
        success!()
    }

    /// A model swap run in `SpawnScene`, after `Update`.
    #[derive(Resource)]
    struct ReloadPlan {
        vrm: Entity,
        spine: Entity,
        prefix: GlobalTransform,
        rest: Quat,
        armed: bool,
    }

    fn reload_in_spawn_scene(world: &mut World) {
        let (vrm, spine, prefix, rest) = {
            let Some(mut plan) = world.get_resource_mut::<ReloadPlan>() else {
                return;
            };
            if !plan.armed {
                return;
            }
            plan.armed = false;
            (
                plan.vrm,
                plan.spine,
                plan.prefix,
                Transform::from_rotation(plan.rest),
            )
        };
        world.entity_mut(spine).insert((
            RestTransform(rest),
            RestGlobalTransform(prefix.mul_transform(rest)),
        ));
        world.entity_mut(vrm).insert(VrmGeneration(2));
        world.trigger(RequestUpdateAnimationGraph { vrm });
    }

    #[test]
    fn a_rebuild_in_spawn_scene_bakes_before_the_next_evaluation() -> TestResult {
        let mut app = crate::tests::test_app();
        app.add_plugins((
            AnimationPlugin,
            VrmaAnimationGraphPlugin,
            VrmaAnimationPlayPlugin,
        ));
        let rest_a = Quat::from_rotation_z(0.4);
        let rest_b = Quat::from_rotation_z(-0.6);
        let (vrm, _root_bone, spine, prefix) = spawn_live_rig(app.world_mut(), rest_a, 1);
        let (vrma, _source, _working) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        // The application had the clip playing before the swap.
        app.world_mut().entity_mut(vrma).insert(VrmaPlaybackState {
            active: true,
            animation: Some(ActiveAnimation::default()),
        });
        // The first, valid rig is baked and playing.
        request_graph(&mut app, vrm);

        // The swap attaches in `SpawnScene`, after `Update`.
        app.insert_resource(ReloadPlan {
            vrm,
            spine,
            prefix,
            rest: rest_b,
            armed: true,
        });
        app.add_systems(SpawnScene, reload_in_spawn_scene);
        app.update();

        assert!(
            app.world().get::<NeedsBake>(vrma).is_none(),
            "a rebuild must bake before the frame's evaluation"
        );
        let reloaded = app
            .world()
            .get::<VrmAnimationClipHandle>(vrma)
            .expect("the reloaded clip handle")
            .0
            .clone();
        let sampled = sample_rotation(app.world_mut(), &reloaded);
        assert!(
            sampled.angle_between(Quat::from_rotation_x(0.5) * rest_b) < 0.001,
            "the clip is baked for the new rest in the rebuild frame, got {sampled:?}"
        );

        app.update();
        let rotation = app.world().get::<Transform>(spine).unwrap().rotation;
        assert!(
            rotation.angle_between(Quat::from_rotation_x(0.5) * rest_b) < 0.01,
            "the first evaluated frame uses the baked clip, got {rotation:?}"
        );
        success!()
    }

    #[test]
    fn adding_a_vrma_at_the_front_does_not_shift_existing_node_indices() -> TestResult {
        let mut app = setup_app();
        let (vrm, root_bone, _spine, _prefix) =
            spawn_live_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (vrma_a, _source_a, _working_a) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        request_graph(&mut app, vrm);

        let index_a = app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph is on the root bone")
            .0
            .clone();

        let (vrma_b, _source_b, _working_b) = spawn_sourced_clip(app.world_mut(), vrm, 0.9);
        app.world_mut()
            .entity_mut(vrm)
            .insert_children(0, &[vrma_b]);
        request_graph(&mut app, vrm);

        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0,
            index_a,
            "an existing clip keeps its node id when a new one is inserted before it"
        );
        let index_b = app
            .world()
            .get::<VrmAnimationNodeIndex>(vrma_b)
            .expect("the new clip joins the graph")
            .0;
        assert_ne!(index_b, index_a, "the new clip gets a fresh node id");
        assert_eq!(
            app.world()
                .get::<AnimationGraphHandle>(root_bone)
                .unwrap()
                .0,
            graph,
            "the same graph asset is reused"
        );
        let assets = app.world().resource::<Assets<AnimationGraph>>();
        let graph_asset = assets.get(&graph).unwrap();
        for index in [index_a, index_b] {
            assert!(matches!(
                graph_asset.graph.node_weight(index).unwrap().node_type,
                AnimationNodeType::Clip(_)
            ));
        }
        success!()
    }

    #[test]
    fn removing_a_vrma_releases_its_clip_and_keeps_the_sibling_node() -> TestResult {
        let mut app = setup_tracked_app();
        let (vrm, root_bone, _spine, _prefix) =
            spawn_live_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (vrma_a, _, _) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        let (vrma_b, _, _) = spawn_sourced_clip(app.world_mut(), vrm, 0.9);
        request_graph(&mut app, vrm);
        for _ in 0..3 {
            app.update();
        }

        let node_a = app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0;
        let node_b = app.world().get::<VrmAnimationNodeIndex>(vrma_b).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph is on the root bone")
            .0
            .clone();
        let nodes_before = app
            .world()
            .resource::<Assets<AnimationGraph>>()
            .get(&graph)
            .unwrap()
            .graph
            .node_count();

        // Remove the second `.vrma` and rebuild repeatedly.
        app.world_mut().entity_mut(vrma_b).despawn();
        for _ in 0..4 {
            request_graph(&mut app, vrm);
            for _ in 0..3 {
                app.update();
            }
        }

        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0,
            node_a,
            "the sibling clip keeps its node id"
        );
        {
            let assets = app.world().resource::<Assets<AnimationGraph>>();
            let graph_asset = assets.get(&graph).unwrap();
            assert_eq!(
                graph_asset.graph.node_count(),
                nodes_before,
                "node ids must never shift, so nodes are never removed"
            );
            assert!(
                matches!(
                    graph_asset.graph.node_weight(node_b).unwrap().node_type,
                    AnimationNodeType::Blend
                ),
                "the retired node is inert and holds no clip"
            );
        }

        let clips = app.world().resource::<Assets<AnimationClip>>().len();
        for _ in 0..4 {
            request_graph(&mut app, vrm);
            for _ in 0..3 {
                app.update();
            }
        }
        assert_eq!(
            app.world().resource::<Assets<AnimationClip>>().len(),
            clips,
            "a retired clip must not stay alive in the graph"
        );
        success!()
    }

    #[test]
    fn despawning_a_vrma_without_a_manual_rebuild_releases_its_clip() -> TestResult {
        let mut app = setup_tracked_app();
        let (vrm, root_bone, _spine, _prefix) =
            spawn_live_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (vrma_a, _, _) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        let (vrma_b, _, _) = spawn_sourced_clip(app.world_mut(), vrm, 0.9);
        request_graph(&mut app, vrm);
        for _ in 0..3 {
            app.update();
        }

        let node_a = app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0;
        let node_b = app.world().get::<VrmAnimationNodeIndex>(vrma_b).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph is on the root bone")
            .0
            .clone();
        app.world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .get_mut(&graph)
            .unwrap()
            .graph
            .node_weight_mut(node_a)
            .unwrap()
            .mask = 0b101;
        let clips_before = app.world().resource::<Assets<AnimationClip>>().len();

        // No manual rebuild: only the reconciliation can notice the despawn.
        app.world_mut().entity_mut(vrma_b).despawn();
        for _ in 0..6 {
            app.update();
        }

        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0,
            node_a,
            "the surviving clip keeps its node id"
        );
        {
            let assets = app.world().resource::<Assets<AnimationGraph>>();
            let graph_asset = assets.get(&graph).unwrap();
            assert!(
                matches!(
                    graph_asset.graph.node_weight(node_b).unwrap().node_type,
                    AnimationNodeType::Blend
                ),
                "a despawn is retired without a manual rebuild"
            );
            assert_eq!(
                graph_asset.graph.node_weight(node_a).unwrap().mask,
                0b101,
                "the surviving clip's mask is untouched"
            );
        }
        assert!(
            app.world().resource::<Assets<AnimationClip>>().len() < clips_before,
            "the removed clip's handles are released"
        );
        success!()
    }

    #[test]
    fn a_readded_vrma_preserves_its_mask_and_reconnects_its_node() -> TestResult {
        let mut app = crate::tests::test_app();
        app.add_plugins((
            AnimationPlugin,
            VrmaAnimationGraphPlugin,
            VrmaAnimationPlayPlugin,
        ));
        let rest = Quat::from_rotation_z(0.4);
        let (vrm, root_bone, spine, _prefix) = spawn_live_rig(app.world_mut(), rest, 1);
        let (vrma, _, _) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        app.world_mut()
            .entity_mut(vrma)
            .insert(VrmaPlaybackState::default());
        request_graph(&mut app, vrm);

        let node = app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0;
        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph is on the root bone")
            .0
            .clone();

        set_node_mask(
            &mut app,
            &AnimationGraphHandle(graph.clone()),
            node,
            VrmaMask::LOWER_BODY,
        );

        // The `.vrma` leaves: its node retires (disconnected and inert).
        app.world_mut().entity_mut(vrma).remove::<Vrma>();
        request_graph(&mut app, vrm);
        {
            let assets = app.world().resource::<Assets<AnimationGraph>>();
            assert!(
                matches!(
                    assets
                        .get(&graph)
                        .unwrap()
                        .graph
                        .node_weight(node)
                        .unwrap()
                        .node_type,
                    AnimationNodeType::Blend
                ),
                "the removed clip's node is retired"
            );
        }

        // It comes back with the same body mask, so the spine stays untouched.
        app.world_mut().entity_mut(vrma).insert(Vrma);
        request_graph(&mut app, vrm);
        assert_eq!(
            app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0,
            node,
            "the readded clip reuses its node id"
        );
        assert_eq!(
            app.world()
                .resource::<Assets<AnimationGraph>>()
                .get(&graph)
                .unwrap()
                .get(node)
                .unwrap()
                .mask,
            VrmaMask::LOWER_BODY,
            "the readded clip keeps its body mask"
        );
        play_vrma(app.world_mut(), vrma, Duration::ZERO);
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            app.world().get::<Transform>(spine).unwrap().rotation,
            Quat::IDENTITY,
            "the preserved mask still excludes the spine"
        );

        // Removing the mask lets the reconnected edge drive the spine again.
        set_node_mask(&mut app, &AnimationGraphHandle(graph), node, VrmaMask::ALL);
        run_frames(&mut app);
        let rotation = app.world().get::<Transform>(spine).unwrap().rotation;
        assert!(
            rotation.angle_between(Quat::from_rotation_x(0.5) * rest) < 0.01,
            "the reconnected clip plays again, got {rotation:?}"
        );
        success!()
    }

    #[test]
    fn reparenting_a_vrma_reclones_for_the_new_owner() -> TestResult {
        let mut app = setup_app();
        let rest_a = Quat::from_rotation_z(0.4);
        let rest_b = Quat::from_rotation_z(-0.6);
        let (a, root_a, _spine_a, _prefix_a) = spawn_live_rig(app.world_mut(), rest_a, 1);
        let (b, root_b, _spine_b, _prefix_b) = spawn_live_rig(app.world_mut(), rest_b, 1);
        let (vrma, _source, working_a) = spawn_sourced_clip(app.world_mut(), a, 0.5);
        request_graph(&mut app, a);

        let node_a = app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0;
        assert!(
            sample_rotation(app.world_mut(), &working_a)
                .angle_between(Quat::from_rotation_x(0.5) * rest_a)
                < 0.001,
            "the first owner bakes its own rest"
        );

        // Reparent under `b`, whose generation is also `1`.
        app.world_mut().entity_mut(vrma).insert(ChildOf(b));
        request_graph(&mut app, b);
        request_graph(&mut app, a);

        let graph_a = app
            .world()
            .get::<AnimationGraphHandle>(root_a)
            .expect("a keeps its graph")
            .0
            .clone();
        let graph_b = app
            .world()
            .get::<AnimationGraphHandle>(root_b)
            .expect("b got a graph")
            .0
            .clone();
        let node_b = app.world().get::<VrmAnimationNodeIndex>(vrma).unwrap().0;
        {
            let assets = app.world().resource::<Assets<AnimationGraph>>();
            assert!(
                matches!(
                    assets
                        .get(&graph_a)
                        .unwrap()
                        .graph
                        .node_weight(node_a)
                        .unwrap()
                        .node_type,
                    AnimationNodeType::Blend
                ),
                "the old owner retires the clip it no longer owns"
            );
            assert!(
                matches!(
                    assets
                        .get(&graph_b)
                        .unwrap()
                        .graph
                        .node_weight(node_b)
                        .unwrap()
                        .node_type,
                    AnimationNodeType::Clip(_)
                ),
                "the new owner binds its own clip node"
            );
        }
        let stamp = app
            .world()
            .get::<VrmaRetargetedFor>(vrma)
            .copied()
            .expect("the clip is retargeted for the new owner");
        assert_eq!(stamp.1, b, "the stamp names the new owner");

        let working_b = app
            .world()
            .get::<VrmAnimationClipHandle>(vrma)
            .expect("the new owner's working clip")
            .0
            .clone();
        assert_ne!(
            working_b.id(),
            working_a.id(),
            "a new owner forces a re-clone despite the same generation"
        );
        assert!(
            sample_rotation(app.world_mut(), &working_b)
                .angle_between(Quat::from_rotation_x(0.5) * rest_b)
                < 0.001,
            "the re-clone is baked for the new rig"
        );
        success!()
    }

    /// Replaces the root bone subtree with a fresh one. Returns `(root_bone, spine)`.
    fn reattach_rig(
        world: &mut World,
        vrm: Entity,
        rest_rotation: Quat,
    ) -> (Entity, Entity) {
        if let Some(old_root) = find_entity_by_name(world, vrm, Vrm::ROOT_BONE) {
            world.entity_mut(old_root).despawn();
        }
        let prefix = world
            .get::<RestWorldTransform>(vrm)
            .map(|rest| rest.0)
            .unwrap_or_default();
        let root_bone = world
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                Transform::default(),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                RetargetSource,
                ChildOf(vrm),
            ))
            .id();
        world
            .entity_mut(vrm)
            .insert((vrm_root_animation_target(), AnimatedBy(root_bone)));
        let rest = Transform::from_rotation(rest_rotation);
        let spine = world
            .spawn((
                VrmBone::from("spine"),
                AnimationTargetId::from_name(&Name::new("destination_spine")),
                AnimatedBy(root_bone),
                RetargetSource,
                Transform::default(),
                RestTransform(rest),
                RestGlobalTransform(prefix.mul_transform(rest)),
                ChildOf(root_bone),
            ))
            .id();
        (root_bone, spine)
    }

    /// Plays a `.vrma` through the production event, with a controllable fade.
    fn play_vrma(
        world: &mut World,
        vrma: Entity,
        transition_duration: Duration,
    ) {
        world
            .commands()
            .entity(vrma)
            .trigger(move |entity| PlayVrma {
                vrma: entity,
                repeat: RepeatAnimation::Never,
                transition_duration,
                reset_spring_bones: false,
            });
    }

    #[test]
    fn a_reattach_carries_the_transitions_and_the_next_play_fades_the_restored_clip() -> TestResult
    {
        let mut app = crate::tests::test_app();
        app.add_plugins((
            AnimationPlugin,
            VrmaAnimationGraphPlugin,
            VrmaAnimationPlayPlugin,
        ));
        let (vrm, _root_bone, _spine, _prefix) =
            spawn_live_rig(app.world_mut(), Quat::from_rotation_z(0.4), 1);
        let (vrma_a, _source_a, _working_a) = spawn_sourced_clip(app.world_mut(), vrm, 0.5);
        let (vrma_b, _source_b, _working_b) = spawn_sourced_clip(app.world_mut(), vrm, 0.9);
        for vrma in [vrma_a, vrma_b] {
            app.world_mut()
                .entity_mut(vrma)
                .insert(VrmaPlaybackState::default());
        }

        request_graph(&mut app, vrm);
        let node_a = app.world().get::<VrmAnimationNodeIndex>(vrma_a).unwrap().0;
        let node_b = app.world().get::<VrmAnimationNodeIndex>(vrma_b).unwrap().0;

        play_vrma(app.world_mut(), vrma_a, Duration::ZERO);
        app.update();

        assert_eq!(
            app.world()
                .get::<VrmAnimationGraphState>(vrm)
                .unwrap()
                .transitions
                .get_main_animation(),
            Some(node_a),
            "the snapshot keeps the live transitions"
        );

        let (new_root, _new_spine) =
            reattach_rig(app.world_mut(), vrm, Quat::from_rotation_z(-0.6));
        app.world_mut().entity_mut(vrm).insert(VrmGeneration(2));
        request_graph(&mut app, vrm);

        assert_eq!(
            app.world()
                .get::<AnimationTransitions>(new_root)
                .unwrap()
                .get_main_animation(),
            Some(node_a),
            "the new root bone inherits the old main animation"
        );
        assert!(
            app.world()
                .get::<AnimationPlayer>(new_root)
                .unwrap()
                .animation(node_a)
                .is_some(),
            "the restored clip is playing on the new player"
        );

        play_vrma(app.world_mut(), vrma_b, Duration::from_millis(300));
        app.update();
        assert_eq!(
            app.world()
                .get::<AnimationTransitions>(new_root)
                .unwrap()
                .get_main_animation(),
            Some(node_b)
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_millis(150));
        app.world_mut()
            .run_system_once(advance_transitions)
            .unwrap();
        let faded = app
            .world()
            .get::<AnimationPlayer>(new_root)
            .unwrap()
            .animation(node_a)
            .expect("the restored clip is still fading out")
            .weight();
        assert!(
            faded < 1.0,
            "the restored clip is faded by the next play, weight {faded}"
        );
        success!()
    }
}
