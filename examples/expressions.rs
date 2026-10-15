//! This example shows how to directly control VRM expressions from code.
//!
//! Press number keys to trigger expressions:
//! - 1: happy (`SetExpressions` — replaces all)
//! - 2: angry (`SetExpressions` — replaces all)
//! - 3: sad (`SetExpressions` — replaces all)
//! - 4: blink (`SetExpressions` — replaces all)
//! - 5: aa lip-sync (`ModifyExpressions::mouth` — resets other vowels)
//! - 6: ih lip-sync (`ModifyExpressions::mouth` — resets other vowels)
//! - 7: ou lip-sync (`ModifyExpressions::mouth` — resets other vowels)
//! - 8: ee lip-sync (`ModifyExpressions::mouth` — resets other vowels)
//! - 0: clear all expressions (return to VRMA control)
//!
//! The avatar is spawned with [`VrmHandle`] on the entity that *is* the VRM
//! root, so the triggers below are aimed directly at an entity the app owns:
//! `SetExpressions` and `ModifyExpressions` are entity events, and the marker
//! component goes on the same spawn.
//!
//! The triggers write the root's `VrmExpressionWeights`, which
//! `apply_expression_morph_binds` (scheduled by `VrmGltfPlugin`) distributes
//! into the meshes' morph targets in the same frame.
//!
//! A trigger fired before the root is `Initialized` is still a no-op: the root
//! has no `VrmExpressionWeights` until the scene is bound, and these events are
//! not buffered. To set an initial expression, wait for readiness with a scoped
//! `On<Insert, Initialized>` observer.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

/// Marks the avatar this example drives.
#[derive(Component)]
struct Avatar;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin))
        .add_systems(Startup, (spawn_light, spawn_camera, spawn_avatar))
        .add_systems(Update, control_expressions)
        .run();
}

fn spawn_light(mut commands: Commands) {
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
        // A default camera does not render the exclusive first-person layers.
        third_person_camera_layers(),
        Transform::from_xyz(0., 0.8, 2.5),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        VrmHandle(asset_server.load("vrm/Elmer.vrm")),
        Avatar,
        Transform::default(),
    ));
}

fn control_expressions(
    mut commands: Commands,
    avatars: Query<Entity, With<Avatar>>,
    input: Res<ButtonInput<KeyCode>>,
) {
    for vrm in &avatars {
        // SetExpressions: replaces all overrides
        if input.just_pressed(KeyCode::Digit1) {
            commands.trigger(SetExpressions::single(vrm, "happy", 1.0));
        }
        if input.just_pressed(KeyCode::Digit2) {
            commands.trigger(SetExpressions::single(vrm, "angry", 1.0));
        }
        if input.just_pressed(KeyCode::Digit3) {
            commands.trigger(SetExpressions::single(vrm, "sad", 1.0));
        }
        if input.just_pressed(KeyCode::Digit4) {
            commands.trigger(SetExpressions::single(vrm, "blink", 1.0));
        }
        // ModifyExpressions::mouth: lip-sync friendly (resets other vowels)
        if input.just_pressed(KeyCode::Digit5) {
            commands.trigger(ModifyExpressions::mouth(vrm, "aa", 1.0));
        }
        if input.just_pressed(KeyCode::Digit6) {
            commands.trigger(ModifyExpressions::mouth(vrm, "ih", 1.0));
        }
        if input.just_pressed(KeyCode::Digit7) {
            commands.trigger(ModifyExpressions::mouth(vrm, "ou", 1.0));
        }
        if input.just_pressed(KeyCode::Digit8) {
            commands.trigger(ModifyExpressions::mouth(vrm, "ee", 1.0));
        }
        if input.just_pressed(KeyCode::Digit0) {
            commands.trigger(ClearExpressions { entity: vrm });
        }
    }
}
