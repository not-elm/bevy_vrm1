//! Root identity for a managed `.vrm`: [`VrmHandle`], the deferred scene
//! resolution, and the attach that promotes the load-time root data onto the
//! entity the application spawned.
//!
//! # The problem
//!
//! The load-time pipeline ([`crate::vrm::gltf`]) bakes every VRM component into
//! the scene [`WorldAsset`] while the file loads. Bevy then instantiates that
//! asset as a *child* of whatever entity carries
//! [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot), asynchronously
//! and only once the asset exists. A plain
//! `commands.spawn(VrmHandle(asset_server.load("x.vrm")))` would therefore leave
//! the application's own entity with none of the components user code addresses
//! — no [`Vrm`], no [`Initialized`], no bone holders.
//!
//! # The identity contract
//!
//! The entity the application spawns *is* the VRM. [`VrmHandle`] stays on it,
//! and when the scene instance is ready the root-level data baked into the
//! scene root is moved onto that entity:
//!
//! ```text
//! user entity : VrmHandle, WorldAssetRoot, Transform, Visibility, Vrm,
//!               Initialized, VrmGeneration, VrmPath, RestWorldTransform,
//!               LookAtProperties, expressions, <Bone>BoneEntity holders, ...
//! ├─ scene root (internal) : VrmSceneRoot, coordinate-conversion Transform,
//! │                          skeleton, AnimationPlayer (on the root bone)
//! └─ .vrma children spawned by the user
//! ```
//!
//! The internal scene root keeps its [`Transform`]/[`GlobalTransform`], its
//! hierarchy, its [`Name`] and the player, because those are what `bevy_gltf` built
//! and moving the conversion transform would overwrite the user's placement.
//! Everything else the handler wrote on the root is moved with a controlled
//! `take`/`insert` (never `move_components`, which would remap the self-scoped
//! `AnimatedBy` of a no-`hips` rig into the user root), preserving every entity
//! reference. [`Initialized`] is inserted last, so `On<Insert, Initialized>`
//! observers see a fully populated root.
//!
//! # The handler bakes a private marker
//!
//! [`scene::mark_initialized`](crate::vrm::gltf::handler::scene) writes
//! [`VrmSceneRoot`] instead of the public markers, so moving components does not
//! fire `On<Add, Vrm>` / `Added<Initialized>` twice. The attach observer inserts
//! the public markers exactly once per successful attach, on the user's entity
//! for a managed spawn and on the child scene root for a raw
//! `WorldAssetRoot(..)` spawn (which keeps the documented "the VRM is the child"
//! behavior).
//!
//! # Gating and cancellation
//!
//! [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot) is inserted
//! only once the scene [`WorldAsset`] is actually present in
//! [`Assets<WorldAsset>`], in `SpawnScene` before bevy's own
//! `WorldInstanceSpawner` pass. Bevy's spawner re-queues a spawn whose asset does
//! not exist yet, and `unregister_instance` / `remove::<WorldAssetRoot>` do not
//! cancel that queued entry; not queueing until the asset exists is what makes a
//! detach-before-load actually cancel. A late instance that still arrives for a
//! cancelled spawn is cleaned up by the global [`WorldInstanceReady`] observer.
//!
//! # Lifecycle bookkeeping
//!
//! [`VrmGeneration`] is bumped on every successful attach. [`VrmInstanceState`]
//! records the owner's current internal scene root, which [`invalidate_dead_rigs`]
//! uses to invalidate a rig whose scene root died without a replacement (a
//! de-registered instance, a hot reload to a non-VRM), while a newer rig — whose
//! recorded root is alive — is never touched by a delayed cleanup. The attach
//! observer is scoped to `ready.instance_id`, and ignores a ready for a
//! superseded instance by comparing against the entity's
//! [`WorldInstance`].

use crate::macros::marker_component;
use crate::prelude::*;
use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::app::{App, Plugin, SceneSpawnerSystems, SpawnScene, Update};
use bevy::asset::{Assets, Handle};
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::{DeferredWorld, World};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::world_serialization::{
    InstanceId, WorldAsset, WorldAssetRoot, WorldInstance, WorldInstanceReady, WorldInstanceSpawner,
};

/// A handle to a `.vrm` asset, and the marker that says "this entity is a VRM
/// the crate manages".
///
/// Spawn it where the avatar should live; the entity keeps its identity across
/// loading, model swaps, hot reload and re-attachment. The entity is given a
/// [`Transform`] and [`Visibility`] (via `#[require]`) and, once the scene
/// instance is ready, [`Vrm`] + [`Initialized`], the bone holders and every
/// other root-level component.
///
/// Inserting a different handle re-resolves the avatar onto the new model;
/// mutating the existing handle in place is caught by [`Changed`] detection. The
/// handle stays on the entity after loading.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy_vrm1::prelude::*;
/// fn spawn_avatar(mut commands: Commands, asset_server: Res<AssetServer>) {
///     let vrm = commands
///         .spawn((
///             VrmHandle(asset_server.load("vrm/Elmer.vrm")),
///             LookAt::Cursor,
///             Transform::from_xyz(0.0, 0.0, 1.0),
///         ))
///         .with_child(VrmaHandle(asset_server.load("vrma/VRMA_01.vrma")))
///         .id();
/// }
/// ```
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
#[require(Transform, Visibility)]
#[component(on_insert = mark_vrm_scene_pending)]
pub struct VrmHandle(pub Handle<Gltf>);

impl VrmHandle {
    /// Creates a new [`VrmHandle`].
    #[must_use]
    pub fn new(handle: Handle<Gltf>) -> Self {
        Self(handle)
    }
}

/// "A [`VrmHandle`] on this entity has not been resolved to a scene yet."
///
/// Inserted by the [`VrmHandle`] `on_insert` hook and re-armed on a `Changed`
/// handle, removed by [`resolve_vrm_scene`] once the matching [`WorldAssetRoot`]
/// is in place. Being a marker rather than a one-frame `Changed` filter is what
/// lets an unloaded handle keep retrying until its asset exists.
#[derive(Component, Default, Debug, Clone, Copy)]
pub(crate) struct PendingVrmScene;

/// Baked by the load-time pipeline onto the root of a `.vrm` scene.
///
/// It is the private counterpart of [`Vrm`] + [`Initialized`]: the attach
/// observer finds the instance's scene root through it, and the public markers
/// are inserted at attach time so their lifecycle events fire exactly once, on
/// the entity the application owns.
marker_component!(
    /// Marks the internal scene root of a `.vrm` scene.
    VrmSceneRoot
);

/// Which attach generation the current rig is.
///
/// Incremented on every successful attach, so a consumer can tell a re-attach
/// (model swap, hot reload) from the first one. A hand-built VRM scene without
/// this component is generation `0` by convention.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct VrmGeneration(pub u64);

/// The owner's currently attached internal scene root.
///
/// Present only while a rig is attached. [`invalidate_dead_rigs`] compares the
/// recorded root against the live scene roots every frame, so a rig whose root
/// died is invalidated while a newer rig — with a different, live root — is not.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct VrmInstanceState {
    /// The internal scene root of the attached instance.
    pub(crate) scene_root: Entity,
}

/// The `on_insert` hook of [`VrmHandle`]: arm the resolver.
fn mark_vrm_scene_pending(
    mut world: DeferredWorld,
    context: HookContext,
) {
    world
        .commands()
        .entity(context.entity)
        .insert(PendingVrmScene);
}

/// Re-arms [`PendingVrmScene`] whenever a [`VrmHandle`] changes.
///
/// The `on_insert` hook only runs on a structural insertion, so a handle mutated
/// in place (`*handle = VrmHandle(..)` / `handle.0 = ..`) would otherwise never
/// be re-resolved. Running on `Changed` — which also covers the insertion case —
/// makes both paths the same.
fn rearm_pending_vrm(
    mut commands: Commands,
    changed: Query<Entity, (Changed<VrmHandle>, Without<PendingVrmScene>)>,
) {
    for entity in &changed {
        commands.entity(entity).insert(PendingVrmScene);
    }
}

/// Resolves every pending [`VrmHandle`] to its scene, gated on the scene
/// [`WorldAsset`] actually being loaded.
///
/// Runs in `SpawnScene` **before** bevy's own `WorldInstanceSpawner` pass and
/// touches the world directly (an exclusive system), so the
/// [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot) it inserts is
/// seen by that pass in the same frame.
///
/// A handle whose `Gltf` is not loaded, whose `Gltf` has no scene, or whose
/// scene `WorldAsset` is not in [`Assets<WorldAsset>`] yet simply stays pending
/// and is retried next frame; the [`WorldAssetRoot`] is never inserted for an
/// asset that does not exist, which is what keeps a later detach a real
/// cancellation rather than a stranded queue entry.
fn resolve_vrm_scene(world: &mut World) {
    let pending: Vec<(Entity, Handle<Gltf>)> = {
        let mut query = world.query_filtered::<(Entity, &VrmHandle), With<PendingVrmScene>>();
        query
            .iter(world)
            .map(|(entity, handle)| (entity, handle.0.clone()))
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let resolved: Vec<(Entity, Handle<WorldAsset>)> = {
        let Some(gltfs) = world.get_resource::<Assets<Gltf>>() else {
            return;
        };
        let Some(world_assets) = world.get_resource::<Assets<WorldAsset>>() else {
            return;
        };
        pending
            .iter()
            .filter_map(|(entity, handle)| {
                let gltf = gltfs.get(handle)?;
                let scene = gltf.scenes.first()?;
                world_assets
                    .contains(scene.id())
                    .then(|| (*entity, scene.clone()))
            })
            .collect()
    };

    for (entity, scene) in resolved {
        if world.get_entity(entity).is_err() {
            continue;
        }
        world.entity_mut(entity).remove::<PendingVrmScene>();
        // Already showing this scene (a re-armed handle that did not change the
        // resolved asset): re-inserting would spuriously respawn the instance.
        let already_resolved = world
            .get::<WorldAssetRoot>(entity)
            .is_some_and(|root| root.0.id() == scene.id());
        if !already_resolved {
            world.entity_mut(entity).insert(WorldAssetRoot(scene));
        }
    }
}

/// The single global [`WorldInstanceReady`] observer.
///
/// Scoped to `ready.instance_id` (the new instance's entities) and to the
/// entity's current [`WorldInstance`], so a ready for a superseded instance is
/// ignored rather than mistaken for a non-VRM replacement.
fn attach_vrm_instance(
    ready: On<WorldInstanceReady>,
    spawner: Option<Res<WorldInstanceSpawner>>,
    managed: Query<(), With<VrmHandle>>,
    raw: Query<(), (With<WorldAssetRoot>, Without<VrmHandle>)>,
    instances: Query<&WorldInstance>,
    scene_roots: Query<(), With<VrmSceneRoot>>,
    mut commands: Commands,
) {
    let parent = ready.event_target();

    // A ready for an instance that has already been superseded (bevy reuses the
    // `InstanceId` on hot reload, and a stale event can outlive a swap) must not
    // touch the owner. Bevy clears the superseded instance's entity map when it
    // despawns it, so there is nothing left to clean either.
    if let Ok(world_instance) = instances.get(parent)
        && **world_instance != ready.instance_id
    {
        return;
    }

    let scene_root = spawner.as_ref().and_then(|spawner| {
        spawner
            .iter_instance_entities(ready.instance_id)
            .find(|entity| scene_roots.contains(*entity))
    });

    if managed.contains(parent) {
        match scene_root {
            Some(root) => {
                commands.queue(move |world: &mut World| attach_managed(world, parent, root));
            }
            // The new instance is not a VRM (or has no scene): drop any stale rig
            // so an `Initialized` root never outlives the rig it described.
            None => {
                commands.queue(move |world: &mut World| invalidate_managed(world, parent));
            }
        }
    } else if raw.contains(parent) {
        // Raw `WorldAssetRoot(..)` spawn: the VRM stays on the child, as before.
        if let Some(root) = scene_root {
            commands.entity(root).insert((Vrm, Initialized));
        }
    } else if scene_root.is_some() {
        // A crate-managed VRM scene whose owner no longer wants it: a queued
        // spawn that survived a detach. Its entities are removed.
        let instance_id = ready.instance_id;
        commands.queue(move |world: &mut World| cleanup_stale_instance(world, instance_id, parent));
    }
}

/// Moves the load-time root data of `scene_root` onto `parent` and marks the
/// attach.
///
/// All of it happens synchronously inside one command, and [`Initialized`] is
/// inserted last, so an `On<Insert, Initialized>` observer always sees a fully
/// populated root.
fn attach_managed(
    world: &mut World,
    parent: Entity,
    scene_root: Entity,
) {
    if world.get_entity(parent).is_err() || world.get_entity(scene_root).is_err() {
        return;
    }
    let generation = world
        .get::<VrmGeneration>(parent)
        .map_or(1, |previous| previous.0.saturating_add(1));

    // Now that a fresh rig is attached, any data left by a previous one is stale:
    // holders may name despawned bones and an `Initialized` root may point at a
    // rig that no longer exists. Clear before moving the new values in.
    clear_attached_state(world, parent);
    move_root_data(world, scene_root, parent);

    world.entity_mut(parent).insert((
        Vrm,
        VrmGeneration(generation),
        VrmInstanceState { scene_root },
    ));
    // Last, so observers on `On<Insert, Initialized>` see `Vrm`, the generation
    // and every root component already in place.
    world.entity_mut(parent).insert(Initialized);
}

/// Removes every attached-rig component from a managed owner.
///
/// `VrmHandle` is deliberately kept: it is the entity's identity, and this is
/// also the hot-reload-to-a-non-VRM path, where the handle still names the asset
/// the application asked for.
fn invalidate_managed(
    world: &mut World,
    parent: Entity,
) {
    if world.get_entity(parent).is_err() {
        return;
    }
    clear_attached_state(world, parent);
}

/// Removes all root data and the attach markers, leaving `VrmHandle` in place.
fn clear_attached_state(
    world: &mut World,
    entity: Entity,
) {
    remove_root_data(world, entity);
    world
        .entity_mut(entity)
        .remove::<(Vrm, Initialized, VrmGeneration, VrmInstanceState)>();
}

/// Despawns a late instance of a spawn that was already cancelled.
///
/// Only reached when the instance carries a [`VrmSceneRoot`], so it can never
/// touch an unrelated glTF or `.vrma` scene. The owner is left as the plain
/// entity the application still holds, and the [`WorldInstance`] component the
/// engine inserted when the spawn was queued is removed with the instance it
/// named.
fn cleanup_stale_instance(
    world: &mut World,
    instance_id: InstanceId,
    parent: Entity,
) {
    if world.get_resource::<WorldInstanceSpawner>().is_none() {
        return;
    }
    world.resource_scope(|world, mut spawner: Mut<WorldInstanceSpawner>| {
        spawner.despawn_instance_sync(world, &instance_id);
    });
    if let Ok(mut owner) = world.get_entity_mut(parent) {
        owner.remove::<WorldInstance>();
    }
}

/// Invalidates any attached rig whose internal scene root no longer exists.
///
/// A rig can die without a replacement `WorldInstanceReady` (an externally
/// de-registered instance, a scene root despawned directly). The recorded
/// [`VrmInstanceState::scene_root`] identifies *this* rig, so a newer rig — with
/// a different, live root — is never invalidated by a delayed check.
fn invalidate_dead_rigs(
    mut commands: Commands,
    owners: Query<(Entity, &VrmInstanceState)>,
    scene_roots: Query<(), With<VrmSceneRoot>>,
) {
    for (owner, state) in &owners {
        if scene_roots.contains(state.scene_root) {
            continue;
        }
        commands.queue(move |world: &mut World| invalidate_managed(world, owner));
    }
}

/// Generates the shared root-level component inventory's move/remove helpers.
///
/// The list lives in one place so the attach ([`move_root_data`]) and the detach
/// ([`remove_root_data`], used by `crate::vrm::detach`) cannot drift apart. Every
/// entry is a single present-or-absent component, and every move is a
/// `take`/`insert` that preserves the value's entity references.
macro_rules! define_root_data_ops {
    ($($ty:ty),* $(,)?) => {
        /// Moves every present root-level component from `from` onto `to`.
        pub(crate) fn move_root_data(world: &mut World, from: Entity, to: Entity) {
            $(
                if let Some(value) = world.entity_mut(from).take::<$ty>() {
                    world.entity_mut(to).insert(value);
                }
            )*
            move_root_holders(world, from, to);
        }

        /// Removes every root-level component from `entity`.
        pub(crate) fn remove_root_data(world: &mut World, entity: Entity) {
            $(
                world.entity_mut(entity).remove::<$ty>();
            )*
            remove_root_holders(world, entity);
        }
    };
}

/// The `<Bone>BoneEntity` holders, written onto the root by the load-time
/// pipeline and shared by every detach/move operation.
macro_rules! define_root_holder_ops {
    ($($bone:ident),+ $(,)?) => {
        /// Moves every `<Bone>BoneEntity` holder present on `from` to `to`.
        pub(crate) fn move_root_holders(world: &mut World, from: Entity, to: Entity) {
            paste::paste! {
                $(
                    if let Some(holder) = world
                        .entity_mut(from)
                        .take::<crate::vrm::humanoid_bone::prelude::[<$bone BoneEntity>]>()
                    {
                        world.entity_mut(to).insert(holder);
                    }
                )+
            }
        }

        /// Removes every `<Bone>BoneEntity` holder from `entity`.
        pub(crate) fn remove_root_holders(world: &mut World, entity: Entity) {
            paste::paste! {
                $(
                    world
                        .entity_mut(entity)
                        .remove::<crate::vrm::humanoid_bone::prelude::[<$bone BoneEntity>]>();
                )+
            }
        }
    };
}

define_root_holder_ops!(
    Hips,
    RightRingProximal,
    RightThumbDistal,
    RightRingIntermediate,
    RightUpperArm,
    LeftIndexProximal,
    LeftUpperLeg,
    LeftFoot,
    LeftIndexDistal,
    LeftThumbMetacarpal,
    RightLowerArm,
    LeftMiddleDistal,
    RightUpperLeg,
    LeftToes,
    LeftThumbDistal,
    RightShoulder,
    RightThumbMetacarpal,
    Spine,
    LeftLowerLeg,
    LeftShoulder,
    LeftUpperArm,
    UpperChest,
    RightToes,
    RightIndexDistal,
    LeftMiddleProximal,
    LeftRingProximal,
    LeftRingDistal,
    LeftThumbProximal,
    LeftIndexIntermediate,
    LeftLittleProximal,
    LeftLittleDistal,
    RightHand,
    RightLittleProximal,
    LeftRingIntermediate,
    RightIndexIntermediate,
    Chest,
    LeftHand,
    RightLittleIntermediate,
    RightFoot,
    RightLowerLeg,
    LeftLittleIntermediate,
    LeftLowerArm,
    RightLittleDistal,
    RightMiddleIntermediate,
    RightMiddleProximal,
    RightThumbProximal,
    Neck,
    Jaw,
    Head,
    LeftEye,
    RightEye,
    LeftMiddleIntermediate,
    RightRingDistal,
    RightIndexProximal,
    RightMiddleDistal,
);

define_root_data_ops!(
    VrmPath,
    RestWorldTransform,
    LookAtProperties,
    ConstraintExecutionOrder,
    VrmExpressionWeights,
    EffectiveExpressionWeights,
    ExpressionMorphBinds,
    ExpressionSettings,
    AnimationTargetId,
    AnimatedBy,
);

/// Root identity runtime: the resolver, the attach observer and the lifecycle
/// bookkeeping.
///
/// Added by [`VrmPlugin`](crate::vrm::VrmPlugin).
pub(crate) struct VrmInstancePlugin;

impl Plugin for VrmInstancePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<VrmHandle>()
            .register_type::<VrmSceneRoot>()
            .add_systems(Update, (rearm_pending_vrm, invalidate_dead_rigs))
            .add_systems(
                SpawnScene,
                resolve_vrm_scene.before(SceneSpawnerSystems::WorldInstanceSpawn),
            )
            .add_observer(attach_vrm_instance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::detach::VrmDetachPlugin;
    use bevy::animation::AnimationPlayer;
    use bevy::world_serialization::{
        DynamicWorld, world_instance_spawner, world_instance_spawner_system,
    };

    /// Counts every `Insert` of `Initialized` on an entity that carries `Vrm`.
    #[derive(Resource, Default)]
    struct Readiness(Vec<Entity>);

    /// A scene shaped like the one the load-time handler builds: a scene root
    /// carrying [`VrmSceneRoot`] plus the whole root inventory, one humanoid bone
    /// holder, and a `root bone -> head` hierarchy.
    ///
    /// `self_player` puts the [`AnimationPlayer`] and the root's [`AnimatedBy`]
    /// on the scene root itself — the no-`hips` fallback — instead of on the root
    /// bone.
    fn vrm_scene(self_player: bool) -> (WorldAsset, Entity, Entity) {
        let mut world = World::new();
        let root_bone = world.spawn_empty().id();
        let head = world
            .spawn((VrmBone::from("head"), ChildOf(root_bone)))
            .id();
        let root = world
            .spawn((
                VrmSceneRoot,
                VrmPath::new("vrm/Test.vrm"),
                RestWorldTransform(GlobalTransform::IDENTITY),
                LookAtProperties::default(),
                ConstraintExecutionOrder(Vec::new()),
                VrmExpressionWeights::default(),
                EffectiveExpressionWeights::default(),
                ExpressionMorphBinds::default(),
                ExpressionSettings::default(),
                vrm_root_animation_target(),
                HeadBoneEntity(head),
            ))
            .id();
        world.entity_mut(root_bone).insert(ChildOf(root));
        if self_player {
            // The no-`hips` fallback: the player is the scene root itself, and
            // the synthetic target is animated by it.
            world
                .entity_mut(root)
                .insert((AnimationPlayer::default(), AnimatedBy(root)));
        } else {
            world.entity_mut(root).insert(AnimatedBy(root_bone));
        }
        (WorldAsset::new(world), root, head)
    }

    /// A root inventory scene with two optional holders, so a swap can drop one.
    fn vrm_scene_with_neck() -> WorldAsset {
        let mut world = World::new();
        let root_bone = world.spawn_empty().id();
        let head = world
            .spawn((VrmBone::from("head"), ChildOf(root_bone)))
            .id();
        let neck = world
            .spawn((VrmBone::from("neck"), ChildOf(root_bone)))
            .id();
        let root = world
            .spawn((
                VrmSceneRoot,
                VrmPath::new("vrm/Neck.vrm"),
                RestWorldTransform(GlobalTransform::IDENTITY),
                ConstraintExecutionOrder(Vec::new()),
                vrm_root_animation_target(),
                AnimatedBy(root_bone),
                HeadBoneEntity(head),
                NeckBoneEntity(neck),
            ))
            .id();
        world.entity_mut(root_bone).insert(ChildOf(root));
        WorldAsset::new(world)
    }

    /// A scene that is *not* a VRM: no [`VrmSceneRoot`] anywhere.
    fn plain_scene() -> WorldAsset {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        world.spawn(ChildOf(root));
        WorldAsset::new(world)
    }

    /// A `Gltf` asset exposing exactly one scene.
    fn gltf_with_scene(scene: Handle<WorldAsset>) -> Gltf {
        Gltf {
            scenes: vec![scene],
            named_scenes: Default::default(),
            meshes: Vec::new(),
            named_meshes: Default::default(),
            materials: Vec::new(),
            named_materials: Default::default(),
            nodes: Vec::new(),
            named_nodes: Default::default(),
            skins: Vec::new(),
            named_skins: Default::default(),
            default_scene: None,
            // `bevy_animation` is an unconditional dependency of this crate
            // (see `Cargo.toml`), so these fields are always present on `Gltf`.
            animations: Vec::new(),
            named_animations: Default::default(),
            source: None,
        }
    }

    /// Adds a scene and a `Gltf` that points at it, returning the handle.
    fn add_vrm(
        app: &mut App,
        scene: WorldAsset,
    ) -> Handle<Gltf> {
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(scene);
        app.world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(scene))
    }

    /// A headless app driving the real [`WorldInstanceSpawner`] plus this
    /// module's resolver, attach observer and detach plugin.
    ///
    /// Every component the fixture scene carries is registered: bevy's spawner
    /// fails (and panics) on an unregistered component in a `WorldAsset`.
    fn instance_app() -> App {
        let mut app = test_app();
        app.init_asset::<WorldAsset>()
            .init_asset::<DynamicWorld>()
            .init_asset::<Gltf>()
            .init_resource::<WorldInstanceSpawner>()
            .init_resource::<Readiness>()
            .register_type::<VrmHandle>()
            .register_type::<VrmSceneRoot>()
            .register_type::<Vrm>()
            .register_type::<Initialized>()
            .register_type::<VrmBone>()
            .register_type::<VrmPath>()
            .register_type::<RestWorldTransform>()
            .register_type::<LookAtProperties>()
            .register_type::<ConstraintExecutionOrder>()
            .register_type::<VrmExpressionWeights>()
            .register_type::<EffectiveExpressionWeights>()
            .register_type::<ExpressionMorphBinds>()
            .register_type::<ExpressionSettings>()
            .register_type::<AnimationTargetId>()
            .register_type::<AnimatedBy>()
            .register_type::<AnimationPlayer>()
            .register_type::<HeadBoneEntity>()
            .register_type::<NeckBoneEntity>()
            .register_type::<ChildOf>()
            .register_type::<Children>()
            .add_plugins(VrmDetachPlugin)
            .add_systems(Update, (rearm_pending_vrm, invalidate_dead_rigs))
            .add_systems(
                SpawnScene,
                (
                    resolve_vrm_scene,
                    (world_instance_spawner, world_instance_spawner_system).chain(),
                )
                    .chain(),
            )
            .add_observer(attach_vrm_instance)
            .add_observer(
                |insert: On<Insert, Initialized>,
                 vrms: Query<(), With<Vrm>>,
                 mut readiness: ResMut<Readiness>| {
                    let entity = insert.event_target();
                    if vrms.contains(entity) {
                        readiness.0.push(entity);
                    }
                },
            );
        app
    }

    /// Spawns a managed VRM and drives it to readiness.
    fn spawn_ready(
        app: &mut App,
        gltf: Handle<Gltf>,
    ) -> Entity {
        let entity = app
            .world_mut()
            .spawn((
                VrmHandle(gltf),
                Name::new("avatar"),
                Transform::from_xyz(0.0, 0.0, 1.0),
            ))
            .id();
        for _ in 0..6 {
            app.update();
        }
        entity
    }

    fn scene_root_of(
        app: &App,
        entity: Entity,
    ) -> Entity {
        app.world()
            .get::<VrmInstanceState>(entity)
            .expect("the owner records its attached scene root")
            .scene_root
    }

    /// The spawned entity keeps its id and its application components, and gains
    /// the whole root inventory; the generation starts at 1.
    #[test]
    fn a_managed_handle_keeps_its_identity_and_adopts_the_root_data() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);

        assert!(app.world().get_entity(vrm).is_ok(), "the id is stable");
        assert!(app.world().get::<Vrm>(vrm).is_some());
        assert!(app.world().get::<Initialized>(vrm).is_some());
        assert_eq!(
            app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
            Some(1),
            "the first attach is generation 1"
        );
        // Application components survive.
        assert_eq!(
            app.world().get::<Transform>(vrm).map(|t| t.translation),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );
        assert_eq!(
            app.world().get::<Name>(vrm).map(Name::as_str),
            Some("avatar")
        );
        // The root inventory moved.
        assert!(app.world().get::<VrmPath>(vrm).is_some());
        assert!(app.world().get::<RestWorldTransform>(vrm).is_some());
        let head = app
            .world()
            .get::<HeadBoneEntity>(vrm)
            .expect("the head holder moved to the owner")
            .0;
        assert_eq!(
            app.world().get::<VrmBone>(head),
            Some(&VrmBone::from("head")),
            "the moved holder names a live bone"
        );
        assert!(app.world().get::<AnimationTargetId>(vrm).is_some());
        assert!(app.world().get::<AnimatedBy>(vrm).is_some());

        // The internal scene root keeps the hierarchy and the marker, and no
        // longer carries the transferred target.
        let root = scene_root_of(&app, vrm);
        assert_ne!(root, vrm);
        assert!(app.world().get::<VrmSceneRoot>(root).is_some());
        assert!(app.world().get::<AnimatedBy>(root).is_none());
        assert!(app.world().get::<Children>(root).is_some());
        assert!(app.world().get::<Vrm>(root).is_none());
        success!()
    }

    /// The synthetic target and `AnimatedBy` move even when the player is the
    /// scene root itself; the player stays behind, so the reference is valid.
    #[test]
    fn a_self_scene_root_player_keeps_its_player_and_moves_animated_by() -> TestResult {
        let mut app = instance_app();
        let (scene, _, _) = vrm_scene(true);
        let gltf = add_vrm(&mut app, scene);
        let vrm = spawn_ready(&mut app, gltf);
        let root = scene_root_of(&app, vrm);

        assert!(app.world().get::<AnimationPlayer>(root).is_some());
        assert!(
            app.world().get::<AnimationPlayer>(vrm).is_none(),
            "the player is kept on the internal scene root"
        );
        assert_eq!(
            app.world()
                .get::<AnimatedBy>(vrm)
                .map(|animated| animated.0),
            Some(root),
            "`AnimatedBy` names the scene-root player, and the reference survives the move"
        );
        assert_eq!(
            app.world().get::<AnimationTargetId>(vrm).copied(),
            Some(vrm_root_animation_target())
        );
        success!()
    }

    /// `Initialized` is inserted exactly once per attach; extra frames do not
    /// re-insert it.
    #[test]
    fn readiness_fires_once_per_attach() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);
        assert_eq!(
            app.world().resource::<Readiness>().0,
            vec![vrm],
            "one `Insert<Initialized>` on the managed root"
        );
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Readiness>().0,
            vec![vrm],
            "a resolved avatar is not re-initialized every frame"
        );
        success!()
    }

    /// Two avatars of one asset are independent: each owns its own holders and
    /// bones.
    #[test]
    fn two_instances_get_independent_holders() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let first = spawn_ready(&mut app, gltf.clone());
        let second = spawn_ready(&mut app, gltf);

        let first_head = app.world().get::<HeadBoneEntity>(first).unwrap().0;
        let second_head = app.world().get::<HeadBoneEntity>(second).unwrap().0;
        assert_ne!(first_head, second_head, "each instance owns its bones");
        assert!(app.world().get::<VrmBone>(first_head).is_some());
        assert!(app.world().get::<VrmBone>(second_head).is_some());
        assert_ne!(scene_root_of(&app, first), scene_root_of(&app, second));
        success!()
    }

    /// A swap to a model without an optional bone drops the old holder; the
    /// shared ones move to the new bone and the generation advances.
    #[test]
    fn a_swap_drops_optional_holders_and_bumps_the_generation() -> TestResult {
        let mut app = instance_app();
        let with_neck = add_vrm(&mut app, vrm_scene_with_neck());
        let without_neck = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, with_neck);
        assert!(app.world().get::<NeckBoneEntity>(vrm).is_some());

        let old_head = app.world().get::<HeadBoneEntity>(vrm).unwrap().0;
        app.world_mut()
            .entity_mut(vrm)
            .get_mut::<VrmHandle>()
            .unwrap()
            .0 = without_neck;
        for _ in 0..6 {
            app.update();
        }

        assert!(
            app.world().get::<NeckBoneEntity>(vrm).is_none(),
            "the new model has no neck holder, so the old one is gone"
        );
        let new_head = app.world().get::<HeadBoneEntity>(vrm).unwrap().0;
        assert_ne!(new_head, old_head, "the holder now names the new bone");
        assert!(app.world().get::<VrmBone>(new_head).is_some());
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(2));
        assert!(app.world().get::<Initialized>(vrm).is_some());
        success!()
    }

    /// A swap to a non-VRM scene invalidates the rig: no `Vrm`, no
    /// `Initialized`, no stale holder, but the handle stays.
    #[test]
    fn a_swap_to_a_non_vrm_invalidates_the_rig() -> TestResult {
        let mut app = instance_app();
        let vrms = add_vrm(&mut app, vrm_scene(false).0);
        let plain_world = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(plain_scene());
        let plain = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(plain_world));
        let vrm = spawn_ready(&mut app, vrms);
        let head = app.world().get::<HeadBoneEntity>(vrm).unwrap().0;
        assert!(app.world().get::<VrmBone>(head).is_some());

        app.world_mut()
            .entity_mut(vrm)
            .get_mut::<VrmHandle>()
            .unwrap()
            .0 = plain;
        for _ in 0..6 {
            app.update();
        }

        assert!(app.world().get::<Vrm>(vrm).is_none());
        assert!(app.world().get::<Initialized>(vrm).is_none());
        assert!(app.world().get::<HeadBoneEntity>(vrm).is_none());
        assert!(
            app.world().get::<VrmHandle>(vrm).is_some(),
            "the handle stays"
        );
        assert!(app.world().get_entity(vrm).is_ok());
        success!()
    }

    /// Repeated swaps settle on the last handle with a strictly newer
    /// generation each time.
    #[test]
    fn repeated_swaps_advance_the_generation() -> TestResult {
        let mut app = instance_app();
        let first = add_vrm(&mut app, vrm_scene(false).0);
        let second = add_vrm(&mut app, vrm_scene_with_neck());
        let vrm = spawn_ready(&mut app, first.clone());

        for (expected, handle) in [(2, second.clone()), (3, first.clone()), (4, second)] {
            app.world_mut()
                .entity_mut(vrm)
                .get_mut::<VrmHandle>()
                .unwrap()
                .0 = handle;
            for _ in 0..6 {
                app.update();
            }
            assert_eq!(
                app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
                Some(expected)
            );
            assert!(app.world().get::<Initialized>(vrm).is_some());
        }
        success!()
    }

    /// The scene `WorldAssetRoot` is not inserted until the scene `WorldAsset`
    /// actually exists, so `remove`/detach before that is a real cancellation.
    #[test]
    fn the_world_asset_root_is_gated_on_the_scene_asset() -> TestResult {
        let mut app = instance_app();
        let scene_handle = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .reserve_handle();
        let gltf = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(scene_handle.clone()));

        let vrm = app.world_mut().spawn(VrmHandle(gltf)).id();
        for _ in 0..4 {
            app.update();
        }
        assert!(
            app.world().get::<WorldAssetRoot>(vrm).is_none(),
            "an unloaded scene asset must not queue a spawn"
        );
        assert!(app.world().get::<Initialized>(vrm).is_none());

        app.world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .insert(&scene_handle, vrm_scene(false).0)
            .unwrap();
        for _ in 0..6 {
            app.update();
        }
        assert!(app.world().get::<WorldAssetRoot>(vrm).is_some());
        assert!(app.world().get::<Initialized>(vrm).is_some());
        success!()
    }

    /// Detaching before the asset loads cancels the spawn; adding the asset
    /// afterwards does not resurrect the avatar.
    #[test]
    fn a_detach_before_the_asset_loads_cancels_the_spawn() -> TestResult {
        let mut app = instance_app();
        let scene_handle = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .reserve_handle();
        let gltf = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(scene_handle.clone()));
        let vrm = app.world_mut().spawn(VrmHandle(gltf)).id();
        app.update();

        app.world_mut()
            .commands()
            .entity(vrm)
            .trigger(RequestDetachVrm);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().get::<VrmHandle>(vrm).is_none());

        app.world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .insert(&scene_handle, vrm_scene(false).0)
            .unwrap();
        for _ in 0..4 {
            app.update();
        }
        assert!(app.world().get::<Vrm>(vrm).is_none());
        assert!(app.world().get::<WorldAssetRoot>(vrm).is_none());
        assert!(app.world().get_entity(vrm).is_ok());
        success!()
    }

    /// A raw `WorldAssetRoot` spawn cancelled before its asset exists: bevy keeps
    /// the queued spawn across `unregister_instance`, so when the asset finally
    /// arrives the late instance still appears and the global observer removes it.
    #[test]
    fn a_raw_pending_detach_cleans_a_late_instance() -> TestResult {
        let mut app = instance_app();
        let scene_handle = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .reserve_handle();
        let owner = app
            .world_mut()
            .spawn(WorldAssetRoot(scene_handle.clone()))
            .id();
        for _ in 0..3 {
            app.update();
        }
        assert!(
            app.world().get::<Children>(owner).is_none(),
            "the asset is not loaded, so no instance exists yet"
        );

        // Cancel by triggering on the owner (the raw-pending shape).
        app.world_mut()
            .commands()
            .entity(owner)
            .trigger(RequestDetachVrm);
        for _ in 0..2 {
            app.update();
        }
        assert!(app.world().get::<WorldAssetRoot>(owner).is_none());

        // The asset arrives afterwards: the engine's queued spawn runs anyway, and
        // the observer scoped to the instance has to clean it up.
        app.world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .insert(&scene_handle, vrm_scene(false).0)
            .unwrap();
        for _ in 0..6 {
            app.update();
        }

        assert!(
            app.world().get::<Children>(owner).is_none(),
            "the late raw instance must be cleaned up, not left attached"
        );
        assert!(
            app.world().get::<WorldInstance>(owner).is_none(),
            "the owner must not keep naming a despawned instance"
        );
        assert!(app.world().get_entity(owner).is_ok());
        success!()
    }

    /// Detach after readiness strips the rig and children, keeps the root alive,
    /// and re-inserting a handle re-attaches onto the same entity.
    #[test]
    fn a_detach_then_reattach_keeps_the_root() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf.clone());
        let scene_root = scene_root_of(&app, vrm);

        app.world_mut()
            .commands()
            .entity(vrm)
            .trigger(RequestDetachVrm);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().get_entity(vrm).is_ok());
        assert!(app.world().get::<Vrm>(vrm).is_none());
        assert!(app.world().get::<Initialized>(vrm).is_none());
        assert!(app.world().get::<VrmHandle>(vrm).is_none());
        assert!(app.world().get_entity(scene_root).is_err());

        // Re-attach: the same entity is reused as the VRM.
        app.world_mut().entity_mut(vrm).insert(VrmHandle(gltf));
        for _ in 0..6 {
            app.update();
        }
        assert!(app.world().get::<Vrm>(vrm).is_some());
        assert!(app.world().get::<Initialized>(vrm).is_some());
        assert!(app.world().get::<HeadBoneEntity>(vrm).is_some());
        success!()
    }

    /// Hot reload reuses the `InstanceId`; the observer still finds the new
    /// scene root and attaches, incrementing the generation.
    #[test]
    fn a_hot_reload_reattaches_the_same_instance_id() -> TestResult {
        let mut app = instance_app();
        let scene_handle = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .reserve_handle();
        let gltf = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(scene_handle.clone()));
        let vrm = app.world_mut().spawn(VrmHandle(gltf)).id();

        app.world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .insert(&scene_handle, vrm_scene(false).0)
            .unwrap();
        for _ in 0..6 {
            app.update();
        }
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(1));
        let first_root = scene_root_of(&app, vrm);

        // Replace the asset: bevy despawns the instance and re-instantiates it
        // under the same `InstanceId`.
        app.world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .insert(&scene_handle, vrm_scene_with_neck())
            .unwrap();
        for _ in 0..8 {
            app.update();
        }

        assert_eq!(
            app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
            Some(2),
            "the reload re-attached and advanced the generation"
        );
        let second_root = scene_root_of(&app, vrm);
        assert_ne!(first_root, second_root);
        assert!(app.world().get_entity(first_root).is_err());
        assert!(app.world().get::<NeckBoneEntity>(vrm).is_some());
        assert!(app.world().get::<Initialized>(vrm).is_some());
        success!()
    }

    /// A managed owner with no live scene root is invalidated without firing on
    /// a newer rig.
    #[test]
    fn a_dead_scene_root_invalidates_only_its_own_rig() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);
        let root = scene_root_of(&app, vrm);

        app.world_mut().despawn(root);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().get::<Vrm>(vrm).is_none());
        assert!(app.world().get::<Initialized>(vrm).is_none());
        assert!(app.world().get::<VrmHandle>(vrm).is_some());
        success!()
    }
}
