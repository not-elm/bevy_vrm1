use bevy::prelude::*;
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};
use bevy_vrm1::vrm::loader::VrmHandle;
use bevy_vrm1::vrm::look_at::LookAt;
use bevy_vrm1::vrm::VrmPlugin;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            PanOrbitCameraPlugin,
            VrmPlugin,
            MeshPickingPlugin,
        ))
        .add_systems(Startup, (spawn_camera, spawn_vrm, spawn_directional_light))
        .add_systems(Update, rotate)
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

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        PanOrbitCamera::default(),
        Transform::from_xyz(0.0, 2.5, 3.5),
    ));
}

fn spawn_vrm(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
) {
    let cube = commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE / 4.))),
            MeshMaterial3d(
                materials.add(StandardMaterial::from_color(Color::linear_rgb(1., 0., 0.))),
            ),
            Transform::from_xyz(1., 2., 1.),
            Rotation,
        ))
        .id();
    commands.spawn((
        VrmHandle(asset_server.load("models/AliciaSolid.vrm")),
        LookAt::Target(cube),
    ));
}

#[derive(Component)]
struct Rotation;

fn rotate(
    mut cube: Query<&mut Transform, With<Rotation>>,
    time: Res<Time>,
) {
    for mut transform in cube.iter_mut() {
        transform.rotate_around(Vec3::ZERO, Quat::from_rotation_z(time.delta_secs()));
    }
}
