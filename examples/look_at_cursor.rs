//! This example demonstrates the `LookAt` functionality of VRM.
//!
//! By using [`LookAt::Cursor`], the VRM model will look at the cursor position.
//!
//!
//! Alternatively, VRM can also look at a specific target by using [`LookAt::Target`].
//! Please refer to `examples/look_at_target.rs` for more details.
//!
//! The avatar is spawned with [`VrmHandle`] on the entity that *is* the VRM
//! root, so `LookAt` needs no readiness dance: there is no `Added<Initialized>`
//! system and no entity lookup, because the entity that carries the VRM data is
//! the one the spawn creates.
//!
//! `LookAt` is evaluated against the `HeadBoneEntity`, `LeftEyeBoneEntity` and
//! `RightEyeBoneEntity` holders on that same entity. The load-time pipeline
//! writes them from `VRMC_vrm.humanoid`, and they remap when the scene is
//! instantiated.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin))
        .add_systems(Startup, (spawn_camera_and_avatar, spawn_directional_light))
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

fn spawn_camera_and_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers.
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 1.3, 1.),
    ));
    commands.spawn((
        VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
        LookAt::Cursor,
        Transform::default(),
    ));
}
