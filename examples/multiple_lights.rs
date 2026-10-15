//! This example shows multiple directional lights on one VRM model: one circles
//! the avatar and one swings back and forth.
//!
//! Neither light sets `RenderLayers` on purpose, which makes them a worked
//! example of `VrmLightLayersPlugin` - added here, because neither `VrmPlugin`
//! nor `VrmGltfPlugin` adds it. A light that carries no render layers at all, or
//! the default `{0}`, is widened to `all_vrm_render_layers`: bevy drops a mesh
//! from a light's shadow map when the two layer sets do not intersect, and
//! `VRMC_vrm.firstPerson` puts the head meshes that `auto` splits off on the
//! exclusive layer 1. A light whose layers are set explicitly is left exactly as
//! it is, so such a light has to include layers 1 and 2 itself.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

#[derive(Component)]
struct RotateCircle;

#[derive(Component)]
struct RotateArc;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            VrmPlugin,
            VrmGltfPlugin,
            VrmLightLayersPlugin,
        ))
        .add_systems(
            Startup,
            (spawn_camera, spawn_avatar, spawn_directional_light),
        )
        .add_systems(Update, (rotate_circle, rotate_arc))
        .run();
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        RotateCircle,
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        RotateArc,
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(1.0, 1., 2.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        // A default camera does not render the exclusive first-person layers.
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 2.5, 3.5),
    ));
    // Ground
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::new(1000.0, 1000.0)))),
        MeshMaterial3d(materials.add(StandardMaterial::from(Color::WHITE))),
    ));
    // Wall
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::new(1000.0, 1000.0)))),
        MeshMaterial3d(materials.add(StandardMaterial::from(Color::WHITE))),
        Transform::from_xyz(0.0, 0.0, -2.0),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn((
        VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
        Transform::default(),
    ));
}

fn rotate_circle(
    mut lights: Query<&mut Transform, With<RotateCircle>>,
    time: Res<Time>,
) {
    for mut transform in lights.iter_mut() {
        transform.rotate(Quat::from_rotation_y(time.delta_secs() * 0.5));
    }
}

fn rotate_arc(
    mut lights: Query<&mut Transform, With<RotateArc>>,
    time: Res<Time>,
) {
    let amplitude = std::f32::consts::PI / 5.;
    let frequency = 0.5;
    let angle = (time.elapsed_secs() * std::f32::consts::TAU * frequency).sin() * amplitude;
    for mut transform in lights.iter_mut() {
        transform.rotation = Quat::from_rotation_y(angle);
    }
}
