//! [`RequestDetachVrm`]: tear down a VRM rig while keeping the entity the
//! application owns.
//!
//! Three shapes of entity can be the target, decided by what the entity carries:
//!
//! * **Managed** — a [`VrmHandle`] is present, so this is the entity the
//!   application spawned with `commands.spawn(VrmHandle(..))`, and it is the VRM
//!   root. Everything the load-time pipeline moved onto it is taken back off, the
//!   handle is dropped and the child hierarchy (the internal scene root and the
//!   user's `.vrma` children) is despawned. The user's own components — `Name`,
//!   `Transform`, `LookAt`, `BodyTracking`, arbitrary ones — are left in place, so
//!   the entity survives as a plain entity and can be re-attached by inserting a
//!   new [`VrmHandle`].
//! * **Raw, ready** — the entity carries [`Vrm`] but no [`VrmHandle`]: a raw
//!   `WorldAssetRoot(..)` spawn whose scene has already instantiated, so the VRM
//!   is the *child* scene root. Its rig is stripped and its children despawned,
//!   and the owner's [`WorldAssetRoot`] is removed so the spawner unregisters the
//!   instance (the component lives on the parent, not on this entity).
//! * **Raw, pending** — the entity carries [`WorldAssetRoot`] but neither
//!   [`Vrm`] nor [`VrmHandle`]: a raw spawn whose asset has not produced an
//!   instance yet. Removing the [`WorldAssetRoot`] cancels it. A `.vrma` root is
//!   excluded: it also carries a [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot),
//!   but it is not a VRM and this event is not its teardown.

use crate::vrm::body_tracking::{BodyTracking, SmoothedGaze};
use crate::vrm::components::{PendingNodeConstraint, VrmHeadOnly, VrmNodeConstraint, VrmNodeIndex};
use crate::vrm::instance::{
    PendingVrmScene, VrmGeneration, VrmHandle, VrmInstanceState, VrmSceneRoot, remove_root_data,
};
use crate::vrm::look_at::LookAt;
use crate::vrm::spring_bone::{SpringJointProps, SpringJointState, SpringRoot};
use crate::vrm::{Initialized, RestGlobalTransform, RestTransform, Vrm, VrmBone};
use crate::vrma::{Vrma, VrmaHandle};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;

/// Triggers VRM detachment on the target entity.
///
/// Removes every VRM component and despawns the child hierarchy, leaving the
/// target entity itself alive.
///
/// Trigger it on the VRM root:
///
/// * a [`VrmHandle`]-managed avatar — the entity the application spawned, which
///   is what the crate's spawn form returns;
/// * a raw `WorldAssetRoot(..)` spawn whose instance is ready — the child scene
///   root carrying [`Vrm`];
/// * a raw `WorldAssetRoot(..)` spawn that has not instantiated yet — the owner
///   entity itself, to cancel it.
///
/// Triggering it anywhere else is a no-op.
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

/// Which teardown a detach request maps to, decided while the entity is still in
/// its pre-detach shape.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum DetachKind {
    /// A [`VrmHandle`]-managed avatar: the target is the VRM root the
    /// application owns.
    Managed,
    /// A raw `WorldAssetRoot` instance already attached: the target is the child
    /// scene root carrying [`Vrm`].
    RawReady,
    /// A raw `WorldAssetRoot` instance not yet spawned: the target is the owner.
    RawPending,
}

fn apply_detach_vrm(
    trigger: On<RequestDetachVrm>,
    managed: Query<(), With<VrmHandle>>,
    raw_ready: Query<(), With<Vrm>>,
    // A raw owner before the instance arrives: it has the root to remove, and is
    // neither a managed avatar nor a `.vrma` (which also uses `WorldAssetRoot`).
    raw_pending: Query<
        (),
        (
            With<WorldAssetRoot>,
            Without<VrmHandle>,
            Without<Vrm>,
            Without<Vrma>,
            Without<VrmaHandle>,
        ),
    >,
    mut commands: Commands,
) {
    let entity = trigger.event_target();
    let kind = if managed.contains(entity) {
        DetachKind::Managed
    } else if raw_ready.contains(entity) {
        DetachKind::RawReady
    } else if raw_pending.contains(entity) {
        DetachKind::RawPending
    } else {
        return;
    };
    // Run against the world directly: `remove_root_data` is an exclusive helper
    // shared with the attach, and the teardown must see the entity's current
    // children rather than the observer's snapshot.
    commands.queue(move |world: &mut World| detach(world, entity, kind));
}

/// Applies the teardown chosen by [`apply_detach_vrm`].
fn detach(
    world: &mut World,
    entity: Entity,
    kind: DetachKind,
) {
    if world.get_entity(entity).is_err() {
        return;
    }
    match kind {
        DetachKind::Managed => detach_managed(world, entity),
        DetachKind::RawReady => detach_raw_ready(world, entity),
        DetachKind::RawPending => detach_raw_pending(world, entity),
    }
}

/// Detaches a [`VrmHandle`]-managed avatar.
///
/// The shared root inventory comes off through [`remove_root_data`] — the same
/// list the attach moves *on*, so the two cannot drift — and then the lifecycle
/// markers and the identity handle go. Components the application put on the
/// entity itself (`Name`, `Transform`, `LookAt`, `BodyTracking`, …) are left
/// alone: the entity keeps its identity and can be re-attached.
fn detach_managed(
    world: &mut World,
    entity: Entity,
) {
    remove_root_data(world, entity);
    world
        .entity_mut(entity)
        .remove::<Vrm>()
        .remove::<Initialized>()
        .remove::<VrmGeneration>()
        .remove::<VrmInstanceState>()
        .remove::<PendingVrmScene>()
        // Dropping the handle ends the managed identity; dropping
        // `WorldAssetRoot` runs bevy's `on_remove` hook, which unregisters the
        // instance from its `WorldInstanceSpawner`.
        .remove::<VrmHandle>()
        .remove::<WorldAssetRoot>();
    despawn_children(world, entity);
}

/// Detaches a raw `WorldAssetRoot` instance that is already attached.
///
/// The target is the child scene root carrying [`Vrm`], so its rig is stripped
/// and its children despawned. [`WorldAssetRoot`] lives on the owner, found
/// through [`ChildOf`], and is removed there to unregister the instance; a
/// hand-built scene that carries it on the target directly is covered too.
fn detach_raw_ready(
    world: &mut World,
    entity: Entity,
) {
    if let Some(owner) = world
        .get::<ChildOf>(entity)
        .map(|child_of| child_of.parent())
        && owner != entity
    {
        world.entity_mut(owner).remove::<WorldAssetRoot>();
    }
    remove_root_data(world, entity);
    world
        .entity_mut(entity)
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
        .remove::<VrmHeadOnly>();
    despawn_children(world, entity);
}

/// Cancels a raw `WorldAssetRoot` spawn that has not instantiated yet.
///
/// There is no rig to strip; removing [`WorldAssetRoot`] unregisters the spawn.
/// Bevy keeps a queued spawn alive across `unregister_instance`, so a late
/// instance is cleaned up by the `WorldInstanceReady` observer in
/// [`crate::vrm::instance`] instead.
fn detach_raw_pending(
    world: &mut World,
    entity: Entity,
) {
    world.entity_mut(entity).remove::<WorldAssetRoot>();
}

/// Despawns all direct children of `entity`.
///
/// `despawn()` is recursive, so this also despawns every descendant.
fn despawn_children(
    world: &mut World,
    entity: Entity,
) {
    let children = world
        .get::<Children>(entity)
        .map(|children| children.to_vec());
    if let Some(children) = children {
        for child in children {
            world.entity_mut(child).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
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

    /// Asserts that `$entity` carries none of the listed components, naming the
    /// one that survived instead of only a line number.
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
        let mut app = test_app();
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

    /// The complete load-time inventory, on one entity, in the two places the
    /// pipeline writes it.
    ///
    /// `handler::scene::finalize` puts the first group on the scene *root*
    /// inside the asset; `handler::nodes::process_node`,
    /// `resolve_constraints`, `build_spring_chains` and `spawn_head_copies` put
    /// the second group on the individual nodes. They are asserted together
    /// because the target of a detach is whatever entity the caller named, so
    /// the removal cannot assume where in the scene a component sits — and
    /// because a component that ships inside a `WorldAsset` outlives every load
    /// that produced it, so the list is the only place it can be caught.
    ///
    /// `ConstraintExecutionOrder` is the one that actually leaked: it lives on
    /// the root, and `apply_node_constraints` iterates every entity carrying one
    /// on every frame.
    #[test]
    fn the_load_time_component_set_is_removed() -> TestResult {
        let mut app = setup_app();
        let source = app.world_mut().spawn_empty().id();
        let root = app.world_mut().spawn_empty().id();

        // Inserted in the same grouping as `handler::scene::finalize`, because
        // that is where each of them comes from.
        app.world_mut().entity_mut(root).insert((
            // `mark_initialized`
            Vrm,
            Initialized,
            Name::new("Scene0"),
            // `insert_rest_transforms`
            RestTransform(Transform::IDENTITY),
            RestGlobalTransform(GlobalTransform::IDENTITY),
            RestWorldTransform(GlobalTransform::IDENTITY),
            // `resolve_constraints`
            ConstraintExecutionOrder(vec![source]),
            VrmNodeConstraint {
                source,
                weight: 1.0,
                kind: VrmConstraintKind::Rotation,
            },
            // `build_expressions`
            VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.5)])),
            ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                "happy".to_owned(),
                Vec::new(),
            )]))),
            // `build_look_at`
            LookAtProperties::default(),
        ));
        app.world_mut().entity_mut(root).insert((
            // `handler::nodes::process_node`
            VrmNodeIndex(7),
            VrmBone::from("hips"),
            PendingNodeConstraint {
                source_node: 7,
                weight: 1.0,
                kind: VrmConstraintKind::Rotation,
            },
            // `build_spring_chains`
            SpringJointProps::default(),
            SpringJointState::default(),
            // `spawn_head_copies`
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
        // The entity itself survives: the result is a plain scene, not nothing.
        assert!(world.get_entity(root).is_ok());
        success!()
    }

    /// The root-side set, so the node-side list above did not quietly narrow it:
    /// `handler::scene::insert_source_path` writes `VrmPath` on the scene root,
    /// `insert_humanoid_bone_holders` puts a bone holder on it, and `LookAt` /
    /// `BodyTracking` are user components the application may have put there.
    ///
    /// `VrmPath` is the asymmetry worth pinning: only `insert_source_path`
    /// writes it, and only on the root, so a mesh node is not guaranteed to have
    /// one. The removal covers the absent case without a branch of its own.
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
            // User-inserted components.
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
        // The bone entity was never the target, and only VRM components are
        // removed, so it survives as an ordinary entity.
        assert!(world.get_entity(hips).is_ok());
        success!()
    }

    /// A managed detach clears the rig but keeps the entity and the application's
    /// own components, and despawns the child hierarchy.
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
        // The user's entity and components survive.
        assert!(world.get_entity(vrm).is_ok(), "the root entity survives");
        assert_eq!(
            world.get::<Name>(vrm).map(Name::as_str),
            Some("avatar"),
            "the user's `Name` is kept"
        );
        assert!(world.get::<Transform>(vrm).is_some(), "placement is kept");
        assert!(world.get::<LookAt>(vrm).is_some(), "`LookAt` is kept");
        // The rig, the identity and the registration are gone.
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
        // The whole child hierarchy is despawned.
        for child in [scene_root, vrma_child, bones] {
            assert!(
                world.get_entity(child).is_err(),
                "child {child:?} despawned"
            );
        }
        success!()
    }

    /// A managed detach then a fresh `VrmHandle` re-attaches onto the same
    /// entity: the identity survives the round trip.
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

        // The application asks for the avatar again: a new handle re-arms the
        // resolver, and the same entity is the VRM root once more.
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

    /// A raw-ready detach removes the `WorldAssetRoot` that lives on the *owner*,
    /// not on the child scene root the event is triggered on.
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

    /// A raw-pending detach cancels a spawn that has no instance yet, by removing
    /// the owner's `WorldAssetRoot`.
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

    /// A `.vrma` root also carries a `WorldAssetRoot`, but it is not a VRM, so a
    /// detach request on it must leave it completely alone.
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

    /// The regression the whole load-time set exists for: a pipeline root that
    /// kept `ConstraintExecutionOrder` stayed in `apply_node_constraints`'s
    /// query for the rest of the app's life, driving bones of a scene that was
    /// supposed to be inert.
    ///
    /// The destination is deliberately *not* a descendant of the root, because
    /// that is the case the removal has to survive: `ConstraintExecutionOrder`
    /// is load-time data that lists entities by id, so it keeps answering for
    /// destinations that outlive the root they were read from — exactly what
    /// `apply_node_constraints` documents when it skips a stale entry.
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
        // Reset and drive again: with the order gone, the destination must stay
        // exactly where it is put.
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

    /// `VrmPath` is only on the scene *root* that
    /// `handler::scene::insert_source_path` wrote it on, so it is not a component
    /// every VRM entity is guaranteed to carry. Detaching one without it must not
    /// panic.
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

    /// The `Vrm` guard is what scopes a detach: an entity that does not own a
    /// VRM mesh hierarchy is left completely alone, even when it carries
    /// components that appear on the removal list.
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
}
