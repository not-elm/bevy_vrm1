//! End-to-end tests for the managed VRM root identity driving the VRMA
//! reattachment pipeline.
//!
//! Unlike the unit tests next to each module, these wire the *real* plugins —
//! [`VrmInstancePlugin`], [`VrmDetachPlugin`], [`VrmaInitializePlugin`],
//! [`VrmaAnimationPlayersPlugin`] and bevy's [`AnimationPlugin`] — on top of the
//! real [`world_instance_spawner`] chain in `SpawnScene`, and spawn a real
//! `VrmHandle`. A hand-built [`WorldAsset`] stands in for the load-time glTF
//! pipeline (the asset loader is not exercised here), carrying the exact root
//! inventory, the crate-private [`VrmSceneRoot`] marker, and the
//! `AnimationTargetId` / `AnimatedBy` root and humanoid bone holders the pipeline
//! would bake, so attach, hot reload, invalidation and detach all run their
//! production code paths.
//!
//! The `.vrma` side is assembled the way `initialize::spawn_vrma` builds it: a
//! pristine [`VrmAnimationSourceClip`], a working [`VrmAnimationClipHandle`], a
//! [`HumanoidBoneRegistry`] and a source rig with rest transforms. The actual
//! file load is not required for the retarget.
//!
//! # What these pin down
//!
//! 1. A `.vrma` preserved across `A -> non-VRM -> B` is re-cloned from its
//!    pristine source and re-baked for `B`'s rest, and — because the graph asset
//!    is per-avatar and persistent — the spine is already evaluated at `B`'s
//!    baked pose in the very frame `B` attaches. `NeedsBake`/`VrmaRetargetPending`
//!    must not survive that frame.
//! 2. A hot reload carries the live playback (every field of `ActiveAnimation`,
//!    including the private progress counters, plus pause/seek/speed/repeat) and
//!    the root bone's `AnimationTransitions` onto the rebuilt player.
//! 3. A clip that left the player (a finished fade) is recorded as inactive and
//!    is not resurrected by a later reload.
//! 4. A duplicate `WorldInstanceReady` is inert and a managed detach removes the
//!    rig, the `.vrma` child and the private graph state while keeping the user's
//!    entity.

use crate::prelude::*;
use crate::success;
use crate::tests::{TestResult, test_app};
use crate::vrm::detach::{RequestDetachVrm, VrmDetachPlugin};
use crate::vrm::gltf::handler::bone_target_id;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use crate::vrm::instance::{VrmGeneration, VrmInstancePlugin, VrmInstanceState, VrmSceneRoot};
use crate::vrma::animation::VrmaAnimationPlayersPlugin;
use crate::vrma::animation::animation_graph::NeedsBake;
use crate::vrma::initialize::VrmaInitializePlugin;
use crate::vrma::{
    LoadedVrma, RetargetSource, VrmAnimationClipHandle, VrmAnimationGraphState,
    VrmAnimationNodeIndex, VrmAnimationSourceClip, Vrma, VrmaNeedsRestore, VrmaPlaybackState,
    VrmaRetargetPending, VrmaRetargetedFor,
};
use bevy::animation::{
    ActiveAnimation, AnimatedBy, AnimationPlugin, AnimationTargetId, RepeatAnimation,
    animated_field,
};
use bevy::app::{AnimationSystems, SceneSpawnerSystems, SpawnScene};
use bevy::asset::Assets;
use bevy::gltf::{Gltf, GltfNode};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use bevy::world_serialization::{
    DynamicWorld, WorldAsset, WorldAssetRoot, WorldInstance, WorldInstanceReady,
    WorldInstanceSpawner, world_instance_spawner, world_instance_spawner_system,
};
use std::time::Duration;

/// Every `LoadedVrma` a test sees, in order.
#[derive(Resource, Default)]
struct LoadedVrmas(Vec<Entity>);

/// One probed frame of the avatar under test.
#[derive(Debug, Clone, Copy)]
struct FramePose {
    /// The owner's `VrmGeneration`, or `None` while no rig is attached.
    generation: Option<u64>,
    needs_bake: bool,
    retarget_pending: bool,
    needs_restore: bool,
    /// The spine's local rotation as written by `animate_targets`.
    rotation: Quat,
}

/// Records one [`FramePose`] per frame for the avatar under test.
///
/// The probe runs in `PostUpdate` immediately after bevy's [`AnimationSystems`]
/// — i.e. after `animate_targets` has written the pose — and before transform
/// propagation. Recording every frame (rather than polling after the fact) is
/// what lets a test assert the *rebuild frame the clip was baked in*, which a
/// "wait for the first non-identity pose" loop can miss.
#[derive(Resource, Default)]
struct ProbeLog {
    owner: Option<Entity>,
    vrma: Option<Entity>,
    frames: Vec<FramePose>,
}

/// The `PostUpdate` probe. See [`ProbeLog`].
fn record_probe(
    mut log: ResMut<ProbeLog>,
    searcher: ChildSearcher,
    transforms: Query<&Transform>,
    generations: Query<&VrmGeneration>,
    needs_bake: Query<(), With<NeedsBake>>,
    retarget_pending: Query<(), With<VrmaRetargetPending>>,
    needs_restore: Query<(), With<VrmaNeedsRestore>>,
) {
    let Some(owner) = log.owner else {
        return;
    };
    let Some(root_bone) = searcher.find_root_bone(owner) else {
        return;
    };
    let rotation = searcher
        .find_by_bone_name(root_bone, &VrmBone::from("spine"))
        .and_then(|spine| transforms.get(spine).ok())
        .map_or(Quat::IDENTITY, |transform| transform.rotation);
    let (baking, retargeting, restoring) = match log.vrma {
        Some(vrma) => (
            needs_bake.contains(vrma),
            retarget_pending.contains(vrma),
            needs_restore.contains(vrma),
        ),
        None => (false, false, false),
    };
    log.frames.push(FramePose {
        generation: generations.get(owner).map(|generation| generation.0).ok(),
        needs_bake: baking,
        retarget_pending: retargeting,
        needs_restore: restoring,
        rotation,
    });
}

/// The header of an app that drives the whole managed chain.
///
/// The asset and spawner resources are initialized the way
/// `WorldSerializationPlugin` does — the plugin itself is deliberately not used,
/// so the tests keep control over the schedule and do not pull in the loaders —
/// and the spawner systems run in bevy's own
/// [`SceneSpawnerSystems::WorldInstanceSpawn`], which
/// [`VrmInstancePlugin`]'s resolver is ordered before.
fn e2e_app() -> App {
    let mut app = test_app();
    app.add_plugins((
        AnimationPlugin,
        VrmDetachPlugin,
        VrmInstancePlugin,
        VrmaInitializePlugin,
        VrmaAnimationPlayersPlugin,
    ))
    .init_asset::<WorldAsset>()
    .init_asset::<DynamicWorld>()
    .init_asset::<Gltf>()
    .init_asset::<VrmaAsset>()
    .init_asset::<GltfNode>()
    .init_resource::<WorldInstanceSpawner>()
    .init_resource::<LoadedVrmas>()
    .init_resource::<ProbeLog>()
    // The scene spawner drops any component whose reflected type is not
    // registered, and panics on one it cannot find, so the fixture inventory is
    // registered up front. (`VrmSceneRoot` and `VrmHandle` are registered by
    // `VrmInstancePlugin`.)
    .register_type::<Vrm>()
    .register_type::<Initialized>()
    .register_type::<VrmPath>()
    .register_type::<RestTransform>()
    .register_type::<RestGlobalTransform>()
    .register_type::<RestWorldTransform>()
    .register_type::<VrmBone>()
    .register_type::<LookAtProperties>()
    .register_type::<ConstraintExecutionOrder>()
    .register_type::<VrmExpressionWeights>()
    .register_type::<EffectiveExpressionWeights>()
    .register_type::<ExpressionMorphBinds>()
    .register_type::<ExpressionSettings>()
    .register_type::<AnimationTargetId>()
    .register_type::<AnimatedBy>()
    .register_type::<AnimationPlayer>()
    .register_type::<AnimationTransitions>()
    .register_type::<RetargetSource>()
    .register_type::<ChildOf>()
    .register_type::<Children>()
    .register_type::<Name>()
    .register_type::<Transform>()
    .register_type::<HeadBoneEntity>()
    .register_type::<HipsBoneEntity>()
    .add_systems(
        SpawnScene,
        (world_instance_spawner, world_instance_spawner_system)
            .chain()
            .in_set(SceneSpawnerSystems::WorldInstanceSpawn),
    )
    // After `animate_targets` wrote the pose this frame, before propagation.
    .add_systems(PostUpdate, record_probe.after(AnimationSystems))
    .add_observer(|event: On<LoadedVrma>, mut loads: ResMut<LoadedVrmas>| {
        loads.0.push(event.vrma);
    });
    app
}

/// A managed VRM scene asset plus the scratch-world entities a hot reload
/// mutates.
struct VrmFixture {
    /// The `Gltf` a `VrmHandle` points at.
    gltf: Handle<Gltf>,
    /// The `WorldAsset` itself, for hot-reload edits.
    asset: Handle<WorldAsset>,
    /// The scratch-world `spine` entity, whose rest pose a reload changes.
    spine: Entity,
    /// The rig's `RestWorldTransform`, reused to recompose the reloaded rest.
    prefix: GlobalTransform,
}

/// Builds the scene the load-time pipeline would bake for a rig whose spine
/// rests at `rest`, including the private [`VrmSceneRoot`] marker the attach
/// observer finds.
///
/// The root bone is a *different* entity from the scene root, so the player and
/// the graph live on the root bone exactly as `handler::scene::setup_animation`
/// places them, and every animation target names it through `AnimatedBy`.
fn vrm_scene(
    rest: Quat,
    prefix: GlobalTransform,
) -> (WorldAsset, Entity) {
    let mut world = World::new();
    let root_bone = world
        .spawn((
            Name::new(Vrm::ROOT_BONE),
            Transform::default(),
            AnimationPlayer::default(),
            AnimationTransitions::default(),
            RetargetSource,
        ))
        .id();
    let hips = world
        .spawn((
            VrmBone::from("hips"),
            bone_target_id("hips"),
            AnimatedBy(root_bone),
            RetargetSource,
            Transform::default(),
            RestTransform::default(),
            RestGlobalTransform(prefix),
            ChildOf(root_bone),
        ))
        .id();
    let head = world
        .spawn((
            VrmBone::from("head"),
            bone_target_id("head"),
            AnimatedBy(root_bone),
            RetargetSource,
            Transform::default(),
            RestTransform::default(),
            RestGlobalTransform(prefix),
            ChildOf(root_bone),
        ))
        .id();
    let spine_rest = Transform::from_rotation(rest);
    let spine = world
        .spawn((
            VrmBone::from("spine"),
            bone_target_id("spine"),
            AnimatedBy(root_bone),
            RetargetSource,
            Transform::default(),
            RestTransform(spine_rest),
            RestGlobalTransform(prefix.mul_transform(spine_rest)),
            ChildOf(root_bone),
        ))
        .id();
    let scene_root = world
        .spawn((
            VrmSceneRoot,
            VrmPath::new("vrm/Test.vrm"),
            RestWorldTransform(prefix),
            LookAtProperties::default(),
            ConstraintExecutionOrder(Vec::new()),
            VrmExpressionWeights::default(),
            EffectiveExpressionWeights::default(),
            ExpressionMorphBinds::default(),
            ExpressionSettings::default(),
            vrm_root_animation_target(),
            AnimatedBy(root_bone),
            HeadBoneEntity(head),
            HipsBoneEntity(hips),
        ))
        .id();
    world.entity_mut(root_bone).insert(ChildOf(scene_root));
    (WorldAsset::new(world), spine)
}

/// A `Gltf` exposing exactly one scene.
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
        animations: Vec::new(),
        named_animations: Default::default(),
        source: None,
    }
}

/// A scene that is not a VRM at all: no [`VrmSceneRoot`] anywhere.
fn plain_scene() -> WorldAsset {
    let mut world = World::new();
    let root = world.spawn_empty().id();
    world.spawn(ChildOf(root));
    WorldAsset::new(world)
}

/// Registers a VRM scene and the `Gltf` that points at it.
fn add_vrm(
    app: &mut App,
    rest: Quat,
    prefix: GlobalTransform,
) -> VrmFixture {
    let (scene, spine) = vrm_scene(rest, prefix);
    let asset = app
        .world_mut()
        .resource_mut::<Assets<WorldAsset>>()
        .add(scene);
    let gltf = app
        .world_mut()
        .resource_mut::<Assets<Gltf>>()
        .add(gltf_with_scene(asset.clone()));
    VrmFixture {
        gltf,
        asset,
        spine,
        prefix,
    }
}

/// Registers a non-VRM `Gltf` for the invalidation step.
fn add_plain(app: &mut App) -> Handle<Gltf> {
    let scene = app
        .world_mut()
        .resource_mut::<Assets<WorldAsset>>()
        .add(plain_scene());
    app.world_mut()
        .resource_mut::<Assets<Gltf>>()
        .add(gltf_with_scene(scene))
}

/// A `.vrma` scene assembled the way `initialize::spawn_vrma` builds one.
struct VrmaFixture {
    entity: Entity,
    source: Handle<AnimationClip>,
    working: Handle<AnimationClip>,
}

/// Spawns a `.vrma` on `owner` whose single `spine` curve holds `angle` around
/// X. `source_prefix` is the source rig's own placement snapshot, so the
/// retarget has a non-trivial prefix to strip on the source side too.
fn add_vrma(
    app: &mut App,
    owner: Entity,
    source_prefix: GlobalTransform,
    angle: f32,
    playback: VrmaPlaybackState,
) -> VrmaFixture {
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
    let source = app
        .world_mut()
        .resource_mut::<Assets<AnimationClip>>()
        .add(clip.clone());
    let working = app
        .world_mut()
        .resource_mut::<Assets<AnimationClip>>()
        .add(clip);
    let registry =
        HumanoidBoneRegistry::from_pairs([(VrmBone::from("spine"), Name::new("source_spine"))]);
    let entity = app
        .world_mut()
        .spawn((
            Vrma,
            Initialized,
            registry,
            RestWorldTransform(source_prefix),
            VrmAnimationSourceClip(source.clone()),
            VrmAnimationClipHandle(working.clone()),
            playback,
            ChildOf(owner),
        ))
        .id();
    app.world_mut().spawn((
        Name::new("source_spine"),
        source_target,
        RestTransform::default(),
        RestGlobalTransform(source_prefix),
        ChildOf(entity),
    ));
    VrmaFixture {
        entity,
        source,
        working,
    }
}

/// A playback state a reattach will restore: active, carrying a live snapshot.
fn active_playback() -> VrmaPlaybackState {
    VrmaPlaybackState {
        active: true,
        animation: Some(ActiveAnimation::default()),
        ..default()
    }
}

/// Spawns the entity the application owns.
fn spawn_owner(
    app: &mut App,
    gltf: Handle<Gltf>,
) -> Entity {
    app.world_mut()
        .spawn((
            VrmHandle(gltf),
            Name::new("avatar"),
            Transform::from_xyz(0.0, 0.0, 1.0),
        ))
        .id()
}

/// Advances until the managed owner has attached, or panics.
fn wait_ready(
    app: &mut App,
    owner: Entity,
) {
    for _ in 0..16 {
        if app.world().get::<Initialized>(owner).is_some() {
            return;
        }
        app.update();
    }
    panic!("the managed avatar never became ready");
}

/// Runs `count` frames with no assertions.
fn run_frames(
    app: &mut App,
    count: usize,
) {
    for _ in 0..count {
        app.update();
    }
}

/// The internal scene root the owner currently records.
fn scene_root_of(
    app: &App,
    owner: Entity,
) -> Entity {
    app.world()
        .get::<VrmInstanceState>(owner)
        .expect("the owner records a live rig")
        .scene_root
}

/// The attach generation the owner currently records.
fn generation(
    app: &App,
    owner: Entity,
) -> Option<u64> {
    app.world().get::<VrmGeneration>(owner).map(|g| g.0)
}

/// The graph node the `.vrma` currently owns.
fn node_index(
    app: &App,
    vrma: Entity,
) -> AnimationNodeIndex {
    app.world()
        .get::<VrmAnimationNodeIndex>(vrma)
        .expect("the graph assigned this .vrma a node")
        .0
}

/// The working clip handle the `.vrma` currently points at.
fn working_handle(
    app: &App,
    vrma: Entity,
) -> Handle<AnimationClip> {
    app.world()
        .get::<VrmAnimationClipHandle>(vrma)
        .expect("the vrma carries a working clip")
        .0
        .clone()
}

/// The recorded playback of a `.vrma`.
fn playback(
    app: &App,
    vrma: Entity,
) -> VrmaPlaybackState {
    app.world()
        .get::<VrmaPlaybackState>(vrma)
        .expect("the vrma snapshots its playback")
        .clone()
}

/// The per-avatar graph state.
fn graph_state(
    app: &App,
    owner: Entity,
) -> VrmAnimationGraphState {
    app.world()
        .get::<VrmAnimationGraphState>(owner)
        .expect("the graph build stored the per-avatar state")
        .clone()
}

/// Depth-first search for the first descendant named `name`.
fn find_descendant_named(
    world: &World,
    root: Entity,
    name: &str,
) -> Option<Entity> {
    if world.get::<Name>(root).is_some_and(|n| n.as_str() == name) {
        return Some(root);
    }
    let children: Vec<Entity> = world
        .get::<Children>(root)
        .map(|children| children.to_vec())
        .unwrap_or_default();
    for child in children {
        if let Some(found) = find_descendant_named(world, child, name) {
            return Some(found);
        }
    }
    None
}

/// Depth-first search for the first descendant carrying `bone`.
fn find_descendant_bone(
    world: &World,
    root: Entity,
    bone: &str,
) -> Option<Entity> {
    if world.get::<VrmBone>(root).is_some_and(|b| b.0 == bone) {
        return Some(root);
    }
    let children: Vec<Entity> = world
        .get::<Children>(root)
        .map(|children| children.to_vec())
        .unwrap_or_default();
    for child in children {
        if let Some(found) = find_descendant_bone(world, child, bone) {
            return Some(found);
        }
    }
    None
}

/// The root bone of `owner`'s live rig.
fn root_bone_of(
    app: &App,
    owner: Entity,
) -> Entity {
    find_descendant_named(app.world(), owner, Vrm::ROOT_BONE).expect("the rig has a root bone")
}

/// The spine's evaluated rotation, i.e. what the animation pass actually wrote.
fn evaluated_spine_rotation(
    app: &App,
    owner: Entity,
) -> Quat {
    let root_bone = root_bone_of(app, owner);
    let spine = find_descendant_bone(app.world(), root_bone, "spine").expect("the spine bone");
    app.world()
        .get::<Transform>(spine)
        .expect("the spine has a Transform")
        .rotation
}

/// Asserts the spine is driven to `angle` for the rig whose spine rests at
/// `rest`.
///
/// The source and destination rest snapshots have their application placement
/// stripped by the retarget, so the expected destination pose is simply
/// `curve . rest`, independently of where either model was placed.
fn assert_spine_pose(
    app: &App,
    owner: Entity,
    angle: f32,
    rest: Quat,
    label: &str,
) {
    let actual = evaluated_spine_rotation(app, owner);
    let expected = Quat::from_rotation_x(angle) * rest;
    assert!(
        actual.angle_between(expected) < 0.01,
        "{label}: the spine must be evaluated at {expected:?}, got {actual:?}"
    );
}

/// Runs frames until the first-time rig is evaluated, then asserts the pose.
///
/// A brand-new graph asset is only threaded by bevy a frame after it is added,
/// so the first attach may need an extra frame before the pose leaves identity.
/// A reattachment reuses the persistent graph and is asserted directly instead.
fn wait_first_evaluation(
    app: &mut App,
    owner: Entity,
    angle: f32,
    rest: Quat,
    label: &str,
) {
    for _ in 0..8 {
        app.update();
        let rotation = evaluated_spine_rotation(app, owner);
        if rotation.angle_between(Quat::IDENTITY) > 0.01 {
            let expected = Quat::from_rotation_x(angle) * rest;
            assert!(
                rotation.angle_between(expected) < 0.01,
                "{label}: the first evaluated frame must be the baked pose, got {rotation:?}"
            );
            return;
        }
    }
    panic!("{label}: the rebuilt rig was never evaluated");
}

/// Replaces the in-place `VrmHandle`, caught by `Changed` detection.
fn set_handle(
    app: &mut App,
    owner: Entity,
    gltf: Handle<Gltf>,
) {
    app.world_mut()
        .entity_mut(owner)
        .get_mut::<VrmHandle>()
        .expect("the owner still carries its handle")
        .0 = gltf;
}

/// Rewrites a scene asset's spine rest pose in place, which emits the
/// `Modified` event the spawner reloads a live instance from.
fn set_asset_spine_rest(
    app: &mut App,
    asset: &Handle<WorldAsset>,
    spine: Entity,
    rest: Quat,
    prefix: GlobalTransform,
) {
    let mut assets = app.world_mut().resource_mut::<Assets<WorldAsset>>();
    let mut world_asset = assets.get_mut(asset).expect("the scene asset exists");
    let local = Transform::from_rotation(rest);
    world_asset.world.entity_mut(spine).insert((
        RestTransform(local),
        RestGlobalTransform(prefix.mul_transform(local)),
    ));
}

/// Plays a `.vrma` through the production event, with a controllable fade.
fn play_vrma(
    app: &mut App,
    vrma: Entity,
    transition_duration: Duration,
) {
    app.world_mut()
        .commands()
        .entity(vrma)
        .trigger(move |entity| PlayVrma {
            vrma: entity,
            repeat: RepeatAnimation::Never,
            transition_duration,
            reset_spring_bones: false,
        });
}

/// The probe frame recorded for the update that just ran.
fn last_probed_frame(app: &App) -> FramePose {
    *app.world()
        .resource::<ProbeLog>()
        .frames
        .last()
        .expect("the probe recorded a frame")
}

/// Points the probe at one avatar's `.vrma`.
fn watch(
    app: &mut App,
    owner: Entity,
    vrma: Entity,
) {
    let mut log = app.world_mut().resource_mut::<ProbeLog>();
    log.owner = Some(owner);
    log.vrma = Some(vrma);
    log.frames.clear();
}

/// A `.vrma` preserved across `A -> non-VRM -> B` must re-bake for `B` and be
/// evaluated at `B`'s pose in the frame `B` attaches, with nothing left pending.
///
/// A generation reset after non-VRM invalidation would let `B` reuse `A`'s
/// baked clip (wrong pose). Baking in `Update` would also miss the attach frame's
/// evaluation, because the attachment runs later in `SpawnScene`.
#[test]
fn a_preserved_vrma_rebakes_before_the_attach_frame_is_evaluated() -> TestResult {
    let mut app = e2e_app();
    let prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(0.3)));
    let source_prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(1.8)));
    let rest_a = Quat::from_rotation_z(0.4);
    let rest_b = Quat::from_rotation_z(-0.6);
    let angle = 0.5;

    let a = add_vrm(&mut app, rest_a, prefix);
    let b = add_vrm(&mut app, rest_b, prefix);
    let plain = add_plain(&mut app);

    let owner = spawn_owner(&mut app, a.gltf.clone());
    let vrma = add_vrma(&mut app, owner, source_prefix, angle, active_playback());
    // A second, independent avatar sharing model A: it must survive the swap.
    let bystander = spawn_owner(&mut app, a.gltf.clone());
    let bystander_vrma = add_vrma(&mut app, bystander, source_prefix, 0.9, active_playback());

    watch(&mut app, owner, vrma.entity);

    wait_ready(&mut app, owner);
    wait_ready(&mut app, bystander);
    assert_eq!(generation(&app, owner), Some(1));
    let first_root = scene_root_of(&app, owner);
    let first_working = working_handle(&app, vrma.entity);
    let bystander_working = working_handle(&app, bystander_vrma.entity);

    // A first-time graph threads one frame late; the pose must still be A's.
    wait_first_evaluation(&mut app, owner, angle, rest_a, "A");
    wait_first_evaluation(
        &mut app,
        bystander,
        0.9,
        rest_a,
        "bystander before the swap",
    );

    // A -> a plain, non-VRM scene: the rig is invalidated but the generation and
    // the `.vrma` entity survive.
    set_handle(&mut app, owner, plain);
    run_frames(&mut app, 6);
    assert!(app.world().get::<Vrm>(owner).is_none());
    assert!(app.world().get::<Initialized>(owner).is_none());
    assert_eq!(
        generation(&app, owner),
        Some(1),
        "the generation is kept across the invalidation"
    );
    assert!(
        app.world().get_entity(vrma.entity).is_ok(),
        "the vrma persists"
    );

    // plain -> B, a different rest pose. The production `Insert<Initialized>`
    // observer drives the graph rebuild; nothing is triggered by hand.
    watch(&mut app, owner, vrma.entity);
    set_handle(&mut app, owner, b.gltf.clone());
    let mut attach_frame = None;
    for _ in 0..14 {
        app.update();
        if generation(&app, owner) == Some(2) {
            attach_frame = Some(last_probed_frame(&app));
            break;
        }
    }
    let frame = attach_frame.expect("model B must attach");
    assert_eq!(frame.generation, Some(2));
    assert!(
        !frame.needs_bake,
        "the rebuild must bake before the attach frame's evaluation ends"
    );
    assert!(
        !frame.retarget_pending,
        "the retarget must have completed in the attach frame"
    );
    assert_spine_pose(&app, owner, angle, rest_b, "B in its attach frame");

    let second_root = scene_root_of(&app, owner);
    assert_ne!(
        first_root, second_root,
        "a genuinely new internal scene root"
    );
    assert!(
        app.world().get_entity(first_root).is_err(),
        "the old instance is despawned"
    );
    assert_eq!(
        app.world()
            .get::<VrmaRetargetedFor>(vrma.entity)
            .map(|r| r.0),
        Some(2),
        "the preserved vrma records the new generation"
    );
    assert_ne!(
        first_working,
        working_handle(&app, vrma.entity),
        "a new generation re-clones the working clip from the pristine source"
    );
    assert_spine_pose(&app, owner, angle, rest_b, "post-B");

    // The bystander is a separate instance of model A: neither its baked clip
    // nor its evaluated pose may move.
    assert_eq!(
        bystander_working,
        working_handle(&app, bystander_vrma.entity),
        "the other avatar's working clip is untouched"
    );
    assert_spine_pose(&app, bystander, 0.9, rest_a, "bystander after the swap");
    success!()
}

/// A hot reload carries the live playback and the root bone's transitions onto
/// the rebuilt player, field for field.
///
/// Saving only the public playback settings would reset the private
/// `ActiveAnimation` progress (`completions`, `elapsed`, `just_completed`,
/// `last_seek_time`). The transitions must be snapshotted alongside the animation.
#[test]
fn a_hot_reload_restores_the_live_animation_and_transitions_exactly() -> TestResult {
    let mut app = e2e_app();
    let prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(0.3)));
    let source_prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(1.8)));
    let rest_a = Quat::from_rotation_z(0.4);
    let rest_b = Quat::from_rotation_z(-0.6);
    let angle = 0.5;

    let scene = add_vrm(&mut app, rest_a, prefix);
    let owner = spawn_owner(&mut app, scene.gltf.clone());
    let vrma = add_vrma(
        &mut app,
        owner,
        source_prefix,
        angle,
        VrmaPlaybackState::default(),
    );
    watch(&mut app, owner, vrma.entity);
    wait_ready(&mut app, owner);
    run_frames(&mut app, 3);

    // Start the clip through the production event, with a real fade.
    play_vrma(&mut app, vrma.entity, Duration::from_millis(100));
    run_frames(&mut app, 6);

    let node = node_index(&app, vrma.entity);
    let root_bone = root_bone_of(&app, owner);
    let first_root = scene_root_of(&app, owner);
    // Compose a live state with progress no setter can reach: a completion, a
    // seek offset, a speed and a repeat mode.
    {
        let mut entity = app.world_mut().entity_mut(root_bone);
        let mut player = entity
            .get_mut::<AnimationPlayer>()
            .expect("the root bone holds the player");
        player
            .animation_mut(node)
            .expect("the clip is playing")
            .set_seek_time(0.05)
            .set_speed(2.0)
            .set_repeat(RepeatAnimation::Count(3));
    }
    // `Time<Virtual>` clamps a frame delta to 0.25 s (its default max), so the
    // completion is accumulated across frames rather than in one large step.
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        0.25,
    )));
    run_frames(&mut app, 3);
    let completions = app
        .world()
        .get::<AnimationPlayer>(root_bone)
        .unwrap()
        .animation(node)
        .unwrap()
        .completions();
    assert!(
        completions >= 1,
        "the live clip must accumulate a completion before the reload"
    );
    // Freeze a rich, deterministic state so the snapshot is stable across the
    // reload-detection frames.
    app.world_mut()
        .entity_mut(root_bone)
        .get_mut::<AnimationPlayer>()
        .unwrap()
        .animation_mut(node)
        .unwrap()
        .set_seek_time(0.25)
        .pause();
    app.update();

    let before = playback(&app, vrma.entity);
    let expected = before
        .animation
        .expect("the snapshot keeps the live animation");
    assert!(expected.is_paused(), "the snapshot records the pause");
    assert_eq!(
        graph_state(&app, owner).transitions.get_main_animation(),
        Some(node),
        "the snapshot keeps the live transitions"
    );

    // Hot reload the same asset with a new rest pose.
    set_asset_spine_rest(&mut app, &scene.asset, scene.spine, rest_b, prefix);

    let mut reloaded_frame = None;
    for _ in 0..12 {
        app.update();
        if generation(&app, owner) == Some(2) {
            reloaded_frame = Some(last_probed_frame(&app));
            break;
        }
    }
    let frame = reloaded_frame.expect("the modified asset must reload its live instance");
    assert!(
        !frame.needs_bake,
        "the reload bakes before the frame is evaluated"
    );
    assert!(
        !frame.retarget_pending,
        "the retarget completed in the reload frame"
    );
    assert_spine_pose(&app, owner, angle, rest_b, "the reload frame");

    assert_ne!(
        scene_root_of(&app, owner),
        first_root,
        "the reload attaches a new internal scene root"
    );
    let second_root_bone = root_bone_of(&app, owner);
    assert_ne!(
        second_root_bone, root_bone,
        "the reload attaches a new root bone and player"
    );
    let restored = app
        .world()
        .get::<AnimationPlayer>(second_root_bone)
        .expect("the new root bone holds the player")
        .animation(node)
        .expect("the restored clip is playing on the new player");
    assert_eq!(
        format!("{restored:?}"),
        format!("{expected:?}"),
        "every field of the live animation, including private progress counters, survives"
    );
    assert!(restored.is_paused(), "the pause survives");
    assert_eq!(
        app.world()
            .get::<AnimationTransitions>(second_root_bone)
            .expect("the new root bone holds transitions")
            .get_main_animation(),
        Some(node),
        "the transitions are carried to the new root bone"
    );
    success!()
}

/// A `.vrma` that left the player (a completed fade) is recorded as inactive and
/// is not resurrected by a later reload.
#[test]
fn a_faded_out_clip_is_not_resurrected_by_a_reload() -> TestResult {
    let mut app = e2e_app();
    let prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(0.3)));
    let source_prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(1.8)));
    let rest_a = Quat::from_rotation_z(0.4);
    let rest_b = Quat::from_rotation_z(-0.6);

    let scene = add_vrm(&mut app, rest_a, prefix);
    let owner = spawn_owner(&mut app, scene.gltf.clone());
    let fading = add_vrma(
        &mut app,
        owner,
        source_prefix,
        0.5,
        VrmaPlaybackState::default(),
    );
    let staying = add_vrma(
        &mut app,
        owner,
        source_prefix,
        0.9,
        VrmaPlaybackState::default(),
    );
    wait_ready(&mut app, owner);
    run_frames(&mut app, 3);

    let fading_node = node_index(&app, fading.entity);
    let staying_node = node_index(&app, staying.entity);

    // Play `fading`, then cross-fade to `staying`: the fade-out removes
    // `fading` from the player entirely.
    play_vrma(&mut app, fading.entity, Duration::ZERO);
    run_frames(&mut app, 3);
    play_vrma(&mut app, staying.entity, Duration::from_millis(300));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        1.0,
    )));
    run_frames(&mut app, 3);

    let root_bone = root_bone_of(&app, owner);
    assert!(
        app.world()
            .get::<AnimationPlayer>(root_bone)
            .unwrap()
            .animation(fading_node)
            .is_none(),
        "the faded-out clip left the player"
    );
    let faded = playback(&app, fading.entity);
    assert!(!faded.active, "a clip that left the player is inactive");
    assert!(faded.animation.is_none(), "and carries no live animation");

    // Reload: the stopped clip must stay stopped.
    set_asset_spine_rest(&mut app, &scene.asset, scene.spine, rest_b, prefix);
    let mut reloaded = false;
    for _ in 0..12 {
        app.update();
        if generation(&app, owner) == Some(2) {
            reloaded = true;
            break;
        }
    }
    assert!(reloaded, "the modified asset must reload its live instance");

    let second_root_bone = root_bone_of(&app, owner);
    let player = app
        .world()
        .get::<AnimationPlayer>(second_root_bone)
        .expect("the new root bone holds the player");
    assert!(
        player.animation(fading_node).is_none(),
        "a faded-out clip must not be resurrected by a reload"
    );
    assert!(
        player.animation(staying_node).is_some(),
        "the clip that was still playing is restored"
    );
    success!()
}

/// A duplicate `WorldInstanceReady` is inert, and a managed detach removes the
/// rig, the `.vrma` child and the private graph state while keeping the user's
/// entity.
#[test]
fn a_duplicate_ready_is_inert_and_a_managed_detach_removes_the_vrma() -> TestResult {
    let mut app = e2e_app();
    let prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(0.3)));
    let source_prefix = GlobalTransform::from(Transform::from_rotation(Quat::from_rotation_y(1.8)));
    let rest = Quat::from_rotation_z(0.4);

    let scene = add_vrm(&mut app, rest, prefix);
    let owner = spawn_owner(&mut app, scene.gltf.clone());
    let vrma = add_vrma(&mut app, owner, source_prefix, 0.5, active_playback());
    wait_ready(&mut app, owner);
    run_frames(&mut app, 3);
    assert_eq!(
        app.world().resource::<LoadedVrmas>().0,
        vec![vrma.entity],
        "one bake, one LoadedVrma per vrma"
    );
    let working = working_handle(&app, vrma.entity);

    // A duplicate ready must not attach a second time, re-bake or re-clone.
    let instance_id = **app
        .world()
        .get::<WorldInstance>(owner)
        .expect("the owner names its instance");
    app.world_mut().commands().trigger(WorldInstanceReady {
        entity: owner,
        instance_id,
    });
    run_frames(&mut app, 3);
    assert_eq!(
        app.world().resource::<LoadedVrmas>().0,
        vec![vrma.entity],
        "a duplicate ready must not re-bake"
    );
    assert_eq!(
        working,
        working_handle(&app, vrma.entity),
        "a duplicate ready must not re-clone the working clip"
    );
    assert_eq!(generation(&app, owner), Some(1));

    // Detach the managed avatar: the rig and every child go, the entity stays.
    let rig_root = scene_root_of(&app, owner);
    app.world_mut()
        .commands()
        .entity(owner)
        .trigger(RequestDetachVrm);
    run_frames(&mut app, 4);

    assert!(
        app.world().get_entity(owner).is_ok(),
        "the user's root survives"
    );
    assert!(app.world().get::<VrmHandle>(owner).is_none());
    assert!(app.world().get::<Vrm>(owner).is_none());
    assert!(app.world().get::<Initialized>(owner).is_none());
    assert!(
        app.world().get::<VrmAnimationGraphState>(owner).is_none(),
        "the per-avatar graph state is derived data and goes with the detach"
    );
    assert!(
        app.world().get_entity(vrma.entity).is_err(),
        "the vrma child is despawned"
    );
    assert!(
        app.world().get_entity(rig_root).is_err(),
        "the internal scene root is despawned"
    );
    let vrm_markers = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Vrm>>();
        query.iter(app.world()).count()
    };
    assert_eq!(vrm_markers, 0, "no entity keeps a false Vrm marker");
    let vrma_markers = {
        let mut query = app.world_mut().query_filtered::<Entity, With<Vrma>>();
        query.iter(app.world()).count()
    };
    assert_eq!(vrma_markers, 0, "the vrma source rig is gone");
    success!()
}
