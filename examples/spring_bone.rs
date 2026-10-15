//! This example demonstrates the use of spring bones in a VRM model to show how the character's hair and clothing can physically sway.
//! This feature is enabled by default and does not require any special settings.
//!
//! Please try dragging and moving the VRM model to see the swaying of the ribbons and hair.
//!
//! `VrmGltfPlugin` builds the chains while the file loads: every
//! `VRMC_springBone` joint, collider and center node is resolved into
//! `SpringRoot`, `SpringJointProps` and `SpringJointState` on the bones of the
//! scene asset, so there is no spring-bone step left to run once the scene is
//! bound to the avatar entity.

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
        Transform::from_xyz(0.0, 0.5, 3.0),
    ));
}

fn spawn_avatar(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // Spring bones need no configuration. The entity spawned with `VrmHandle`
    // is the avatar's root, so dragging it drags the whole model.
    commands
        .spawn((
            VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")),
            Transform::default(),
        ))
        .observe(apply_drag_move_vrm);
}

/// The picked mesh is a descendant of the entity spawned with [`VrmHandle`],
/// which is the avatar's root and the entity that positions it, so its root
/// ancestor is the one to move.
fn apply_drag_move_vrm(
    trigger: On<Pointer<Drag>>,
    mut transforms: Query<&mut Transform>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    parents: Query<&ChildOf>,
) {
    let vrm_entity = parents.root_ancestor(trigger.event_target());
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
