//! Demonstrates `VRMC_vrm.firstPerson` support.
//!
//! The main window shows the avatar in third person. Press `F` to switch the
//! main camera between third-person and first-person render layers: in
//! first-person mode the avatar's head (the part split off by `auto`)
//! disappears from the main window.
//!
//! The small dark viewport in the top-left corner is a camera attached to the
//! head bone, tilted down: it sees the floor, the cubes and the body, but
//! never the head.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, Viewport};
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

#[derive(Component)]
struct MainCamera;

#[derive(Resource, Default)]
struct FirstPersonMode {
    enabled: bool,
    saved_camera: Option<Transform>,
}

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin))
        .init_resource::<FirstPersonMode>()
        .add_systems(Startup, (spawn_main_camera, spawn_scene, spawn_vrm))
        .add_systems(
            Update,
            (
                attach_head_camera,
                (toggle_main_camera, sync_first_person_camera).chain(),
            ),
        )
        .run();
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Ground.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::new(30.0, 30.0)))),
        MeshMaterial3d(materials.add(StandardMaterial::from(Color::WHITE))),
    ));
    // Reference cubes in front of the avatar (VRM 1.0 avatars face +Z),
    // so the first-person camera has something to look at.
    for (x, z, color) in [
        (-1.0, 4.5, Color::srgb(0.9, 0.3, 0.3)),
        (0.0, 5.0, Color::srgb(0.3, 0.9, 0.3)),
        (1.0, 4.5, Color::srgb(0.3, 0.3, 0.9)),
    ] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.4, 0.4, 0.4))),
            MeshMaterial3d(materials.add(StandardMaterial::from(color))),
            Transform::from_xyz(x, 0.2, z),
        ));
    }
}

/// The main camera starts in third-person mode
/// (the `ThirdPersonCamera` observer assigns its render layers).
fn spawn_main_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        MainCamera,
        ThirdPersonCamera,
        Transform::from_xyz(0.0, 1.2, 2.5).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.spawn(VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")));
}

/// Once the avatar has finished loading, attaches a picture-in-picture camera
/// to the head bone and enables first-person mode (this performs the `auto` head split).
fn attach_head_camera(
    mut commands: Commands,
    vrms: Query<(Entity, &HeadBoneEntity), Added<HeadBoneEntity>>,
) {
    for (vrm, head) in vrms.iter() {
        commands.entity(head.0).with_children(|spawner| {
            spawner.spawn((
                Camera3d::default(),
                Camera {
                    // Render on top of the main camera.
                    order: 1,
                    viewport: Some(Viewport {
                        physical_position: UVec2::new(10, 10),
                        physical_size: UVec2::new(400, 300),
                        ..default()
                    }),
                    // A distinct background makes the viewport clearly visible.
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.05, 0.05, 0.15)),
                    ..default()
                },
                FirstPersonCamera,
                // At eye level, tilted down so the body is in view.
                Transform::from_xyz(0.0, 0.06, 0.0)
                    .looking_to(Dir3::new(Vec3::new(0.0, -0.4, 1.0)).unwrap(), Vec3::Y),
            ));
        });
        commands.entity(vrm).trigger(RequestEnableFirstPerson);
    }
}

/// Press `F` to toggle the main camera between the external third-person view
/// and a true first-person view from the avatar's eyes: both the render
/// layers and the camera transform are switched.
fn toggle_main_camera(
    keys: Res<ButtonInput<KeyCode>>,
    layers: Res<FirstPersonLayers>,
    mut mode: ResMut<FirstPersonMode>,
    mut cameras: Query<(&mut Transform, &mut RenderLayers), With<MainCamera>>,
) {
    if !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    mode.enabled = !mode.enabled;
    for (mut transform, mut render_layers) in cameras.iter_mut() {
        if mode.enabled {
            // Remember the external view to restore it later.
            mode.saved_camera = Some(*transform);
            *render_layers = RenderLayers::default().with(layers.first_person_only);
        } else {
            if let Some(saved) = mode.saved_camera.take() {
                *transform = saved;
            }
            *render_layers = RenderLayers::default().with(layers.third_person_only);
        }
    }
}

/// While first-person mode is enabled, the main camera follows the head bone
/// every frame (so it also tracks animations).
fn sync_first_person_camera(
    mode: Res<FirstPersonMode>,
    vrms: Query<&HeadBoneEntity>,
    globals: Query<&GlobalTransform>,
    mut cameras: Query<&mut Transform, With<MainCamera>>,
) {
    if !mode.enabled {
        return;
    }
    let Some(head) = vrms.iter().next() else {
        return;
    };
    let Ok(head_global) = globals.get(head.0) else {
        return;
    };
    // Eye position slightly above the head joint; VRM 1.0 avatars face +Z.
    // GlobalTransform is one frame behind, which is fine for a demo.
    let eye = head_global.transform_point(Vec3::new(0.0, 0.06, 0.0));
    let Ok(forward) = Dir3::new(head_global.rotation() * Vec3::Z) else {
        return;
    };
    for mut transform in cameras.iter_mut() {
        *transform = Transform::from_translation(eye).looking_to(forward, Vec3::Y);
    }
}
