use bevy::prelude::*;
use bevy_vrm1::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin))
        .add_systems(Startup, (spawn_camera_and_vrm, spawn_directional_light))
        .run();
}

fn spawn_camera_and_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.3, 1.0)));
    commands.spawn((
        VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
        LookAt::Cursor,
        BodyTracking::default(),
    ));
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, 1.0, 0.0)),
    ));
}
