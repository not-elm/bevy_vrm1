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
//! ├─ scene root (internal) : VrmSceneRoot, Name, Transform, skeleton,
//! │                          AnimationPlayer (on the root bone)
//! └─ .vrma children spawned by the user
//! ```
//!
//! The internal scene root keeps its [`Transform`]/[`GlobalTransform`], its
//! hierarchy, its [`Name`] and the player, because those are what `bevy_gltf` built
//! and moving its transform would overwrite the user's placement. The [`Name`]
//! (`meta.name`) is copied onto a user entity that has none.
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
//! cancelled spawn is cleaned up by the global [`WorldInstanceReady`] observer. A
//! handle that failed to load, or whose asset exposes no scene, stops being
//! retried instead of pending forever ([`classify_pending`]) until its asset is
//! reloaded ([`rearm_pending_vrm`]). Re-resolving an
//! unchanged non-VRM handle does not re-insert `WorldAssetRoot`, so the spawner
//! (which reacts to `Changed<WorldAssetRoot>`) does not respawn it on every
//! re-arm. Every deferred mutation re-checks the live world before acting, so a
//! `WorldInstanceReady` observed by user code that detaches or re-targets the
//! owner in between cannot resurrect `Vrm` on a cancelled root.
//!
//! # Lifecycle bookkeeping
//!
//! [`VrmGeneration`] is bumped on every successful attach and is **kept across an
//! invalidation**, so a preserved `.vrma` that recorded the old generation always
//! sees a newer one and retargets instead of reusing a stale bake.
//! [`VrmInstanceState`] records the owner's current internal scene root, which
//! [`invalidate_dead_rigs`] uses to invalidate a rig whose scene root died
//! without a replacement (a de-registered instance, a hot reload to a non-VRM),
//! while a newer rig — whose recorded root is alive — is never touched by a
//! delayed cleanup. Every deferred attach/invalidate revalidates the instance or
//! root it was queued for, and an attach to the already-recorded root is a no-op,
//! so a duplicate `WorldInstanceReady` cannot move root data twice. The attach
//! observer is scoped to `ready.instance_id`, and ignores a ready for a
//! superseded instance by comparing against the entity's [`WorldInstance`].

use crate::macros::marker_component;
use crate::prelude::*;
use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::app::{App, Plugin, SceneSpawnerSystems, SpawnScene, Update};
use bevy::asset::{AssetEvent, AssetId, AssetServer, Assets, Handle};
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::{DeferredWorld, World};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::world_serialization::{
    DynamicWorldRoot, InstanceId, WorldAsset, WorldAssetRoot, WorldInstance, WorldInstanceReady,
    WorldInstanceSpawner,
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

/// The owner's [`Name`] was copied from the model at attach, not supplied by
/// the application, so a swap replaces it and a detach removes it.
///
/// It holds the copied value: a `Name` the application writes afterwards
/// differs from it, and is then the application's and kept.
#[derive(Component, Debug, Clone)]
pub(crate) struct NameFromModel(Name);

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

/// Re-arms [`PendingVrmScene`] whenever a [`VrmHandle`] changes or its asset is
/// (re)loaded.
///
/// The `on_insert` hook only runs on a structural insertion, so a handle mutated
/// in place (`*handle = VrmHandle(..)` / `handle.0 = ..`) would otherwise never
/// be re-resolved. Running on `Changed` — which also covers the insertion case —
/// makes both paths the same. The asset events retry a handle the resolver gave
/// up on: a failed or scene-less file that is fixed and reloaded arrives under
/// the same, unchanged handle.
fn rearm_pending_vrm(
    mut commands: Commands,
    mut gltf_events: MessageReader<AssetEvent<Gltf>>,
    changed: Query<Entity, (Changed<VrmHandle>, Without<PendingVrmScene>)>,
    idle: Query<(Entity, &VrmHandle), Without<PendingVrmScene>>,
) {
    for entity in &changed {
        commands.entity(entity).insert(PendingVrmScene);
    }
    let loaded: Vec<AssetId<Gltf>> = gltf_events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    for (entity, handle) in &idle {
        if loaded.contains(&handle.0.id()) {
            commands.entity(entity).insert(PendingVrmScene);
        }
    }
}

/// What one pending [`VrmHandle`] resolves to this frame.
enum PendingOutcome {
    /// The scene `WorldAsset` exists: install (or keep) the `WorldAssetRoot`.
    Ready(Handle<WorldAsset>),
    /// The asset — or its scene sub-asset — is still loading: retry next frame.
    Loading,
    /// The load failed, or the asset exposes no scene at all. Retrying can never
    /// succeed, so the pending marker is dropped instead of spinning forever.
    Unresolvable,
}

/// Classifies one pending handle against the loaded assets.
///
/// The old rig, if any, is deliberately left untouched while a new asset is
/// `Loading`: that is the documented "keep the last valid model until the new
/// one is ready" policy. A definitive failure (`Failed`) or an asset with no
/// scene yields [`PendingOutcome::Unresolvable`], so a broken handle cannot leave
/// a permanent pending loop behind.
fn classify_pending(
    handle: &Handle<Gltf>,
    gltfs: &Assets<Gltf>,
    world_assets: Option<&Assets<WorldAsset>>,
    asset_server: Option<&AssetServer>,
) -> PendingOutcome {
    let Some(gltf) = gltfs.get(handle) else {
        // Not in `Assets<Gltf>` yet: either still loading or it failed.
        return if asset_server.is_some_and(|server| server.load_state(handle.id()).is_failed()) {
            PendingOutcome::Unresolvable
        } else {
            PendingOutcome::Loading
        };
    };
    // The `Gltf` loaded but exposes no scene: no VRM can ever be instantiated
    // from it.
    let Some(scene) = gltf.scenes.first() else {
        return PendingOutcome::Unresolvable;
    };
    let Some(world_assets) = world_assets else {
        return PendingOutcome::Loading;
    };
    if world_assets.contains(scene.id()) {
        return PendingOutcome::Ready(scene.clone());
    }
    // The scene exists on the `Gltf` but its `WorldAsset` is not loaded yet. Only
    // a `Failed` load is fatal; otherwise it is still on its way.
    if asset_server.is_some_and(|server| server.load_state(scene.id()).is_failed()) {
        PendingOutcome::Unresolvable
    } else {
        PendingOutcome::Loading
    }
}

/// Resolves every pending [`VrmHandle`] to its scene, gated on the scene
/// [`WorldAsset`] actually being loaded.
///
/// Runs in `SpawnScene` immediately before bevy's own `WorldInstanceSpawner`
/// pass and touches the world directly (an exclusive system), so the
/// [`WorldAssetRoot`](bevy::world_serialization::WorldAssetRoot) it inserts is
/// seen by that pass in the same frame. The re-arming pass
/// ([`rearm_pending_vrm`]) runs earlier in the same `SpawnScene` chain, so a
/// `VrmHandle` mutated anywhere in `Update` is resolved in the same frame.
///
/// A handle whose `Gltf` is *loading*, or whose scene `WorldAsset` is not in
/// [`Assets<WorldAsset>`] yet, stays pending and is retried next frame; the
/// [`WorldAssetRoot`] is never inserted for an asset that does not exist, which
/// is what keeps a later detach a real cancellation rather than a stranded queue
/// entry. A definitive failure stops the retry loop ([`PendingOutcome`]).
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

    let mut ready: Vec<(Entity, Handle<WorldAsset>)> = Vec::new();
    let mut unresolvable: Vec<Entity> = Vec::new();
    {
        let Some(gltfs) = world.get_resource::<Assets<Gltf>>() else {
            return;
        };
        let world_assets = world.get_resource::<Assets<WorldAsset>>();
        let asset_server = world.get_resource::<AssetServer>();
        for (entity, handle) in &pending {
            match classify_pending(handle, gltfs, world_assets, asset_server) {
                PendingOutcome::Ready(scene) => ready.push((*entity, scene)),
                PendingOutcome::Unresolvable => unresolvable.push(*entity),
                PendingOutcome::Loading => {}
            }
        }
    }

    for entity in unresolvable {
        if world.get_entity(entity).is_ok() {
            world.entity_mut(entity).remove::<PendingVrmScene>();
        }
    }

    // Whether each target scene is a VRM scene at all. A scene with no
    // `VrmSceneRoot` has nothing to attach, so re-resolving an unchanged non-VRM
    // handle must **not** re-insert `WorldAssetRoot`: the spawner responds to
    // `Changed<WorldAssetRoot>` and would respawn the instance on every re-arm,
    // forever. Only a genuine VRM scene needs the dead-rig recovery path.
    let scene_has_vrm: Vec<bool> = {
        let world_assets = world.get_resource::<Assets<WorldAsset>>();
        ready
            .iter()
            .map(|(_, scene)| {
                world_assets.is_some_and(|assets| {
                    assets.get(scene.id()).is_some_and(|asset| {
                        asset
                            .world
                            .iter_entities()
                            .any(|entity| entity.contains::<VrmSceneRoot>())
                    })
                })
            })
            .collect()
    };

    for ((entity, scene), scene_has_vrm) in ready.into_iter().zip(scene_has_vrm) {
        if world.get_entity(entity).is_err() {
            continue;
        }
        world.entity_mut(entity).remove::<PendingVrmScene>();
        // Already resolved: the `WorldAssetRoot` still names this exact scene and
        // either the rig is alive or the scene is not a VRM at all (so there is
        // nothing to recover). Re-inserting would spuriously respawn the
        // instance. A VRM scene whose recorded rig died *is* re-inserted, so the
        // same handle recovers instead of staying broken.
        let already_resolved = world
            .get::<WorldAssetRoot>(entity)
            .is_some_and(|root| root.0.id() == scene.id())
            && (!scene_has_vrm
                || world
                    .get::<VrmInstanceState>(entity)
                    .is_some_and(|state| world.get_entity(state.scene_root).is_ok()));
        if !already_resolved {
            world.entity_mut(entity).insert(WorldAssetRoot(scene));
        }
    }
}

/// The single global [`WorldInstanceReady`] observer.
///
/// Scoped to `ready.instance_id` (the new instance's entities) and to the
/// entity's current [`WorldInstance`], so a ready for a superseded instance is
/// ignored rather than mistaken for a non-VRM replacement. A superseded
/// instance that still carries a [`VrmSceneRoot`] — a queued raw spawn that
/// landed after its owner moved on — is removed instead of being left as an
/// orphan, but the owner's *current* instance is never touched.
///
/// Every branch only *classifies* the shape here; the actual mutation is
/// deferred into a command that re-checks the live world ([`attach_managed`],
/// [`attach_raw_instance`], [`invalidate_managed`], [`cleanup_stale_instance`]),
/// because a user observer on the same event may detach the owner, change its
/// handle or remove its `WorldAssetRoot` between this event and the command.
/// A stale command must never resurrect `Vrm` on a cancelled root, nor tear
/// down a rig that has since been re-attached.
fn attach_vrm_instance(
    ready: On<WorldInstanceReady>,
    spawner: Option<Res<WorldInstanceSpawner>>,
    managed: Query<(), With<VrmHandle>>,
    raw_owner: Query<
        (),
        (
            With<WorldAssetRoot>,
            Without<VrmHandle>,
            Without<VrmInstanceState>,
            Without<Vrma>,
            Without<VrmaHandle>,
        ),
    >,
    instances: Query<&WorldInstance>,
    dynamic_owner: Query<(), With<DynamicWorldRoot>>,
    scene_roots: Query<(), With<VrmSceneRoot>>,
    mut commands: Commands,
) {
    // A world asset spawned without a parent triggers the event with
    // `Entity::PLACEHOLDER`; nothing crate-managed can be attached there.
    let parent = ready.event_target();
    if parent == Entity::PLACEHOLDER {
        return;
    }

    let scene_root = spawner.as_ref().and_then(|spawner| {
        spawner
            .iter_instance_entities(ready.instance_id)
            .find(|entity| scene_roots.contains(*entity))
    });

    // A ready for an instance that has already been superseded (bevy reuses the
    // `InstanceId` on hot reload, and a stale event can outlive a swap) must not
    // touch the owner's current rig. A crate-managed VRM scene that still landed
    // is removed so it cannot stay as an orphan.
    if instances
        .get(parent)
        .is_ok_and(|world_instance| **world_instance != ready.instance_id)
    {
        if scene_root.is_some() {
            let instance_id = ready.instance_id;
            commands
                .queue(move |world: &mut World| cleanup_stale_instance(world, instance_id, parent));
        }
        return;
    }

    if managed.contains(parent) {
        match scene_root {
            Some(root) => {
                let instance_id = ready.instance_id;
                commands.queue(move |world: &mut World| {
                    attach_managed(world, parent, root, instance_id);
                });
            }
            // The new instance is not a VRM (or has no scene): drop any stale rig
            // so an `Initialized` root never outlives the rig it described. The
            // guard keeps a delayed drop from wiping a newer attach.
            None => {
                let instance_id = ready.instance_id;
                commands.queue(move |world: &mut World| {
                    invalidate_managed(world, parent, InvalidateGuard::Instance(instance_id));
                });
            }
        }
    } else if raw_owner.contains(parent) {
        // Raw `WorldAssetRoot(..)` spawn: the VRM stays on the child, as before.
        // Deferred and re-validated in the live world, so a duplicate ready does
        // not re-fire `Insert<Initialized>` and a stale shape is cleaned up.
        if let Some(root) = scene_root {
            let instance_id = ready.instance_id;
            commands.queue(move |world: &mut World| {
                attach_raw_instance(world, parent, root, instance_id);
            });
        }
    } else if scene_root.is_some() && instances.contains(parent) && !dynamic_owner.contains(parent)
    {
        // The owner's own `WorldAssetRoot` queued this instance and no longer
        // wants it: a spawn that survived a detach. Its entities are removed. An
        // instance bevy spawned for anything else (`spawn_as_child`, a
        // `DynamicWorldRoot`) belongs to the application and is left alone.
        let instance_id = ready.instance_id;
        commands.queue(move |world: &mut World| cleanup_stale_instance(world, instance_id, parent));
    }
}

/// Moves the load-time root data of `scene_root` onto `parent` and marks the
/// attach.
///
/// All of it happens synchronously inside one command, and [`Initialized`] is
/// inserted last, so an `On<Insert, Initialized>` observer always sees a fully
/// populated root. The command revalidates the live world, so a duplicate ready
/// is a no-op, a ready whose instance was superseded cannot move data onto a
/// newer rig, and an owner that was detached or had its `WorldAssetRoot`
/// removed before the command ran is cleaned up instead of resurrecting `Vrm`
/// on a cancelled root.
///
/// [`VrmGeneration`] is read *before* clearing and is **not** removed by
/// [`clear_attached_state`], so the counter keeps increasing across an
/// invalidation. A preserved `.vrma` records the generation it was retargeted
/// for; a reset counter would let it mistake a new rig for the old one and reuse
/// a stale bake.
fn attach_managed(
    world: &mut World,
    parent: Entity,
    scene_root: Entity,
    instance_id: InstanceId,
) {
    if world.get_entity(parent).is_err() || world.get_entity(scene_root).is_err() {
        return;
    }
    // Cancelled or repurposed before this deferred command ran: a detached owner
    // (no `VrmHandle`) or a removed `WorldAssetRoot` means the instance is stale,
    // so it is cleaned up rather than promoted onto the entity.
    if world.get::<VrmHandle>(parent).is_none() || world.get::<WorldAssetRoot>(parent).is_none() {
        cleanup_stale_instance(world, instance_id, parent);
        return;
    }
    // The scene root must still belong to the very instance that became ready.
    if !instance_owns(world, instance_id, scene_root) {
        return;
    }
    // Superseded between the ready event and this deferred command.
    if world
        .get::<WorldInstance>(parent)
        .is_some_and(|world_instance| **world_instance != instance_id)
    {
        return;
    }
    // Idempotent: this exact rig is already attached (a duplicate ready for the
    // same instance must not move the root data a second time).
    if world
        .get::<VrmInstanceState>(parent)
        .is_some_and(|state| state.scene_root == scene_root)
    {
        return;
    }
    let generation = world
        .get::<VrmGeneration>(parent)
        .map_or(1, |previous| previous.0.saturating_add(1));

    // Now that a fresh rig is attached, any data left by a previous one is stale:
    // holders may name despawned bones and an `Initialized` root may point at a
    // rig that no longer exists. Clear before moving the new values in.
    //
    // Only a rig that is still recorded left such data: an invalidation and a
    // detach clear theirs. On the first attach the root-data types present on
    // the entity were put there by the application (a `LookAtProperties` for a
    // model that declares no `lookAt`, say) and must survive, as they did when
    // the model's values were only inserted.
    if world.get::<VrmInstanceState>(parent).is_some() {
        clear_attached_state(world, parent);
    }
    move_root_data(world, scene_root, parent);
    // Every insert and removal above flushed the world's command queue, so an
    // application observer (on `Remove<Initialized>` or `Add<VrmPath>`, say) may
    // already have detached or despawned the owner or the scene. Re-check
    // before each step that touches them again.
    if !attach_still_wanted(world, parent, scene_root) {
        return;
    }
    // The loader names the scene root after `meta.name`. A `Name` the
    // application gave the entity wins. A scene root that is also the root bone
    // (a `hips` listed directly in the glTF scene) carries `Vrm::ROOT_BONE`
    // instead, which is a lookup key rather than the model's name.
    if world.get::<Name>(parent).is_none()
        && let Some(name) = world
            .get::<Name>(scene_root)
            .filter(|name| name.as_str() != Vrm::ROOT_BONE)
            .cloned()
    {
        world
            .entity_mut(parent)
            .insert((name.clone(), NameFromModel(name)));
    }
    if !attach_still_wanted(world, parent, scene_root) {
        return;
    }

    world.entity_mut(parent).insert((
        Vrm,
        VrmGeneration(generation),
        VrmInstanceState { scene_root },
    ));
    // An `On<Add, Vrm>` observer may have detached the owner; `Initialized` must
    // then not be resurrected on it.
    if !attach_still_wanted(world, parent, scene_root)
        || world
            .get::<VrmInstanceState>(parent)
            .is_none_or(|state| state.scene_root != scene_root)
    {
        return;
    }
    // Last, so observers on `On<Insert, Initialized>` see `Vrm`, the generation
    // and every root component already in place.
    world.entity_mut(parent).insert(Initialized);
}

/// Whether the attach in progress may still write to `parent`: the owner and
/// its scene root are alive and the owner is still a managed avatar that wants
/// an instance.
fn attach_still_wanted(
    world: &World,
    parent: Entity,
    scene_root: Entity,
) -> bool {
    world.get_entity(parent).is_ok()
        && world.get_entity(scene_root).is_ok()
        && world.get::<VrmHandle>(parent).is_some()
        && world.get::<WorldAssetRoot>(parent).is_some()
}

/// Attaches the public markers to a raw spawn's child scene root.
///
/// Re-checked against the live world, because a user observer on the same
/// `WorldInstanceReady` may have detached the spawn, turned the owner into a
/// managed avatar or re-targeted it before this command ran. Only a still-raw
/// owner whose current instance still owns `root` gets `Vrm` + `Initialized`; a
/// stale instance is cleaned up, so `Vrm` is never resurrected on a cancelled
/// root and a duplicate ready never re-fires `Insert<Initialized>`.
fn attach_raw_instance(
    world: &mut World,
    owner: Entity,
    root: Entity,
    instance_id: InstanceId,
) {
    if world.get_entity(owner).is_err() || world.get_entity(root).is_err() {
        return;
    }
    let owns = instance_owns(world, instance_id, root);
    // The owner is still the raw `WorldAssetRoot(..)` spawn this event was for:
    // not managed, not a `.vrma`, and still wanting an instance.
    let still_raw = world.get::<WorldAssetRoot>(owner).is_some()
        && world.get::<VrmHandle>(owner).is_none()
        && world.get::<Vrma>(owner).is_none()
        && world.get::<VrmaHandle>(owner).is_none();
    // The ready must still be the owner's current instance.
    let still_current = world
        .get::<WorldInstance>(owner)
        .is_none_or(|world_instance| **world_instance == instance_id);

    if owns && still_raw && still_current {
        if world.get::<Vrm>(root).is_none() {
            world.entity_mut(root).insert((Vrm, Initialized));
        }
        return;
    }
    // The scene is ours but the owner no longer wants it: a cancelled or
    // superseded raw spawn that landed late.
    if owns {
        cleanup_stale_instance(world, instance_id, owner);
    }
}

/// Whether `entity` is still one of the entities the spawner tracks for
/// `instance_id`.
///
/// `true` when there is no spawner to ask (a hand-built app), so the check never
/// blocks a legitimate attach.
fn instance_owns(
    world: &World,
    instance_id: InstanceId,
    entity: Entity,
) -> bool {
    let Some(spawner) = world.get_resource::<WorldInstanceSpawner>() else {
        return true;
    };
    spawner
        .iter_instance_entities(instance_id)
        .any(|candidate| candidate == entity)
}

/// The entities the spawner tracks for `instance_id` (empty without a spawner).
pub(crate) fn instance_entities(
    world: &World,
    instance_id: InstanceId,
) -> Vec<Entity> {
    world
        .get_resource::<WorldInstanceSpawner>()
        .map(|spawner| spawner.iter_instance_entities(instance_id).collect())
        .unwrap_or_default()
}

/// Why an attach-less invalidation is allowed to run.
///
/// Every invalidation is deferred into a command, so between the system that
/// decided it and the command running the world may have moved on. The guard is
/// re-checked in the command: a delayed drop must never remove the components of
/// a newer attach.
#[derive(Debug, Copy, Clone)]
enum InvalidateGuard {
    /// Only touch the owner if its live instance is still this one.
    Instance(InstanceId),
    /// Only touch the owner if it still records this exact internal scene root.
    Root(Entity),
}

/// Removes every attached-rig component from a managed owner, but only if the
/// guard still matches.
///
/// `VrmHandle` is deliberately kept: it is the entity's identity, and this is
/// also the hot-reload-to-a-non-VRM path, where the handle still names the asset
/// the application asked for. `VrmGeneration` is kept too, so the next attach is
/// strictly newer than the invalidated one.
fn invalidate_managed(
    world: &mut World,
    parent: Entity,
    guard: InvalidateGuard,
) {
    if world.get_entity(parent).is_err() {
        return;
    }
    let still_current = match guard {
        InvalidateGuard::Instance(instance_id) => {
            world
                .get::<WorldInstance>(parent)
                .map(|world_instance| **world_instance)
                == Some(instance_id)
        }
        InvalidateGuard::Root(scene_root) => {
            world
                .get::<VrmInstanceState>(parent)
                .map(|state| state.scene_root)
                == Some(scene_root)
        }
    };
    if !still_current {
        return;
    }
    clear_attached_state(world, parent);
}

/// Removes all root data and the attach markers, leaving `VrmHandle` and the
/// monotonic [`VrmGeneration`] in place.
fn clear_attached_state(
    world: &mut World,
    entity: Entity,
) {
    remove_root_data(world, entity);
    remove_model_name(world, entity);
    if let Ok(mut entity) = world.get_entity_mut(entity) {
        entity.remove::<(Vrm, Initialized, VrmInstanceState)>();
    }
}

/// Removes a [`Name`] the attach copied from the model, keeping one the
/// application supplied, including one it wrote over the copied name later.
pub(crate) fn remove_model_name(
    world: &mut World,
    entity: Entity,
) {
    if let Ok(mut owner) = world.get_entity_mut(entity)
        && let Some(NameFromModel(copied)) = owner.take::<NameFromModel>()
        && let Ok(mut owner) = world.get_entity_mut(entity)
        && owner.get::<Name>() == Some(&copied)
    {
        owner.remove::<Name>();
    }
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
    // Only clear the owner's `WorldInstance` if it still names the instance just
    // cleaned up. A raw owner that has already re-targeted carries the *new*
    // instance there, and removing it would strip the live request.
    if world
        .get::<WorldInstance>(parent)
        .is_some_and(|world_instance| **world_instance == instance_id)
    {
        world.entity_mut(parent).remove::<WorldInstance>();
    }
}

/// Invalidates any attached rig whose internal scene root no longer exists.
///
/// A rig can die without a replacement `WorldInstanceReady` (an externally
/// de-registered instance, a scene root despawned directly). The recorded
/// [`VrmInstanceState::scene_root`] identifies *this* rig, and the same root is
/// passed to the deferred invalidation as its guard, so a newer rig — which
/// records a different, live root — is never invalidated by a delayed check.
fn invalidate_dead_rigs(
    mut commands: Commands,
    owners: Query<(Entity, &VrmInstanceState)>,
    scene_roots: Query<(), With<VrmSceneRoot>>,
) {
    for (owner, state) in &owners {
        if scene_roots.contains(state.scene_root) {
            continue;
        }
        let expected_root = state.scene_root;
        commands.queue(move |world: &mut World| {
            invalidate_managed(world, owner, InvalidateGuard::Root(expected_root));
        });
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
        ///
        /// Each take and insert flushes the world's command queue, so either
        /// entity can be despawned by an observer between two moves; a move then
        /// stops instead of panicking.
        pub(crate) fn move_root_data(world: &mut World, from: Entity, to: Entity) {
            $(
                if let Ok(mut source) = world.get_entity_mut(from)
                    && let Some(value) = source.take::<$ty>()
                    && let Ok(mut target) = world.get_entity_mut(to)
                {
                    target.insert(value);
                }
            )*
            move_root_holders(world, from, to);
        }

        /// Removes every root-level component from `entity`.
        ///
        /// Each removal flushes the world's command queue, so the entity can be
        /// despawned by an observer between two of them; the rest is then moot.
        pub(crate) fn remove_root_data(world: &mut World, entity: Entity) {
            $(
                if let Ok(mut entity) = world.get_entity_mut(entity) {
                    entity.remove::<$ty>();
                }
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
                    if let Ok(mut source) = world.get_entity_mut(from)
                        && let Some(holder) = source
                            .take::<crate::vrm::humanoid_bone::prelude::[<$bone BoneEntity>]>()
                        && let Ok(mut target) = world.get_entity_mut(to)
                    {
                        target.insert(holder);
                    }
                )+
            }
        }

        /// Removes every `<Bone>BoneEntity` holder from `entity`.
        pub(crate) fn remove_root_holders(world: &mut World, entity: Entity) {
            paste::paste! {
                $(
                    if let Ok(mut entity) = world.get_entity_mut(entity) {
                        entity.remove::<crate::vrm::humanoid_bone::prelude::[<$bone BoneEntity>]>();
                    }
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
            .add_systems(Update, invalidate_dead_rigs)
            .add_systems(
                SpawnScene,
                // Re-arm after `Update` (and so after any handle mutation there)
                // but before the resolver, which then sees `Changed<VrmHandle>`
                // from the same frame.
                (rearm_pending_vrm, resolve_vrm_scene)
                    .chain()
                    .before(SceneSpawnerSystems::WorldInstanceSpawn),
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
    use crate::vrma::VrmAnimationGraphState;
    use bevy::animation::AnimationPlayer;
    use bevy::world_serialization::{
        DynamicWorld, world_instance_spawner, world_instance_spawner_system,
    };

    /// Counts every `Insert` of `Initialized` on an entity that carries `Vrm`.
    #[derive(Resource, Default)]
    struct Readiness(Vec<Entity>);

    /// A handle to swap onto every managed avatar on the next `Update`.
    #[derive(Resource, Default)]
    struct PendingSwap(Option<Handle<Gltf>>);

    /// Applies [`PendingSwap`] inside `Update`, i.e. as a same-frame in-place
    /// `VrmHandle` mutation, which the re-arm in `SpawnScene` then has to pick up.
    fn apply_pending_swap(
        mut swap: ResMut<PendingSwap>,
        mut handles: Query<&mut VrmHandle>,
    ) {
        let Some(handle) = swap.0.take() else {
            return;
        };
        for mut vrm_handle in &mut handles {
            vrm_handle.0 = handle.clone();
        }
    }

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
        gltf_with_scenes(vec![scene])
    }

    /// A `Gltf` asset exposing no scene at all: a definitive failure to resolve.
    fn gltf_without_scene() -> Gltf {
        gltf_with_scenes(Vec::new())
    }

    /// A `Gltf` asset exposing the given scenes.
    fn gltf_with_scenes(scenes: Vec<Handle<WorldAsset>>) -> Gltf {
        Gltf {
            scenes,
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
            .init_resource::<PendingSwap>()
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
            .register_type::<Name>()
            .register_type::<ChildOf>()
            .register_type::<Children>()
            .add_plugins(VrmDetachPlugin)
            .add_systems(Update, (invalidate_dead_rigs, apply_pending_swap))
            .add_systems(
                SpawnScene,
                (
                    (rearm_pending_vrm, resolve_vrm_scene).chain(),
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

    /// The counter must not reset when a rig is invalidated, or a preserved
    /// `.vrma` that recorded the old generation would mistake the next rig for
    /// the same one and reuse a stale bake.
    #[test]
    fn generation_keeps_increasing_across_invalidation() -> TestResult {
        let mut app = instance_app();
        let first = add_vrm(&mut app, vrm_scene(false).0);
        let second = add_vrm(&mut app, vrm_scene_with_neck());
        let plain_world = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(plain_scene());
        let plain = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(plain_world));

        let vrm = spawn_ready(&mut app, first);
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(1));

        // A -> non-VRM: the rig is invalidated, but the counter must survive.
        app.world_mut()
            .entity_mut(vrm)
            .get_mut::<VrmHandle>()
            .unwrap()
            .0 = plain;
        for _ in 0..6 {
            app.update();
        }
        assert!(app.world().get::<Vrm>(vrm).is_none());
        assert_eq!(
            app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
            Some(1),
            "the generation is kept across invalidation"
        );

        // non-VRM -> B: the new rig must be strictly newer than A.
        app.world_mut()
            .entity_mut(vrm)
            .get_mut::<VrmHandle>()
            .unwrap()
            .0 = second;
        for _ in 0..6 {
            app.update();
        }
        assert!(app.world().get::<Vrm>(vrm).is_some());
        assert_eq!(
            app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
            Some(2),
            "a valid rig after an invalid one is a new generation, not a reset to 1"
        );
        success!()
    }

    /// Destroying the scene root invalidates the rig but keeps the monotonic
    /// generation, and re-inserting the *same* handle recovers a fresh instance
    /// instead of staying broken behind the already-resolved `WorldAssetRoot`.
    #[test]
    fn a_destroyed_rig_recovers_from_the_same_handle() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf.clone());
        let old_root = scene_root_of(&app, vrm);
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(1));

        app.world_mut().despawn(old_root);
        for _ in 0..3 {
            app.update();
        }
        assert!(
            app.world().get::<Vrm>(vrm).is_none(),
            "the dead rig is gone"
        );
        assert_eq!(
            app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
            Some(1),
            "the dead rig still keeps the counter"
        );

        app.world_mut().entity_mut(vrm).insert(VrmHandle(gltf));
        for _ in 0..6 {
            app.update();
        }
        assert!(app.world().get::<Vrm>(vrm).is_some(), "the rig recovers");
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(2));
        assert_ne!(
            scene_root_of(&app, vrm),
            old_root,
            "a genuinely new instance"
        );
        success!()
    }

    /// A duplicate `WorldInstanceReady` for the same instance must not move the
    /// root data again or bump the generation: the scene root keeps its
    /// `VrmSceneRoot`, so without the idempotency guard the observer would attach
    /// a second time.
    #[test]
    fn a_duplicate_ready_does_not_reattach() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(1));
        assert_eq!(app.world().resource::<Readiness>().0, vec![vrm]);

        let instance_id = **app.world().get::<WorldInstance>(vrm).unwrap();
        app.world_mut().commands().trigger(WorldInstanceReady {
            entity: vrm,
            instance_id,
        });
        for _ in 0..3 {
            app.update();
        }

        assert_eq!(
            app.world().get::<VrmGeneration>(vrm).map(|g| g.0),
            Some(1),
            "a duplicate ready must not bump the generation"
        );
        assert_eq!(
            app.world().resource::<Readiness>().0,
            vec![vrm],
            "a duplicate ready must not fire `Insert<Initialized>` again"
        );
        success!()
    }

    /// A `VrmHandle` mutated in place during `Update` is resolved in the same
    /// frame: the re-arm runs in `SpawnScene`, after every `Update` system, so a
    /// late mutation cannot leave the model selection stale for a frame.
    #[test]
    fn a_handle_swapped_in_update_resolves_in_the_same_frame() -> TestResult {
        let mut app = instance_app();
        let first = add_vrm(&mut app, vrm_scene(false).0);
        let second = add_vrm(&mut app, vrm_scene_with_neck());
        let vrm = spawn_ready(&mut app, first);
        assert!(app.world().get::<NeckBoneEntity>(vrm).is_none());

        app.world_mut().resource_mut::<PendingSwap>().0 = Some(second);
        app.update();

        assert!(app.world().get::<NeckBoneEntity>(vrm).is_some());
        assert_eq!(app.world().get::<VrmGeneration>(vrm).map(|g| g.0), Some(2));
        success!()
    }

    /// A scene-less asset is a definitive failure: the pending loop stops instead
    /// of spinning forever, and nothing is left claiming readiness.
    #[test]
    fn a_handle_without_a_scene_stops_pending() -> TestResult {
        let mut app = instance_app();
        let gltf = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_without_scene());
        let vrm = app.world_mut().spawn(VrmHandle(gltf)).id();

        for _ in 0..4 {
            app.update();
        }
        assert!(
            app.world().get::<PendingVrmScene>(vrm).is_none(),
            "a scene-less asset must stop the retry loop"
        );
        assert!(app.world().get::<WorldAssetRoot>(vrm).is_none());
        assert!(app.world().get::<Initialized>(vrm).is_none());
        assert!(app.world().get_entity(vrm).is_ok());
        success!()
    }

    /// The fixture scene with the `Name` the loader gives a VRM scene root.
    fn named_vrm_scene(name: &'static str) -> WorldAsset {
        let (mut scene, root, _) = vrm_scene(false);
        scene.world.entity_mut(root).insert(Name::new(name));
        scene
    }

    /// An owner without a `Name` takes the model's, and follows it across a
    /// swap; a `Name` the application supplied is never replaced.
    #[test]
    fn an_unnamed_owner_takes_the_model_name() -> TestResult {
        let mut app = instance_app();
        let first = add_vrm(&mut app, named_vrm_scene("First"));
        let second = add_vrm(&mut app, named_vrm_scene("Second"));
        let unnamed = app.world_mut().spawn(VrmHandle(first.clone())).id();
        let named = app
            .world_mut()
            .spawn((VrmHandle(first), Name::new("avatar")))
            .id();
        for _ in 0..6 {
            app.update();
        }
        let name = |app: &App, entity: Entity| {
            app.world()
                .get::<Name>(entity)
                .map(|name| name.as_str().to_owned())
        };
        assert_eq!(name(&app, unnamed).as_deref(), Some("First"));
        assert_eq!(name(&app, named).as_deref(), Some("avatar"));

        for entity in [unnamed, named] {
            app.world_mut()
                .entity_mut(entity)
                .get_mut::<VrmHandle>()
                .unwrap()
                .0 = second.clone();
        }
        for _ in 0..6 {
            app.update();
        }
        assert_eq!(name(&app, unnamed).as_deref(), Some("Second"));
        assert_eq!(name(&app, named).as_deref(), Some("avatar"));

        for entity in [unnamed, named] {
            app.world_mut()
                .commands()
                .entity(entity)
                .trigger(RequestDetachVrm);
        }
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(name(&app, unnamed), None, "the model's name goes with it");
        assert_eq!(name(&app, named).as_deref(), Some("avatar"));
        success!()
    }

    /// Every structural change of the attach flushes the world's commands, so an
    /// application observer runs in the middle of it. One that detaches the
    /// avatar as soon as it becomes a `Vrm` must not get `Initialized` back, and
    /// one that despawns it must not make the attach panic.
    #[test]
    fn an_observer_that_tears_the_avatar_down_mid_attach_wins() -> TestResult {
        let mut app = instance_app();
        let handle = add_vrm(&mut app, vrm_scene(false).0);
        let detached = app.world_mut().spawn(VrmHandle(handle.clone())).id();
        let despawned = app.world_mut().spawn(VrmHandle(handle)).id();
        app.add_observer(move |add: On<Add, Vrm>, mut commands: Commands| {
            let entity = add.event_target();
            if entity == detached {
                commands.entity(entity).trigger(RequestDetachVrm);
            } else if entity == despawned {
                commands.entity(entity).despawn();
            }
        });
        for _ in 0..6 {
            app.update();
        }

        assert!(app.world().get::<Initialized>(detached).is_none());
        assert!(app.world().get::<Vrm>(detached).is_none());
        assert!(app.world().get_entity(despawned).is_err());
        success!()
    }

    /// `lookAt` is optional in `VRMC_vrm`. An application that spawns the avatar
    /// with its own `LookAtProperties` for a model that declares none keeps it:
    /// the first attach has no previous rig whose data would be stale.
    #[test]
    fn the_first_attach_keeps_root_data_the_model_does_not_provide() -> TestResult {
        let mut app = instance_app();
        let handle = add_vrm(&mut app, vrm_scene_with_neck());
        let properties = LookAtProperties {
            offset_from_head_bone: [0.0, 0.1, 0.0],
            ..Default::default()
        };
        let vrm = app.world_mut().spawn((VrmHandle(handle), properties)).id();
        for _ in 0..6 {
            app.update();
        }

        assert!(app.world().get::<Initialized>(vrm).is_some());
        assert_eq!(
            app.world()
                .get::<LookAtProperties>(vrm)
                .map(|properties| properties.offset_from_head_bone),
            Some([0.0, 0.1, 0.0])
        );
        success!()
    }

    /// A `Name` the application writes over the copied model name is the
    /// application's from then on: a swap and a detach keep it.
    #[test]
    fn a_name_written_over_the_model_name_is_kept() -> TestResult {
        let mut app = instance_app();
        let first = add_vrm(&mut app, named_vrm_scene("First"));
        let second = add_vrm(&mut app, named_vrm_scene("Second"));
        let vrm = app.world_mut().spawn(VrmHandle(first)).id();
        for _ in 0..6 {
            app.update();
        }
        let name = |app: &App| {
            app.world()
                .get::<Name>(vrm)
                .map(|name| name.as_str().to_owned())
        };
        assert_eq!(name(&app).as_deref(), Some("First"));

        app.world_mut()
            .entity_mut(vrm)
            .insert(Name::new("Player 1"));
        app.world_mut()
            .entity_mut(vrm)
            .get_mut::<VrmHandle>()
            .unwrap()
            .0 = second;
        for _ in 0..6 {
            app.update();
        }
        assert_eq!(name(&app).as_deref(), Some("Player 1"));

        app.world_mut()
            .commands()
            .entity(vrm)
            .trigger(RequestDetachVrm);
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(name(&app).as_deref(), Some("Player 1"));
        success!()
    }

    /// A broken file fixed on disk and hot-reloaded replaces the asset under the
    /// same handle; the owner the resolver gave up on has to pick it up.
    #[test]
    fn an_unresolvable_handle_is_retried_after_a_reload() -> TestResult {
        let mut app = instance_app();
        let gltf = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_without_scene());
        let vrm = app.world_mut().spawn(VrmHandle(gltf.clone())).id();
        for _ in 0..4 {
            app.update();
        }
        assert!(app.world().get::<PendingVrmScene>(vrm).is_none());

        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(vrm_scene(false).0);
        app.world_mut()
            .resource_mut::<Assets<Gltf>>()
            .insert(&gltf, gltf_with_scene(scene))
            .unwrap();
        for _ in 0..6 {
            app.update();
        }

        assert!(app.world().get::<Initialized>(vrm).is_some());
        success!()
    }

    /// A VRM scene bevy spawned without a `WorldAssetRoot` on the parent was
    /// never queued by this crate, so it is not a cancelled spawn.
    #[test]
    fn a_vrm_scene_spawned_as_a_child_is_kept() -> TestResult {
        let mut app = instance_app();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(vrm_scene(false).0);
        let parent = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<WorldInstanceSpawner>()
            .spawn_as_child(scene, parent);
        for _ in 0..4 {
            app.update();
        }

        let root = app
            .world()
            .get::<Children>(parent)
            .expect("the instance survives")
            .iter()
            .next()
            .unwrap();
        assert!(app.world().get::<VrmSceneRoot>(root).is_some());
        success!()
    }

    #[test]
    fn a_vrm_scene_spawned_through_a_dynamic_world_root_is_kept() -> TestResult {
        let mut app = instance_app();
        let dynamic = {
            let registry = app.world().resource::<AppTypeRegistry>().read();
            DynamicWorld::from_world_asset(&vrm_scene(false).0, &registry)
        };
        let dynamic = app
            .world_mut()
            .resource_mut::<Assets<DynamicWorld>>()
            .add(dynamic);
        let owner = app.world_mut().spawn(DynamicWorldRoot(dynamic)).id();
        for _ in 0..4 {
            app.update();
        }

        assert!(app.world().get::<WorldInstance>(owner).is_some());
        let root = app
            .world()
            .get::<Children>(owner)
            .expect("the instance survives")
            .iter()
            .next()
            .unwrap();
        assert!(app.world().get::<VrmSceneRoot>(root).is_some());
        success!()
    }

    /// `RequestDetachVrm` on the owner of a spawned scene that is not a VRM (a
    /// level, a prop) is not a VRM detach.
    #[test]
    fn detaching_a_non_vrm_raw_owner_is_a_no_op() -> TestResult {
        let mut app = instance_app();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(plain_scene());
        let owner = app.world_mut().spawn(WorldAssetRoot(scene)).id();
        for _ in 0..3 {
            app.update();
        }
        let root = app
            .world()
            .get::<Children>(owner)
            .expect("the plain scene is instantiated")
            .iter()
            .next()
            .unwrap();

        app.world_mut()
            .commands()
            .entity(owner)
            .trigger(RequestDetachVrm);
        for _ in 0..2 {
            app.update();
        }

        assert!(app.world().get::<WorldAssetRoot>(owner).is_some());
        assert!(app.world().get::<WorldInstance>(owner).is_some());
        assert!(app.world().get_entity(root).is_ok());
        success!()
    }

    /// A raw spawn that is re-targeted before its first asset loads must not leave
    /// the superseded queued instance behind as an orphan when that asset finally
    /// arrives.
    #[test]
    fn a_superseded_raw_instance_is_cleaned_up() -> TestResult {
        let mut app = instance_app();
        let first = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .reserve_handle();
        let second = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .reserve_handle();
        let owner = app.world_mut().spawn(WorldAssetRoot(first.clone())).id();
        for _ in 0..2 {
            app.update();
        }

        // Re-target before `first` loads.
        app.world_mut()
            .entity_mut(owner)
            .insert(WorldAssetRoot(second));
        for _ in 0..2 {
            app.update();
        }

        // `first` arrives afterwards: its queued spawn still runs, and its
        // crate-managed scene must be removed, not parented as an orphan.
        app.world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .insert(&first, vrm_scene(false).0)
            .unwrap();
        for _ in 0..6 {
            app.update();
        }

        assert!(
            app.world().get::<Children>(owner).is_none(),
            "the superseded instance must be cleaned up"
        );
        assert!(
            app.world().get::<WorldInstance>(owner).is_some(),
            "cleaning the superseded instance must keep the owner's newer request"
        );
        let orphan_roots = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, With<VrmSceneRoot>>();
            query.iter(app.world()).count()
        };
        assert_eq!(orphan_roots, 0, "no orphaned VRM scene root survives");
        success!()
    }

    /// Detaching a raw spawn by naming its *owner* (not the child root) must tear
    /// the ready instance down: the owner is not a "pending" spawn once its rig is
    /// attached, so it has to be classified as ready and stripped.
    #[test]
    fn a_raw_owner_detach_after_ready_removes_the_rig() -> TestResult {
        let mut app = instance_app();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(vrm_scene(false).0);
        let owner = app.world_mut().spawn(WorldAssetRoot(scene)).id();
        for _ in 0..6 {
            app.update();
        }

        let root = app
            .world()
            .get::<Children>(owner)
            .expect("the raw instance is parented under the owner")
            .iter()
            .next()
            .expect("the instance has a root");
        assert!(
            app.world().get::<Vrm>(root).is_some(),
            "the child is the VRM"
        );
        // A raw full detach also drops the private VRMA graph state on this root.
        app.world_mut()
            .entity_mut(root)
            .insert(VrmAnimationGraphState::default());

        app.world_mut()
            .commands()
            .entity(owner)
            .trigger(RequestDetachVrm);
        for _ in 0..3 {
            app.update();
        }

        assert!(app.world().get::<WorldAssetRoot>(owner).is_none());
        assert!(
            app.world().get::<WorldInstance>(owner).is_none(),
            "the owner must not keep naming the torn-down instance"
        );
        assert!(
            app.world().get::<VrmAnimationGraphState>(root).is_none(),
            "the raw root's graph state is dropped with the rig"
        );
        assert!(
            app.world().get::<Vrm>(root).is_none(),
            "the child rig is stripped"
        );
        let remaining = {
            let mut query = app.world_mut().query_filtered::<Entity, With<Vrm>>();
            query.iter(app.world()).count()
        };
        assert_eq!(
            remaining, 0,
            "no orphaned VRM survives the raw owner detach"
        );
        assert!(
            app.world().get_entity(root).is_err(),
            "the request named the owner, so the instance's scene root goes too"
        );
        assert!(
            app.world()
                .get::<Children>(owner)
                .is_none_or(|children| children.is_empty()),
            "no stripped instance root is left under the owner"
        );
        success!()
    }

    /// Re-arming an unchanged non-VRM handle must not re-insert `WorldAssetRoot`:
    /// the spawner reacts to `Changed<WorldAssetRoot>` and would respawn the
    /// instance on every re-arm, forever.
    #[test]
    fn a_non_vrm_handle_does_not_respawn_on_reresolve() -> TestResult {
        let mut app = instance_app();
        let plain_world = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(plain_scene());
        let plain = app
            .world_mut()
            .resource_mut::<Assets<Gltf>>()
            .add(gltf_with_scene(plain_world));
        let vrm = spawn_ready(&mut app, plain.clone());
        assert!(app.world().get::<Vrm>(vrm).is_none());
        let instance_before = **app
            .world()
            .get::<WorldInstance>(vrm)
            .expect("the non-VRM instance resolved");

        // Re-inserting the same handle re-arms the resolver via `Changed`.
        app.world_mut().entity_mut(vrm).insert(VrmHandle(plain));
        for _ in 0..3 {
            app.update();
        }

        assert_eq!(
            **app
                .world()
                .get::<WorldInstance>(vrm)
                .expect("still resolved"),
            instance_before,
            "an unchanged non-VRM scene must not respawn"
        );
        assert!(app.world().get::<Vrm>(vrm).is_none());
        success!()
    }

    /// A `WorldInstanceReady` for an owner that has already given up the avatar
    /// must not resurrect `Vrm`: the live instance is cleaned up instead. The
    /// scene root is asserted gone, so the test proves cleanup acted on the
    /// current instance rather than the event being ignored as invalid.
    #[test]
    fn a_ready_after_detach_does_not_resurrect_the_rig() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);
        assert!(app.world().get::<Vrm>(vrm).is_some());
        let root = scene_root_of(&app, vrm);
        let instance_id = **app.world().get::<WorldInstance>(vrm).unwrap();

        // The owner gives up the avatar, but a stale ready still arrives.
        app.world_mut().entity_mut(vrm).remove::<VrmHandle>();
        app.world_mut().commands().trigger(WorldInstanceReady {
            entity: vrm,
            instance_id,
        });
        for _ in 0..3 {
            app.update();
        }

        assert!(
            app.world().get_entity(root).is_err(),
            "the live instance is torn down, not ignored"
        );
        assert!(app.world().get::<Vrm>(vrm).is_none());
        assert!(app.world().get::<Initialized>(vrm).is_none());
        assert!(
            app.world().get::<WorldInstance>(vrm).is_none(),
            "the stale instance is unregistered from the owner"
        );
        assert!(app.world().get_entity(vrm).is_ok());
        success!()
    }

    /// `attach_managed`'s deferred guard: a rig whose owner has been detached
    /// since the ready event is not promoted; the stale instance is cleaned up.
    #[test]
    fn a_deferred_attach_does_not_promote_a_detached_owner() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);
        let root = scene_root_of(&app, vrm);
        let instance_id = **app.world().get::<WorldInstance>(vrm).unwrap();

        // The owner gives up the handle (and the attached markers) before the
        // deferred command would run.
        app.world_mut()
            .entity_mut(vrm)
            .remove::<(VrmHandle, Vrm, Initialized, VrmInstanceState)>();

        attach_managed(app.world_mut(), vrm, root, instance_id);

        assert!(
            app.world().get::<Vrm>(vrm).is_none(),
            "a detached owner is not promoted"
        );
        assert!(
            app.world().get_entity(root).is_err(),
            "the stale instance is cleaned up instead"
        );
        success!()
    }

    /// A `WorldAssetRoot` ancestor that does **not** own the rig must not be
    /// unregistered: only the true instance owner is ever touched.
    #[test]
    fn an_unrelated_world_asset_root_ancestor_is_not_touched() -> TestResult {
        let mut app = instance_app();
        // An unrelated scene owner with its own instance registration.
        let outer_scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(plain_scene());
        let owner = app.world_mut().spawn(WorldAssetRoot(outer_scene)).id();
        for _ in 0..3 {
            app.update();
        }
        // Its metadata is gone, so ownership cannot be verified from it.
        app.world_mut().entity_mut(owner).remove::<WorldInstance>();

        let rig = app.world_mut().spawn(Vrm).id();
        app.world_mut().entity_mut(owner).add_child(rig);

        app.world_mut()
            .commands()
            .entity(rig)
            .trigger(RequestDetachVrm);
        for _ in 0..2 {
            app.update();
        }

        assert!(
            app.world().get::<WorldAssetRoot>(owner).is_some(),
            "an unrelated `WorldAssetRoot` ancestor must keep its registration"
        );
        assert!(
            app.world().get::<Vrm>(rig).is_none(),
            "the rig is still stripped"
        );
        assert!(app.world().get_entity(rig).is_ok());
        success!()
    }

    /// A node the application reparented out of the hierarchy is still cleaned up
    /// on a raw detach, but only the instance's own entities are touched.
    #[test]
    fn a_reparented_instance_node_is_still_cleaned_up() -> TestResult {
        let mut app = instance_app();
        let scene = app
            .world_mut()
            .resource_mut::<Assets<WorldAsset>>()
            .add(vrm_scene(false).0);
        let owner = app.world_mut().spawn(WorldAssetRoot(scene)).id();
        for _ in 0..6 {
            app.update();
        }

        let root = app
            .world()
            .get::<Children>(owner)
            .unwrap()
            .iter()
            .next()
            .unwrap();
        let bone = app
            .world()
            .get::<Children>(root)
            .unwrap()
            .iter()
            .next()
            .unwrap();
        let leaf = app
            .world()
            .get::<Children>(bone)
            .unwrap()
            .iter()
            .next()
            .unwrap();

        // Reparent the bone away, taking its subtree with it.
        let holder = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(holder).add_child(bone);
        app.update();

        app.world_mut()
            .commands()
            .entity(owner)
            .trigger(RequestDetachVrm);
        for _ in 0..2 {
            app.update();
        }

        assert!(
            app.world().get_entity(bone).is_err(),
            "the reparented node is cleaned up"
        );
        assert!(
            app.world().get_entity(leaf).is_err(),
            "and its subtree follows"
        );
        assert!(
            app.world().get_entity(holder).is_ok(),
            "the new parent (not owned by the instance) survives"
        );
        success!()
    }

    /// A managed full detach despawns the whole hierarchy — including a node the
    /// application reparented out of it — and drops the derived graph state.
    #[test]
    fn a_managed_detach_clears_a_moved_node_and_the_graph_state() -> TestResult {
        let mut app = instance_app();
        let gltf = add_vrm(&mut app, vrm_scene(false).0);
        let vrm = spawn_ready(&mut app, gltf);
        let root = scene_root_of(&app, vrm);
        let bone = app
            .world()
            .get::<Children>(root)
            .unwrap()
            .iter()
            .next()
            .unwrap();
        let leaf = app
            .world()
            .get::<Children>(bone)
            .unwrap()
            .iter()
            .next()
            .unwrap();

        // The private graph state the runtime persists on the stable owner.
        app.world_mut()
            .entity_mut(vrm)
            .insert(VrmAnimationGraphState::default());
        // Reparent a node away, taking its subtree with it.
        let holder = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(holder).add_child(bone);
        app.update();

        app.world_mut()
            .commands()
            .entity(vrm)
            .trigger(RequestDetachVrm);
        for _ in 0..3 {
            app.update();
        }

        assert!(
            app.world().get_entity(root).is_err(),
            "the hierarchy is despawned"
        );
        assert!(
            app.world().get_entity(bone).is_err(),
            "the reparented node is despawned too"
        );
        assert!(
            app.world().get_entity(leaf).is_err(),
            "and its subtree follows"
        );
        assert!(
            app.world().get_entity(holder).is_ok(),
            "the new parent survives"
        );
        assert!(
            app.world().get::<VrmAnimationGraphState>(vrm).is_none(),
            "a full detach drops the derived graph state"
        );
        assert!(app.world().get_entity(vrm).is_ok());
        success!()
    }

    /// An invalidation must keep the private graph state so a reload can rebuild
    /// into it; only a full detach drops it.
    #[test]
    fn an_invalidation_keeps_the_graph_state() -> TestResult {
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
        app.world_mut()
            .entity_mut(vrm)
            .insert(VrmAnimationGraphState::default());

        app.world_mut()
            .entity_mut(vrm)
            .get_mut::<VrmHandle>()
            .unwrap()
            .0 = plain;
        for _ in 0..6 {
            app.update();
        }

        assert!(
            app.world().get::<Vrm>(vrm).is_none(),
            "the rig was invalidated"
        );
        assert!(
            app.world().get::<VrmAnimationGraphState>(vrm).is_some(),
            "an invalidation keeps the graph state for the reload"
        );
        success!()
    }
}
