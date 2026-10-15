//! Loads a VRM avatar through the stock Bevy glTF loader.
//!
//! This is the acceptance case for the load-time pipeline: `VrmGltfPlugin`
//! registers the VRM extension handler, so `bevy_gltf` writes every VRM
//! component into the scene asset while the file loads. Those computations stay
//! at load time; a thin scene-ready step then binds the root data onto the
//! entity the app spawned. `VrmPlugin` supplies the runtime systems (spring
//! bones, gaze control, expressions, node constraints) on top.
//!
//! [`VrmHandle`] is how an app spawns a `.vrm`. It wraps the `Handle<Gltf>` the
//! load returns and stays on the entity after it is ready. The entity it is
//! spawned on **is** the VRM root — it receives `Initialized` once the scene is
//! bound — and it keeps its id when the file is hot-reloaded or its handle is
//! replaced.

use bevy::prelude::*;
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};
use bevy_vrm1::prelude::*;

fn main() {
    App::new()
        // `VrmGltfPlugin` must be added after `DefaultPlugins`: it needs
        // `GltfPlugin` for its resource registry and `PbrPlugin`'s handler to
        // have run first.
        .add_plugins((
            DefaultPlugins,
            VrmPlugin,
            VrmGltfPlugin,
            PanOrbitCameraPlugin,
        ))
        .add_systems(
            Startup,
            (spawn_camera, spawn_avatar, spawn_directional_light),
        )
        .run();
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        // `VrmLightLayersPlugin` widens any light that carries no explicit
        // layers, so this is belt and braces: it documents that the head
        // geometry split off by `firstPerson: auto` — which lives on the
        // exclusive layer 1 — still has to reach this light's shadow map.
        all_vrm_render_layers(),
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        PanOrbitCamera::default(),
        Camera3d::default(),
        // The head meshes split off by `firstPerson: auto` live on the
        // third-person-only layer, which a default camera does not render.
        third_person_camera_layers(),
        Transform::from_xyz(0.0, 2.5, 3.5).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Ground
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::new(1000.0, 1000.0)))),
        MeshMaterial3d(materials.add(StandardMaterial::from(Color::WHITE))),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // A `.vrm` file loads like any glTF scene: the handler does the rest.
    // `VrmHandle` requires a `Transform` (and `Visibility`), which is where the
    // avatar is placed.
    commands.spawn((
        VrmHandle(asset_server.load("vrm/Elmer.vrm")),
        Transform::default(),
    ));
}
