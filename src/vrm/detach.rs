use crate::vrm::body_tracking::{BodyTracking, SmoothedGaze};
use crate::vrm::components::{PendingNodeConstraint, VrmHeadOnly, VrmNodeConstraint, VrmNodeIndex};
use crate::vrm::instance::{
    PendingVrmScene, VrmGeneration, VrmHandle, VrmInstanceState, VrmSceneRoot, instance_entities,
    remove_model_name, remove_root_data,
};
use crate::vrm::look_at::LookAt;
use crate::vrm::spring_bone::{SpringJointProps, SpringJointState, SpringRoot};
use crate::vrm::{Initialized, RestGlobalTransform, RestTransform, Vrm, VrmBone};
use crate::vrma::{VrmAnimationGraphState, Vrma, VrmaHandle};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

/// Triggers VRM detachment on the target entity.
///
/// Removes all VRM-related components and despawns the child hierarchy,
/// leaving the root entity itself alive.
///
/// # Usage
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::vrm::detach::RequestDetachVrm;
/// fn detach(mut commands: Commands, vrm_entity: Entity) {
///     commands.entity(vrm_entity).trigger(RequestDetachVrm);
/// }
/// ```
#[derive(EntityEvent)]
pub struct RequestDetachVrm(pub Entity);

pub(crate) struct VrmDetachPlugin;

impl Plugin for VrmDetachPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_observer(apply_detach_vrm);
    }
}

fn apply_detach_vrm(
    trigger: On<RequestDetachVrm>,
    managed: Query<(), With<VrmHandle>>,
    vrm_root: Query<(), With<Vrm>>,
    raw_owner: Query<
        (),
        (
            With<WorldAssetRoot>,
            Without<VrmHandle>,
            Without<Vrma>,
            Without<VrmaHandle>,
        ),
    >,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    if managed.contains(entity) {
        commands.queue(move |world: &mut World| detach_managed(world, entity));
    } else if vrm_root.contains(entity) || raw_owner.contains(entity) {
        commands.queue(move |world: &mut World| detach_raw(world, entity));
    }
}

/// Detaches a [`VrmHandle`]-managed avatar, keeping the entity and the
/// application's own components.
fn detach_managed(
    world: &mut World,
    entity: Entity,
) {
    if world.get_entity(entity).is_err() {
        return;
    }
    if world.get::<VrmHandle>(entity).is_none() {
        return;
    }
    let tracked = world
        .get::<WorldInstance>(entity)
        .map(|instance| **instance)
        .map(|instance_id| instance_entities(world, instance_id))
        .unwrap_or_default();

    remove_root_data(world, entity);
    remove_model_name(world, entity);
    world.entity_mut(entity).remove::<WorldAssetRoot>();
    world
        .entity_mut(entity)
        .remove::<Vrm>()
        .remove::<Initialized>()
        .remove::<VrmGeneration>()
        .remove::<VrmInstanceState>()
        .remove::<PendingVrmScene>()
        .remove::<VrmHandle>()
        .remove::<WorldInstance>()
        .remove::<VrmAnimationGraphState>();
    despawn_children(world, entity);
    for tracked_entity in tracked {
        if tracked_entity == entity {
            continue;
        }
        if world.get_entity(tracked_entity).is_ok() {
            world.entity_mut(tracked_entity).despawn();
        }
    }
}

/// Detaches a raw `WorldAssetRoot` spawn, whether the request named the child
/// VRM root or the owner.
fn detach_raw(
    world: &mut World,
    entity: Entity,
) {
    if world.get_entity(entity).is_err() {
        return;
    }
    if world.get::<VrmHandle>(entity).is_some()
        || world.get::<Vrma>(entity).is_some()
        || world.get::<VrmaHandle>(entity).is_some()
    {
        return;
    }
    let entity_is_root = world.get::<Vrm>(entity).is_some();
    let entity_is_owner = world.get::<WorldAssetRoot>(entity).is_some();
    if !entity_is_root && !entity_is_owner {
        return;
    }
    let (owner, root) = if entity_is_root {
        let owner = if entity_is_owner {
            Some(entity)
        } else {
            find_raw_owner(world, entity)
        };
        (owner, Some(entity))
    } else {
        (Some(entity), find_raw_instance_root(world, entity))
    };

    let tracked = owner
        .and_then(|owner| world.get::<WorldInstance>(owner).map(|instance| **instance))
        .map(|instance_id| instance_entities(world, instance_id))
        .unwrap_or_default();
    let spawned = !tracked.is_empty();
    if root.is_none()
        && spawned
        && tracked
            .iter()
            .all(|&entity| world.get::<VrmSceneRoot>(entity).is_none())
    {
        return;
    }

    if let Some(owner) = owner
        && world.get_entity(owner).is_ok()
    {
        world.entity_mut(owner).remove::<WorldAssetRoot>();
    }
    if let Some(root) = root
        && world.get_entity(root).is_ok()
    {
        strip_raw_root(world, root);
    }
    let kept_root = root.filter(|_| entity_is_root);
    for tracked_entity in tracked {
        if Some(tracked_entity) == kept_root {
            continue;
        }
        if world.get_entity(tracked_entity).is_ok() {
            world.entity_mut(tracked_entity).despawn();
        }
    }
    if spawned
        && let Some(owner) = owner
        && world.get_entity(owner).is_ok()
    {
        world.entity_mut(owner).remove::<WorldInstance>();
    }
}

/// Finds the [`WorldAssetRoot`] owner of the raw instance that `root` belongs to.
fn find_raw_owner(
    world: &World,
    root: Entity,
) -> Option<Entity> {
    let spawner = world.get_resource::<WorldInstanceSpawner>();
    let mut current = root;
    while let Some(parent) = world.get::<ChildOf>(current).map(ChildOf::parent) {
        if world.get::<WorldAssetRoot>(parent).is_some() {
            match (
                spawner,
                world
                    .get::<WorldInstance>(parent)
                    .map(|instance| **instance),
            ) {
                (Some(spawner), Some(instance_id)) => {
                    if spawner
                        .iter_instance_entities(instance_id)
                        .any(|entity| entity == root)
                    {
                        return Some(parent);
                    }
                }
                (None, _) => return Some(parent),
                (Some(_), None) => {}
            }
        }
        current = parent;
    }
    None
}

/// Finds the [`Vrm`] root of `owner`'s live instance, if it has spawned.
fn find_raw_instance_root(
    world: &World,
    owner: Entity,
) -> Option<Entity> {
    let spawner = world.get_resource::<WorldInstanceSpawner>()?;
    let instance_id = world
        .get::<WorldInstance>(owner)
        .map(|instance| **instance)?;
    spawner
        .iter_instance_entities(instance_id)
        .find(|entity| world.get::<Vrm>(*entity).is_some())
}

/// Strips the VRM rig off a raw instance's scene root and despawns its children.
fn strip_raw_root(
    world: &mut World,
    root: Entity,
) {
    remove_root_data(world, root);
    world
        .entity_mut(root)
        .remove::<Vrm>()
        .remove::<Initialized>()
        .remove::<VrmSceneRoot>()
        .remove::<Name>()
        .remove::<WorldAssetRoot>()
        .remove::<RestTransform>()
        .remove::<RestGlobalTransform>()
        .remove::<LookAt>()
        .remove::<BodyTracking>()
        .remove::<SmoothedGaze>()
        .remove::<VrmNodeIndex>()
        .remove::<VrmBone>()
        .remove::<PendingNodeConstraint>()
        .remove::<VrmNodeConstraint>()
        .remove::<SpringRoot>()
        .remove::<SpringJointProps>()
        .remove::<SpringJointState>()
        .remove::<VrmHeadOnly>()
        .remove::<VrmAnimationGraphState>();
    despawn_children(world, root);
}

/// Despawns all descendants of the entity.
///
/// `EntityWorldMut::despawn_children` skips a child that something else (an
/// application observer reacting to an earlier despawn) already removed, where
/// despawning each child through `World::entity_mut` would panic.
fn despawn_children(
    world: &mut World,
    entity: Entity,
) {
    if let Ok(mut entity) = world.get_entity_mut(entity) {
        entity.despawn_children();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::components::{ConstraintExecutionOrder, VrmConstraintKind};
    use crate::vrm::expressions::{ExpressionCategory, ExpressionOverrideType};
    use crate::vrm::gltf::extensions::vrmc_vrm::LookAtProperties;
    use crate::vrm::humanoid_bone::prelude::HipsBoneEntity;
    use crate::vrm::runtime::apply_node_constraints;
    use crate::vrm::spring_bone::{SpringCenterNode, SpringColliders, SpringJoints};
    use crate::vrm::{RestWorldTransform, VrmPath};
    use crate::vrma::animation::prelude::{
        EffectiveExpressionWeights, ExpressionMorphBinds, ExpressionSetting, ExpressionSettings,
        MorphBindTable, VrmExpressionWeights,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::platform::collections::HashMap;
    use std::f32::consts::FRAC_PI_2;

    macro_rules! assert_all_removed {
        ($entity:expr, $($component:ty),+ $(,)?) => {
            $(
                assert!(
                    !$entity.contains::<$component>(),
                    "`{}` survived the detach",
                    stringify!($component)
                );
            )+
        };
    }

    fn setup_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(VrmDetachPlugin);
        app
    }

    fn trigger_detach(
        app: &mut App,
        entity: Entity,
    ) {
        app.world_mut()
            .commands()
            .entity(entity)
            .trigger(RequestDetachVrm);
        app.update();
    }

    fn run_constraints(app: &mut App) {
        app.world_mut()
            .run_system_once(apply_node_constraints)
            .expect("Failed to run apply_node_constraints");
        app.world_mut().flush();
    }

    fn set_rotation(
        app: &mut App,
        entity: Entity,
        rotation: Quat,
    ) {
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .expect("the entity has a Transform")
            .rotation = rotation;
    }

    fn rotation(
        app: &App,
        entity: Entity,
    ) -> Quat {
        app.world().get::<Transform>(entity).unwrap().rotation
    }

    /// `q` and `-q` are the same rotation, so compare up to sign.
    fn rotation_is(
        actual: Quat,
        expected: Quat,
    ) -> bool {
        actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5)
    }

    #[test]
    fn test_detach_removes_vrm_components() {
        let mut app = setup_app();

        let vrm_entity = app
            .world_mut()
            .spawn((Vrm, Initialized, RestWorldTransform::default()))
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        let world = app.world();
        assert!(!world.entity(vrm_entity).contains::<Vrm>());
        assert!(!world.entity(vrm_entity).contains::<Initialized>());
        assert!(!world.entity(vrm_entity).contains::<RestWorldTransform>());
        // Entity itself survives
        assert!(world.get_entity(vrm_entity).is_ok());
    }

    #[test]
    fn test_detach_despawns_children() {
        let mut app = setup_app();

        let child = app.world_mut().spawn_empty().id();
        let vrm_entity = app.world_mut().spawn(Vrm).id();
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .add_child(child);
        app.update();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Root survives
        assert!(app.world().get_entity(vrm_entity).is_ok());
        // Child is despawned
        assert!(app.world().get_entity(child).is_err());
    }

    #[test]
    fn test_detach_on_non_vrm_entity() {
        let mut app = setup_app();

        let child = app.world_mut().spawn_empty().id();
        let entity = app.world_mut().spawn(Name::new("not-a-vrm")).id();
        app.world_mut().commands().entity(entity).add_child(child);
        app.update();

        app.world_mut()
            .commands()
            .entity(entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Name and children should remain since this isn't a VRM entity
        assert!(app.world().entity(entity).contains::<Name>());
        assert!(app.world().get_entity(child).is_ok());
    }

    #[test]
    fn test_detach_idempotent() {
        let mut app = setup_app();

        let vrm_entity = app.world_mut().spawn(Vrm).id();

        // First detach
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        // Second detach — should not panic
        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestDetachVrm);
        app.update();

        assert!(app.world().get_entity(vrm_entity).is_ok());
    }

    #[test]
    fn the_load_time_component_set_is_removed() -> TestResult {
        let mut app = setup_app();
        let source = app.world_mut().spawn_empty().id();
        let root = app.world_mut().spawn_empty().id();

        app.world_mut().entity_mut(root).insert((
            Vrm,
            Initialized,
            Name::new("Scene0"),
            RestTransform(Transform::IDENTITY),
            RestGlobalTransform(GlobalTransform::IDENTITY),
            RestWorldTransform(GlobalTransform::IDENTITY),
            ConstraintExecutionOrder(vec![source]),
            VrmNodeConstraint {
                source,
                weight: 1.0,
                kind: VrmConstraintKind::Rotation,
            },
            VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.5)])),
            ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                "happy".to_owned(),
                Vec::new(),
            )]))),
            LookAtProperties::default(),
        ));
        app.world_mut().entity_mut(root).insert((
            VrmNodeIndex(7),
            VrmBone::from("hips"),
            PendingNodeConstraint {
                source_node: 7,
                weight: 1.0,
                kind: VrmConstraintKind::Rotation,
            },
            SpringJointProps::default(),
            SpringJointState::default(),
            VrmHeadOnly,
        ));
        app.world_mut().entity_mut(root).insert(SpringRoot {
            joints: SpringJoints(vec![source]),
            colliders: SpringColliders(Vec::new()),
            center_node: SpringCenterNode(Some(source)),
        });
        app.world_mut()
            .entity_mut(root)
            .insert(ExpressionSettings(HashMap::from([(
                "happy".to_owned(),
                ExpressionSetting {
                    is_binary: false,
                    category: ExpressionCategory::Other,
                    override_blink: ExpressionOverrideType::None,
                    override_look_at: ExpressionOverrideType::None,
                    override_mouth: ExpressionOverrideType::None,
                },
            )])));
        app.world_mut()
            .entity_mut(root)
            .insert(EffectiveExpressionWeights::default());

        trigger_detach(&mut app, root);

        let world = app.world();
        let entity = world.entity(root);
        assert_all_removed!(
            entity,
            Vrm,
            Initialized,
            Name,
            ConstraintExecutionOrder,
            VrmNodeConstraint,
            VrmNodeIndex,
            VrmBone,
            PendingNodeConstraint,
            VrmExpressionWeights,
            EffectiveExpressionWeights,
            ExpressionMorphBinds,
            ExpressionSettings,
            SpringRoot,
            SpringJointProps,
            SpringJointState,
            VrmHeadOnly,
            LookAtProperties,
            RestTransform,
            RestGlobalTransform,
            RestWorldTransform,
        );
        assert!(world.get_entity(root).is_ok());
        success!()
    }

    #[test]
    fn the_root_side_component_set_is_removed() -> TestResult {
        let mut app = setup_app();
        let hips = app.world_mut().spawn_empty().id();
        let root = app.world_mut().spawn_empty().id();

        app.world_mut().entity_mut(root).insert((
            Vrm,
            Initialized,
            Name::new("Elmer"),
            WorldAssetRoot(Handle::default()),
            VrmPath::new("vrm/Elmer.vrm"),
            RestWorldTransform::default(),
            LookAtProperties::default(),
            LookAt::Cursor,
            BodyTracking::default(),
            SmoothedGaze::default(),
            HipsBoneEntity(hips),
        ));

        trigger_detach(&mut app, root);

        let world = app.world();
        let entity = world.entity(root);
        assert_all_removed!(
            entity,
            Vrm,
            Initialized,
            Name,
            WorldAssetRoot,
            VrmPath,
            RestWorldTransform,
            LookAtProperties,
            LookAt,
            BodyTracking,
            SmoothedGaze,
            HipsBoneEntity,
        );
        assert!(world.get_entity(hips).is_ok());
        success!()
    }

    #[test]
    fn a_managed_detach_keeps_the_user_root_and_clears_the_rig() -> TestResult {
        let mut app = setup_app();
        let scene_root = app.world_mut().spawn_empty().id();
        let vrma_child = app.world_mut().spawn_empty().id();
        let bones = app.world_mut().spawn_empty().id();
        let hips = app.world_mut().spawn_empty().id();

        let vrm = app
            .world_mut()
            .spawn((
                VrmHandle(Handle::default()),
                Name::new("avatar"),
                Transform::from_xyz(1.0, 2.0, 3.0),
                LookAt::Cursor,
            ))
            .id();
        app.world_mut().entity_mut(vrm).insert((
            Vrm,
            Initialized,
            PendingVrmScene,
            VrmGeneration(2),
            VrmInstanceState { scene_root },
            WorldAssetRoot(Handle::default()),
            VrmPath::new("vrm/Elmer.vrm"),
            RestWorldTransform::default(),
            LookAtProperties::default(),
            ConstraintExecutionOrder(Vec::new()),
            HipsBoneEntity(hips),
        ));
        app.world_mut()
            .commands()
            .entity(vrm)
            .add_children(&[scene_root, vrma_child, bones]);
        app.update();

        trigger_detach(&mut app, vrm);

        let world = app.world();
        assert!(world.get_entity(vrm).is_ok(), "the root entity survives");
        assert_eq!(
            world.get::<Name>(vrm).map(Name::as_str),
            Some("avatar"),
            "the user's `Name` is kept"
        );
        assert!(world.get::<Transform>(vrm).is_some(), "placement is kept");
        assert!(world.get::<LookAt>(vrm).is_some(), "`LookAt` is kept");
        assert_all_removed!(
            world.entity(vrm),
            Vrm,
            Initialized,
            VrmHandle,
            PendingVrmScene,
            VrmGeneration,
            VrmInstanceState,
            WorldAssetRoot,
            VrmPath,
            RestWorldTransform,
            LookAtProperties,
            ConstraintExecutionOrder,
            HipsBoneEntity,
        );
        for child in [scene_root, vrma_child, bones] {
            assert!(
                world.get_entity(child).is_err(),
                "child {child:?} despawned"
            );
        }
        success!()
    }

    #[test]
    fn a_managed_detach_then_reattach_keeps_the_entity() -> TestResult {
        let mut app = setup_app();
        let scene_root = app.world_mut().spawn_empty().id();
        let vrm = app
            .world_mut()
            .spawn((VrmHandle(Handle::default()), Name::new("avatar")))
            .id();
        app.world_mut().entity_mut(vrm).insert((
            Vrm,
            Initialized,
            VrmGeneration(1),
            VrmInstanceState { scene_root },
        ));

        trigger_detach(&mut app, vrm);
        assert!(app.world().get::<VrmHandle>(vrm).is_none());

        app.world_mut()
            .entity_mut(vrm)
            .insert(VrmHandle(Handle::default()));
        app.update();

        assert!(app.world().get_entity(vrm).is_ok());
        assert_eq!(
            app.world().get::<Name>(vrm).map(Name::as_str),
            Some("avatar")
        );
        success!()
    }

    #[test]
    fn a_raw_ready_detach_clears_the_owners_world_asset_root() -> TestResult {
        let mut app = setup_app();
        let scene_root = app.world_mut().spawn(Vrm).id();
        let owner = app
            .world_mut()
            .spawn(WorldAssetRoot(Handle::default()))
            .id();
        app.world_mut()
            .commands()
            .entity(owner)
            .add_child(scene_root);
        app.update();

        trigger_detach(&mut app, scene_root);

        assert!(
            app.world().get::<WorldAssetRoot>(owner).is_none(),
            "the owner's registration is cleared"
        );
        assert!(app.world().get::<Vrm>(scene_root).is_none());
        assert!(app.world().get_entity(owner).is_ok());
        assert!(app.world().get_entity(scene_root).is_ok());
        success!()
    }

    #[test]
    fn a_raw_pending_detach_cancels_the_spawn() -> TestResult {
        let mut app = setup_app();
        let owner = app
            .world_mut()
            .spawn(WorldAssetRoot(Handle::default()))
            .id();

        trigger_detach(&mut app, owner);

        assert!(app.world().get::<WorldAssetRoot>(owner).is_none());
        assert!(app.world().get_entity(owner).is_ok());
        success!()
    }

    #[test]
    fn a_vrma_root_is_not_detached() -> TestResult {
        let mut app = setup_app();
        let vrma = app
            .world_mut()
            .spawn((Vrma, WorldAssetRoot(Handle::default())))
            .id();

        trigger_detach(&mut app, vrma);

        assert!(
            app.world().get::<WorldAssetRoot>(vrma).is_some(),
            "a `.vrma` is not a VRM and must keep its registration"
        );
        success!()
    }

    #[test]
    fn a_detached_pipeline_scene_stops_running_node_constraints() -> TestResult {
        let mut app = setup_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, RestTransform(Transform::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                RestTransform(Transform::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let root = app
            .world_mut()
            .spawn((
                Vrm,
                Initialized,
                ConstraintExecutionOrder(vec![destination]),
            ))
            .id();

        let driven = Quat::from_rotation_x(FRAC_PI_2);
        set_rotation(&mut app, source, driven);
        run_constraints(&mut app);
        assert!(
            rotation_is(rotation(&app, destination), driven),
            "before the detach the pipeline constraint pass drives the destination: got {:?}",
            rotation(&app, destination)
        );

        trigger_detach(&mut app, root);

        assert!(
            app.world().get::<ConstraintExecutionOrder>(root).is_none(),
            "the execution order is what kept the root in the pass's query"
        );
        set_rotation(&mut app, destination, Quat::IDENTITY);
        set_rotation(&mut app, source, driven);
        run_constraints(&mut app);
        assert_eq!(
            rotation(&app, destination),
            Quat::IDENTITY,
            "a detached scene must not keep writing node constraints"
        );
        success!()
    }

    #[test]
    fn detaching_a_scene_without_a_vrm_path_is_fine() -> TestResult {
        let mut app = setup_app();
        let root = app
            .world_mut()
            .spawn((Vrm, Initialized, ConstraintExecutionOrder(Vec::new())))
            .id();

        trigger_detach(&mut app, root);

        assert!(app.world().get_entity(root).is_ok());
        success!()
    }

    #[test]
    fn detaching_an_entity_without_vrm_is_a_no_op() -> TestResult {
        let mut app = setup_app();
        let node = app
            .world_mut()
            .spawn((
                VrmNodeIndex(3),
                VrmBone::from("hips"),
                RestTransform::default(),
            ))
            .id();

        trigger_detach(&mut app, node);

        assert!(
            app.world().get::<VrmBone>(node).is_some(),
            "a node without `Vrm` is not a VRM root and must not be stripped"
        );
        success!()
    }

    #[test]
    fn a_same_entity_raw_owner_is_removed() -> TestResult {
        let mut app = setup_app();
        let entity = app
            .world_mut()
            .spawn((Vrm, Initialized, WorldAssetRoot(Handle::default())))
            .id();

        trigger_detach(&mut app, entity);

        assert!(app.world().get::<Vrm>(entity).is_none());
        assert!(
            app.world().get::<WorldAssetRoot>(entity).is_none(),
            "the same-entity root registration is removed too"
        );
        assert!(app.world().get_entity(entity).is_ok());
        success!()
    }
}
