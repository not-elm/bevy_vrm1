//! This example shows how to make a VRM model look at a specific entity.
//! The VRM model will track a red cube as its target.
//! The cube can be freely moved by dragging it with the mouse.

use bevy::prelude::*;
use bevy_vrm1::vrm::loader::VrmHandle;
use bevy_vrm1::vrm::look_at::LookAt;
use bevy_vrm1::vrm::VrmPlugin;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, MeshPickingPlugin))
        .add_systems(Startup, (spawn_camera, spawn_vrm, spawn_directional_light))
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
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 1.0, 3.5)));
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
            Transform::from_xyz(0.5, 2., 1.),
        ))
        .observe(apply_drag_move_cube)
        .id();
    commands.spawn((
        VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
        LookAt::Target(cube),
    ));
}

fn apply_drag_move_cube(
    trigger: Trigger<Pointer<Drag>>,
    mut transforms: Query<&mut Transform>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    parents: Query<&ChildOf>,
) {
    let (camera, camera_gtf) = cameras.single().expect("expected a camera");
    let Ok(ray) = camera.viewport_to_world(camera_gtf, trigger.pointer_location.position) else {
        return;
    };
    let Ok(mut tf) = transforms.get_mut(trigger.target) else {
        return;
    };
    let plane = InfinitePlane3d::new(camera_gtf.back());
    let Some(distance) = ray.intersect_plane(tf.translation, plane) else {
        return;
    };
    tf.translation = ray.get_point(distance);
}
