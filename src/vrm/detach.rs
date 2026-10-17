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
    PendingVrmScene, VrmGeneration, VrmHandle, VrmInstanceState, VrmSceneRoot, instance_entities,
    remove_root_data,
};
use crate::vrm::look_at::LookAt;
use crate::vrm::spring_bone::{SpringJointProps, SpringJointState, SpringRoot};
use crate::vrm::{Initialized, RestGlobalTransform, RestTransform, Vrm, VrmBone};
use crate::vrma::{VrmAnimationGraphState, Vrma, VrmaHandle};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstance, WorldInstanceSpawner};

/// Triggers VRM detachment on the target entity.
///
/// Removes every VRM component and despawns the child hierarchy, leaving the
/// target entity itself alive.
///
/// Trigger it on:
///
/// * a [`VrmHandle`]-managed avatar — the entity the application spawned, which
///   is what the crate's spawn form returns;
/// * a raw `WorldAssetRoot(..)` spawn whose instance is ready — either the child
///   scene root carrying [`Vrm`], or the owner that carries
///   [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot);
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

fn apply_detach_vrm(
    trigger: On<RequestDetachVrm>,
    managed: Query<(), With<VrmHandle>>,
    vrm_root: Query<(), With<Vrm>>,
    // A raw `WorldAssetRoot` owner. Excludes a `.vrma` root, which also carries a
    // `WorldAssetRoot` but is not a VRM and is not this event's teardown.
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
        // Both the child VRM root and the raw owner resolve their owner and rig
        // against the live world inside the command.
        commands.queue(move |world: &mut World| detach_raw(world, entity));
    }
}

/// Detaches a [`VrmHandle`]-managed avatar.
///
/// The shared root inventory comes off through [`remove_root_data`] — the same
/// list the attach moves *on*, so the two cannot drift — and then the lifecycle
/// markers and the identity handle go. Components the application put on the
/// entity itself (`Name`, `Transform`, `LookAt`, `BodyTracking`, …) are left
/// alone: the entity keeps its identity and can be re-attached.
///
/// The generation is dropped here, unlike on an invalidation: every child of the
/// entity is despawned, so a preserved `.vrma` cannot survive to be confused by
/// a reset counter. Any instance node the application reparented out of the
/// hierarchy is despawned too (captured before the `WorldAssetRoot` hook
/// unregisters the instance), and the private [`VrmAnimationGraphState`] — whose
/// clip handles are derived data of this rig — is removed with it.
fn detach_managed(
    world: &mut World,
    entity: Entity,
) {
    if world.get_entity(entity).is_err() {
        return;
    }
    // Revalidate the shape the event was classified as: a stale event must not
    // tear down an entity that is no longer the managed avatar (already detached,
    // or re-created as something else).
    if world.get::<VrmHandle>(entity).is_none() {
        return;
    }
    // Capture the instance's tracked entities *before* `WorldAssetRoot` is
    // removed: its `on_remove` unregisters the instance, after which the spawner
    // can no longer report them. This is what lets a managed node the
    // application reparented out of the hierarchy be despawned too.
    let tracked = world
        .get::<WorldInstance>(entity)
        .map(|instance| **instance)
        .map(|instance_id| instance_entities(world, instance_id))
        .unwrap_or_default();

    remove_root_data(world, entity);
    // `WorldAssetRoot` must be removed while `WorldInstance` is still present:
    // its `on_remove` hook reads the latter to unregister the instance from the
    // spawner. Removing it first is what keeps the spawner from leaking the
    // instance.
    world.entity_mut(entity).remove::<WorldAssetRoot>();
    world
        .entity_mut(entity)
        .remove::<Vrm>()
        .remove::<Initialized>()
        .remove::<VrmGeneration>()
        .remove::<VrmInstanceState>()
        .remove::<PendingVrmScene>()
        .remove::<VrmHandle>()
        // The engine never removes `WorldInstance` itself; without this the owner
        // would keep naming an instance that no longer exists.
        .remove::<WorldInstance>()
        // The private graph state persists across an *invalidation* so a reload
        // can rebuild into it, but a full detach drops it: its clip handles are
        // derived data of the rig that is being torn down.
        .remove::<VrmAnimationGraphState>();
    despawn_children(world, entity);
    // `despawn_children` already despawned the owner's own hierarchy; this only
    // catches tracked scene nodes that were reparented away. Dead ids are skipped.
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
///
/// The true instance owner is found through [`WorldInstance`] /
/// [`WorldInstanceSpawner`] rather than by trusting an arbitrary [`ChildOf`], so
/// an unrelated avatar or scene in the ancestry is never unregistered. The shape
/// is re-validated against the live world first: a stale event must not tear down
/// a rig that has since become a managed avatar or a `.vrma`, nor touch an
/// unrelated ancestor that merely happens to carry a `WorldAssetRoot`.
fn detach_raw(
    world: &mut World,
    entity: Entity,
) {
    if world.get_entity(entity).is_err() {
        return;
    }
    // Revalidate the raw shape the event was classified as.
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
    // A hand-built same-entity shape carries the `WorldAssetRoot` *and* the rig
    // on one entity: it is its own owner, so it is torn down as well instead of
    // being left registered and half-stripped.
    let (owner, root) = if entity_is_root {
        let owner = if entity_is_owner {
            Some(entity)
        } else {
            find_raw_owner(world, entity)
        };
        (owner, Some(entity))
    } else {
        // The request named the owner: find the VRM root of its live instance, so
        // a ready spawn is torn down too instead of being left as an orphan.
        (Some(entity), find_raw_instance_root(world, entity))
    };

    // Collect the instance's tracked entities *before* `WorldAssetRoot` is
    // removed: its `on_remove` unregisters the instance, after which the spawner
    // can no longer report them. Capturing ids first lets a node the application
    // reparented out of the hierarchy be cleaned up too, without despawning
    // anything the instance does not own.
    let tracked = owner
        .and_then(|owner| world.get::<WorldInstance>(owner).map(|instance| **instance))
        .map(|instance_id| instance_entities(world, instance_id))
        .unwrap_or_default();

    if let Some(owner) = owner
        && world.get_entity(owner).is_ok()
    {
        // Removal order matters: `WorldAssetRoot`'s `on_remove` reads
        // `WorldInstance` to unregister the instance.
        world.entity_mut(owner).remove::<WorldAssetRoot>();
    }
    if let Some(root) = root
        && world.get_entity(root).is_ok()
    {
        strip_raw_root(world, root);
    }
    // `strip_raw_root` already despawned the root's own hierarchy; this only
    // catches tracked nodes that were reparented away. Dead ids are skipped.
    for tracked_entity in tracked {
        if Some(tracked_entity) == root {
            continue;
        }
        if world.get_entity(tracked_entity).is_ok() {
            world.entity_mut(tracked_entity).despawn();
        }
    }
    if let Some(owner) = owner
        && world.get_entity(owner).is_ok()
    {
        world.entity_mut(owner).remove::<WorldInstance>();
    }
}

/// Finds the [`WorldAssetRoot`] owner of the raw instance that `root` belongs to.
///
/// Walks the ancestors and accepts only one that actually owns `root`'s instance.
/// The nearest-ancestor fallback is used **only when there is no
/// [`WorldInstanceSpawner`]** at all (a hand-built scene, where there is no
/// bookkeeping to verify against). With a spawner present but no
/// [`WorldInstance`] on an ancestor, that ancestor cannot be verified as the
/// owner, so it is skipped rather than trusting an unrelated nested avatar.
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
                    // A `WorldAssetRoot` ancestor that does not own this instance:
                    // keep walking instead of trusting it.
                }
                // No spawner at all: the nearest `WorldAssetRoot` ancestor is the
                // owner by construction.
                (None, _) => return Some(parent),
                // A spawner is present but this ancestor has no instance metadata:
                // it cannot be verified as the owner, so do not touch it.
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

/// Strips the VRM rig off a raw instance's child scene root and despawns its
/// children, leaving the scene root itself alive as a plain scene.
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
        // The private VRMA graph state persisted on this raw Vrm root is derived
        // data of the rig, so it goes with a full detach too.
        .remove::<VrmAnimationGraphState>();
    despawn_children(world, root);
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

    /// A hand-built shape that carries the rig *and* its own `WorldAssetRoot` on
    /// one entity must have that registration removed too, not left half-stripped.
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
