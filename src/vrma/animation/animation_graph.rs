use crate::prelude::{ChildSearcher, RestGlobalTransform, RestTransform, RestWorldTransform};
use crate::vrm::Vrm;
use crate::vrm::expressions::VrmExpressionRegistry;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrma::animation::bake::{bake_rotation_curve, bake_translation_curve};
use crate::vrma::animation::bone_rotation::{
    RetargetRotationTable, compute_rotation_transformations, strip_world_prefix,
};
use crate::vrma::animation::bone_translation::{
    RetargetTranslationTable, compute_hips_transformation,
};
use crate::vrma::{LoadedVrma, VrmAnimationClipHandle, VrmAnimationNodeIndex};
use bevy::animation::{AnimationTargetId, animated_field};
use bevy::app::App;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// Marker: VRMA clip needs baking.
#[derive(Component)]
pub(crate) struct NeedsBake;

/// Rebuilding a graph must not transform an already retargeted clip again.
#[derive(Component)]
struct ClipRetargetRequested;

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
            .add_observer(apply_replace_humanoid_bone_animation_clips)
            .add_observer(apply_regenerate_expression_clips)
            .add_systems(Update, apply_bake_clips);
    }
}

fn apply_animation_graph(
    trigger: On<RequestUpdateAnimationGraph>,
    mut commands: Commands,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    childrens: Query<&Children>,
    vrmas: Query<(Entity, &VrmAnimationClipHandle, Has<ClipRetargetRequested>)>,
    child_searcher: ChildSearcher,
    entities: Query<(Has<AnimationPlayer>, Option<&AnimationGraphHandle>)>,
) {
    let vrm_entity = trigger.vrm;
    let Ok(children) = childrens.get(vrm_entity) else {
        return;
    };
    let animation_graph = generate_animation_graph(&mut commands, &vrmas, children);
    let animation_graph_handle = AnimationGraphHandle(graphs.add(animation_graph));
    insert_animation_graph_into_root_bone(
        vrm_entity,
        animation_graph_handle.clone(),
        &mut commands,
        &child_searcher,
    );
    insert_animation_graph_into_expressions(
        trigger.vrm,
        &mut commands,
        &mut graphs,
        &animation_graph_handle,
        &entities,
        &child_searcher,
        &childrens,
    );
    // Fan out to every new clip, exactly once. Previous clips already contain
    // destination-space curves and must not be retargeted again on a rebuild.
    for child in children.iter() {
        if let Ok((_, _, false)) = vrmas.get(child) {
            commands
                .entity(child)
                .insert(ClipRetargetRequested)
                .trigger(RequestUpdateAnimationClips);
            // The graph and node indices must exist before observers can play
            // a clip in response to LoadedVrma.
            commands.trigger(LoadedVrma {
                vrm: vrm_entity,
                vrma: child,
            });
        }
    }
}

fn generate_animation_graph(
    commands: &mut Commands,
    vrmas_query: &Query<(Entity, &VrmAnimationClipHandle, Has<ClipRetargetRequested>)>,
    children: &Children,
) -> AnimationGraph {
    let vrmas = children
        .iter()
        .flat_map(|child| vrmas_query.get(child).ok())
        .collect::<Vec<_>>();
    let (graph, nodes) = AnimationGraph::from_clips(vrmas.iter().map(|(_, h, _)| h.0.clone()));
    for (i, (entity, _, _)) in vrmas.iter().enumerate() {
        commands
            .entity(*entity)
            .insert(VrmAnimationNodeIndex(nodes[i]));
    }
    graph
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

fn insert_animation_graph_into_expressions(
    entity: Entity,
    commands: &mut Commands,
    graphs: &mut Assets<AnimationGraph>,
    animation_graph_handle: &AnimationGraphHandle,
    expressions: &Query<(Has<AnimationPlayer>, Option<&AnimationGraphHandle>)>,
    searcher: &ChildSearcher,
    childrens: &Query<&Children>,
) {
    let Some(expressions_root) = searcher.find_expressions_root(entity) else {
        return;
    };
    let Ok(expression_children) = childrens.get(expressions_root) else {
        return;
    };
    for expression in expression_children.iter() {
        let Ok((has_player, previous_handle)) = expressions.get(expression) else {
            continue;
        };
        if let Some(previous_handle) = previous_handle {
            graphs.remove(previous_handle);
        }
        if has_player {
            commands
                .entity(expression)
                .insert(animation_graph_handle.clone());
        }
    }
}

fn apply_replace_humanoid_bone_animation_clips(
    trigger: On<RequestUpdateAnimationClips>,
    mut commands: Commands,
    mut clips: ResMut<Assets<AnimationClip>>,
    clip_handles: Query<&VrmAnimationClipHandle>,
    parents: Query<&ChildOf>,
    vrms: Query<&HumanoidBoneRegistry>,
    bones: Query<(&RestTransform, &RestGlobalTransform, &AnimationTargetId)>,
    model_rests: Query<&RestWorldTransform>,
    searcher: ChildSearcher,
) {
    let vrma_entity = trigger.event_target();
    let Ok(ChildOf(vrm_entity)) = parents.get(vrma_entity) else {
        return;
    };
    let Ok(registry) = vrms.get(vrma_entity) else {
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
    commands.entity(vrma_entity).insert(NeedsBake);
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
    }
}

/// Maps animation targets only within the owning VRM's bone-root subtree.
fn collect_own_rig_targets(
    world: &World,
    vrm_entity: Entity,
) -> Option<HashMap<AnimationTargetId, Entity>> {
    // ChildSearcher is a system param and cannot be used from an exclusive
    // system, so the root bone is found by name here the same way.
    let root_bone = find_entity_by_name(world, vrm_entity, Vrm::ROOT_BONE)?;
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

fn apply_regenerate_expression_clips(
    trigger: On<RequestUpdateAnimationClips>,
    mut clips: ResMut<Assets<AnimationClip>>,
    clip_handles: Query<&VrmAnimationClipHandle>,
    animation_targets: Query<&AnimationTargetId>,
    expressions: Query<&VrmExpressionRegistry>,
    searcher: ChildSearcher,
    parents: Query<&ChildOf>,
) {
    let vrma_entity = trigger.event_target();
    let Ok(vrm_entity) = parents.get(vrma_entity).map(|c| c.parent()) else {
        return;
    };
    let Some(expressions_root) = searcher.find_expressions_root(vrm_entity) else {
        return;
    };
    let Ok(vrm_animation_clip_handle) = clip_handles.get(vrma_entity) else {
        return;
    };
    let Some(mut clip) = clips.get_mut(vrm_animation_clip_handle.0.id()) else {
        return;
    };
    let Ok(registry) = expressions.get(vrm_entity) else {
        return;
    };
    for (expression, _) in registry.iter() {
        let Some(vrma_expression) = searcher.find_from_name(vrma_entity, expression) else {
            continue;
        };
        let Some(expression_entity) = searcher.find_from_name(expressions_root, expression) else {
            continue;
        };
        let Ok(vrma_target) = animation_targets.get(vrma_expression) else {
            continue;
        };
        let Ok(target) = animation_targets.get(expression_entity) else {
            continue;
        };
        let animation_curves = clip.curves_mut();
        if let Some(curves) = animation_curves.remove(vrma_target) {
            animation_curves.insert(*target, curves);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{Initialized, LoadedVrma, VrmBone, Vrma};
    use crate::vrm::gltf::extensions::VrmNode;
    use bevy::animation::AnimationEntityMut;
    use bevy::gltf::GltfNode;

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
}
