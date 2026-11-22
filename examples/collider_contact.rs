//! `AliciaSolid` ships sphere colliders; this example adds one capsule beside a hair strand.
//!
//! Space: teleport the avatar and reseed the spring joints. The hair starts at its live pose.
//! 1: teleport without reseeding. The old tail position and inertia make the hair snap.
//! 2: switch between two VRMA poses with `reset_spring_bones: true`.
//! 3: switch poses with the flag off, so the previous spring state carries over.

use bevy::animation::RepeatAnimation;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::time::Duration;

#[derive(Component, Default)]
struct PoseVrma(bool);

#[derive(Resource, Default)]
struct PendingReseed(bool);

#[derive(Resource, Default)]
struct PoseVrmaChoice(bool);

#[derive(Resource, Default)]
struct TeleportSide(bool);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmaPlugin))
        .init_resource::<PendingReseed>()
        .init_resource::<PoseVrmaChoice>()
        .init_resource::<TeleportSide>()
        .add_systems(Startup, (spawn_directional_light, spawn_camera, spawn_vrm))
        .add_systems(
            Update,
            (
                add_contact_capsule,
                teleport_with_reseed.run_if(input_just_pressed(KeyCode::Space)),
                teleport_without_reseed.run_if(input_just_pressed(KeyCode::Digit1)),
                play_pose::<true>.run_if(input_just_pressed(KeyCode::Digit2)),
                play_pose::<false>.run_if(input_just_pressed(KeyCode::Digit3)),
            ),
        )
        .add_systems(
            PostUpdate,
            reseed_after_teleport
                .after(VrmSystemSets::PropagateAfterExpressions)
                .before(VrmSystemSets::SpringBone),
        )
        .run();
}

fn spawn_directional_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 3.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.8, 2.5).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands
        .spawn(VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")))
        .with_children(|cmd| {
            cmd.spawn((
                PoseVrma(false),
                VrmaHandle(asset_server.load("vrma/VRMA_01.vrma")),
            ));
            cmd.spawn((
                PoseVrma(true),
                VrmaHandle(asset_server.load("vrma/different_pose.vrma")),
            ));
        });
}

fn add_contact_capsule(
    mut commands: Commands,
    roots: Query<(Entity, &SpringRoot), Added<SpringRoot>>,
    names: Query<&Name>,
    globals: Query<&GlobalTransform>,
    parents: Query<&ChildOf>,
    mut added: Local<bool>,
) {
    if *added {
        return;
    }
    for (root_entity, spring_root) in roots.iter() {
        let (Some(&head), Some(&next)) = (spring_root.joints.first(), spring_root.joints.get(1))
        else {
            continue;
        };
        let Ok(name) = names.get(head) else {
            continue;
        };
        if name.as_str() != "hair_01_01" {
            continue;
        }
        let (Ok(head_gtf), Ok(next_gtf)) = (globals.get(head), globals.get(next)) else {
            continue;
        };
        let head_pos = head_gtf.translation();
        let bone_length = head_pos.distance(next_gtf.translation());
        if bone_length < 1e-4 {
            continue;
        }
        let Ok(vrm_root) = globals.get(parents.root_ancestor(root_entity)) else {
            continue;
        };
        let center = head_pos + Vec3::X * (bone_length * 0.35);
        let shape = ColliderShape::Capsule(Capsule {
            offset: [0.0; 3],
            radius: bone_length * 0.2,
            tail: [0.0, -bone_length, 0.0],
        });
        let collider = commands
            .spawn((
                Transform::from_translation(
                    vrm_root.to_matrix().inverse().transform_point3(center),
                ),
                GlobalTransform::from_translation(center),
                ChildOf(parents.root_ancestor(root_entity)),
                shape,
            ))
            .id();
        let mut colliders = spring_root.colliders.clone();
        colliders.0.push((collider, shape));
        commands.entity(root_entity).insert(SpringRoot {
            joints: spring_root.joints.clone(),
            colliders,
            center_node: spring_root.center_node.clone(),
        });
        *added = true;
        return;
    }
}

fn teleport(
    vrms: Query<Entity, With<Initialized>>,
    mut transforms: Query<&mut Transform>,
    mut side: ResMut<TeleportSide>,
    mut pending: ResMut<PendingReseed>,
    reseed: bool,
) {
    let target_x = if side.0 { -1.2 } else { 1.2 };
    side.0 = !side.0;
    if let Ok(vrm) = vrms.single()
        && let Ok(mut transform) = transforms.get_mut(vrm)
    {
        transform.translation.x = target_x;
    }
    pending.0 = reseed;
}

fn teleport_with_reseed(
    vrms: Query<Entity, With<Initialized>>,
    transforms: Query<&mut Transform>,
    side: ResMut<TeleportSide>,
    pending: ResMut<PendingReseed>,
) {
    teleport(vrms, transforms, side, pending, true);
}

fn teleport_without_reseed(
    vrms: Query<Entity, With<Initialized>>,
    transforms: Query<&mut Transform>,
    side: ResMut<TeleportSide>,
    pending: ResMut<PendingReseed>,
) {
    teleport(vrms, transforms, side, pending, false);
}

fn play_pose<const RESET: bool>(
    mut commands: Commands,
    mut choice: ResMut<PoseVrmaChoice>,
    vrmas: Query<(Entity, &PoseVrma)>,
) {
    let target = vrmas
        .iter()
        .find(|(_, marker)| marker.0 == choice.0)
        .map(|(entity, _)| entity);
    choice.0 = !choice.0;
    let Some(vrma) = target else {
        return;
    };
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::ZERO,
        vrma,
        reset_spring_bones: RESET,
    });
}

fn reseed_after_teleport(world: &mut World) {
    let pending = std::mem::take(&mut world.resource_mut::<PendingReseed>().0);
    if !pending {
        return;
    }
    let vrm = {
        let mut query = world.query_filtered::<Entity, With<Initialized>>();
        query.iter(world).next()
    };
    if let Some(vrm) = vrm {
        world.trigger(ReseedSpringBones(vrm));
    }
}
