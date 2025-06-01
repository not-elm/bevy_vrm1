use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy_vrm1::vrm::loader::VrmHandle;
use bevy_vrm1::vrm::{Vrm, VrmPlugin};
use bevy_vrm1::vrma::animation::play::PlayVrma;
use bevy_vrm1::vrma::animation::AnimationPlayerEntityTo;
use bevy_vrm1::vrma::{VrmaDuration, VrmaEntity, VrmaHandle, VrmaPlugin};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmaPlugin))
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

}
