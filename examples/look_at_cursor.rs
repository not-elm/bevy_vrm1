use bevy::prelude::*;
use bevy_vrm1::vrm::loader::VrmHandle;
use bevy_vrm1::vrm::look_at::LookAt;
use bevy_vrm1::vrm::VrmPlugin;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin))
        .add_systems(Startup, (spawn_camera_and_vrm, spawn_directional_light))
        .run();
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera_and_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let camera = commands
        .spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.3, 1.)))
        .id();

    commands.spawn((
        VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
        LookAt::Cursor {
            camera: Some(camera),
        },
    ));
}
