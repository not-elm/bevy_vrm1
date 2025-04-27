use bevy::prelude::*;
use bevy_vrma::vrm::loader::VrmHandle;
use bevy_vrma::vrm::VrmPlugin;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin))
        .add_systems(Startup, (spawn_camera, spawn_vrm))
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(0., 1., 2.5)));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn(VrmHandle(asset_server.load("models/sample.vrm")));
}
