//! This example demonstrates the use of spring bones in a VRM model to show how the character's hair and clothing can physically sway.
//! You can drag the character to move it around.

use bevy::prelude::*;
use bevy_vrm1::vrm::loader::VrmHandle;
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
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 0.5, 3.5)));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands
        .spawn(VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")))
        .observe(apply_drag_move_vrm);
}

fn apply_drag_move_vrm(
    trigger: Trigger<Pointer<Drag>>,
    mut transforms: Query<&mut Transform>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    parents: Query<&ChildOf>,
) {
    let vrm_entity = parents.root_ancestor(trigger.target);
    let (camera, camera_gtf) = cameras.single().expect("expected a camera");
    let Ok(ray) = camera.viewport_to_world(camera_gtf, trigger.pointer_location.position) else {
        return;
    };
    let Ok(mut tf) = transforms.get_mut(vrm_entity) else {
        return;
    };
    let plane = InfinitePlane3d::new(camera_gtf.back());
    let Some(distance) = ray.intersect_plane(tf.translation, plane) else {
        return;
    };
    tf.translation = ray.get_point(distance);
}
