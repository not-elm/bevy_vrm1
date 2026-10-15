//! You can transition to the corresponding animation by pressing the number keys 1 to 4.
//!
//! We use [`AnimationTransitions`] internally for bone animations to achieve smooth transitions,
//! but there is an issue where interpolation fails if the initial poses of the source and target VRMAs differ.
//! If anyone has a good solution, please feel free to open an issue or submit a PR.
//!
//! 1. `VRMA_01.vrma`
//! 2. `VRMA_02.vrma`
//! 3. `VRMA_03.vrma`
//! 4. `different_pose.vrma`
//!
//! The avatar is spawned with [`VrmHandle`] on the entity that *is* the VRM
//! root, and all four `.vrma` handles are spawned as its children in the same
//! call. Every VRMA lands as a child of the avatar root, which receives
//! [`Initialized`] once the scene is bound - the marker `VrmaPlugin` waits for
//! before it resolves a handle (`src/vrma/initialize.rs:38-44`). No entity id is
//! assumed anywhere.
//!
//! The four asset loads start here, in `Startup`, in parallel with the `.vrm`.
//! The `.vrma` may therefore finish loading before the avatar's scene is bound,
//! or long after - each handle is kept alive by its `VrmaHandle` component, so
//! neither order is lost. This example also relies on the graph build waiting
//! for *all* VRMA children before it assigns node indices
//! (`src/vrma/initialize.rs:116-142`), which is what keeps the four transitions
//! below addressing the same graph.
//!
//! # Both kinds of track transition
//!
//! The bone tracks this transition is about do play on an avatar bound from a
//! load-time scene: the retarget resolves the destination bone by [`VrmBone`], which the
//! pipeline writes on every bone (`src/vrm/gltf/handler/nodes.rs:39`), and the
//! graph, its mask groups and the `AnimationPlayer` on `Vrm::ROOT_BONE` are all
//! built from components a pipeline scene does carry
//! (`src/vrm/gltf/handler/scene.rs:548-593`).
//!
//! The *expression* tracks transition with them. `VRMA_01.vrma` drives `happy`,
//! `aa` and `blinkRight`, `VRMA_02.vrma` drives `happy` and `blink`, and
//! `VRMA_03.vrma` drives `happy` - each encoded, per the specification, as a
//! `translation` curve whose x component is the weight, which nothing can
//! consume as a morph. `retarget_expression_curves`
//! (`src/vrma/animation/expressions.rs:154`) rewrites each into a single
//! [`ExpressionWeightProperty`] curve addressed by
//! [`vrm_root_animation_target`], the synthetic target `setup_animation` puts
//! on the avatar root (`src/vrm/gltf/handler/scene.rs:581`), and the weights
//! reach the meshes through [`apply_expression_morph_binds`]. Both halves land
//! in the *same* `AnimationClip` asset
//! (`src/vrma/animation/animation_graph.rs:163-246`), so the one
//! [`PlayVrma`] per key blends the face and the body with a single
//! `AnimationTransitions` weight and the two cannot drift apart.
//!
//! Two consequences worth knowing:
//!
//! * `different_pose.vrma` (key `4`) declares **no** `expressions` at all - only a
//!   `humanoid` block - so it contributes no expression curves and never drives
//!   the face to neutral. Its `PlayVrma` zeroes nothing
//!   (`reset_expression_weights` clears the expressions the clip *declares*,
//!   `play.rs:101`), and this example never triggers [`StopVrma`], which is what
//!   would clear the outgoing clip's own expressions (`play.rs:153-181`). Press
//!   `4` while a face is playing and that face keeps the last weight the
//!   previous clip wrote.
//! * the caveat at the top of this file is about bones only, and this example
//!   does not resolve it. The retarget normalizes each `.vrma` against the
//!   destination's rest pose
//!   (`src/vrma/animation/bone_rotation.rs:88-97`), so all four clips evaluate
//!   the avatar's own rest pose at `t = 0`; what `AnimationTransitions` then
//!   interpolates is those already-composed world rotations, and blending two
//!   quaternions is not the same as blending the local deltas they were built
//!   from.
//!
//! # Known gap: eye gaze
//!
//! `VRMC_vrm_animation.lookAt` is not read - the schema the `.vrma` loader
//! deserializes (`src/vrma/gltf/extensions.rs:33-39`) has no such field - so a
//! file's gaze direction and `offsetFromHeadBone` are dropped. Per the
//! specification `LookAt` is also the only permitted source of gaze, since
//! `leftEye` / `rightEye` may not carry humanoid animation data and `lookUp` /
//! `lookDown` / `lookLeft` / `lookRight` may not carry expression data. None of
//! these four files declares `lookAt`, so nothing here is affected.

use bevy::animation::RepeatAnimation;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::time::Duration;

/// Tags the VRMA this example plays for the number key `I`.
#[derive(Component, Default)]
struct VrmaNo<const I: usize>;

fn main() {
    App::new()
        // `VrmGltfPlugin` claims the `vrm` extension for `Handle<Gltf>`, which is
        // what `VrmHandle` wraps, so it has to be in the app. `VrmaPlugin` still
        // owns the `.vrma` loader.
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin, VrmaPlugin))
        .add_systems(
            Startup,
            (spawn_directional_light, spawn_camera, spawn_avatar),
        )
        .add_systems(
            Update,
            (
                play_vrma::<1>.run_if(input_just_pressed(KeyCode::Digit1)),
                play_vrma::<2>.run_if(input_just_pressed(KeyCode::Digit2)),
                play_vrma::<3>.run_if(input_just_pressed(KeyCode::Digit3)),
                play_vrma::<4>.run_if(input_just_pressed(KeyCode::Digit4)),
            ),
        )
        .run();
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        // A default light does not shadow the exclusive first-person layers.
        all_vrm_render_layers(),
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers,
        // and `firstPerson: auto` put this avatar's head copies on one of them.
        third_person_camera_layers(),
        Transform::from_xyz(0., 0.8, 2.5),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // The four loads start here, in parallel with the `.vrm`. Each handle is
    // kept alive by its `VrmaHandle` component until it is needed.
    let vrma_01 = asset_server.load("vrma/VRMA_01.vrma");
    let vrma_02 = asset_server.load("vrma/VRMA_02.vrma");
    let vrma_03 = asset_server.load("vrma/VRMA_03.vrma");
    let different_pose = asset_server.load("vrma/different_pose.vrma");
    commands
        .spawn((
            VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
            Transform::default(),
        ))
        .with_children(|avatar| {
            // Each VRMA is a child of the VRM it retargets to. `VrmaNo` survives
            // untouched: `spawn_vrma` on the VRMA side removes `VrmaHandle`
            // and never touches the rest of the bundle, so the keys below still
            // find these entities after the `.vrma` has loaded.
            avatar.spawn((VrmaNo::<1>, VrmaHandle(vrma_01)));
            avatar.spawn((VrmaNo::<2>, VrmaHandle(vrma_02)));
            avatar.spawn((VrmaNo::<3>, VrmaHandle(vrma_03)));
            avatar.spawn((VrmaNo::<4>, VrmaHandle(different_pose)));
        });
}

/// Transitions the avatar onto the VRMA tagged `I`.
///
/// A key pressed before the graph exists does nothing: the graph build assigns
/// the `AnimationNodeIndex` this trigger needs, and `PlayVrma` ignores a VRMA that
/// has none. That is the readiness rule, not a race - once every `.vrma` has
/// loaded, the four transitions all address the one graph.
fn play_vrma<const I: usize>(
    mut commands: Commands,
    vrmas: Query<Entity, With<VrmaNo<I>>>,
) {
    let Ok(vrma_entity) = vrmas.single() else {
        return;
    };
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::from_millis(300),
        vrma: vrma_entity,
        reset_spring_bones: false,
    });
}
