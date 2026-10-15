//! This example shows how to make a VRM model look at a specific entity.
//! The VRM model will track a red cube as its target.
//! The cube can be freely moved by dragging it with the mouse.
//!
//! The avatar is spawned with [`VrmHandle`] on the entity that *is* the VRM
//! root, so `LookAt::Target(cube)` goes on that same spawn: no `LookTarget`
//! resource and no `Added<Initialized>` system are needed.
//!
//! `LookAt` is evaluated against the `HeadBoneEntity`, `LeftEyeBoneEntity` and
//! `RightEyeBoneEntity` holders on that same entity. The load-time pipeline
//! writes them from `VRMC_vrm.humanoid`, and they remap when the scene is
//! instantiated.

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin, MeshPickingPlugin))
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
        Transform::from_xyz(0.0, 1.0, 3.5),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cube = commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE / 4.))),
            MeshMaterial3d(
                materials.add(StandardMaterial::from_color(Color::linear_rgb(1., 0., 0.))),
            ),
            Transform::from_xyz(0.5, 1., 1.),
        ))
        .observe(apply_drag_move_cube)
        .id();
    commands.spawn((
        VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
        LookAt::Target(cube),
        Transform::default(),
    ));
}

fn apply_drag_move_cube(
    trigger: On<Pointer<Drag>>,
    mut transforms: Query<&mut Transform>,
    cameras: Query<(&Camera, &GlobalTransform)>,
) {
    let (camera, camera_gtf) = cameras.single().expect("expected a camera");
    let Ok(ray) = camera.viewport_to_world(camera_gtf, trigger.pointer_location.position) else {
        return;
    };
    let Ok(mut tf) = transforms.get_mut(trigger.event_target()) else {
        return;
    };
    let plane = InfinitePlane3d::new(camera_gtf.back());
    let Some(distance) = ray.intersect_plane(tf.translation, plane) else {
        return;
    };
    tf.translation = ray.get_point(distance);
}
