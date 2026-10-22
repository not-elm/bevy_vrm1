//! You can transition to the corresponding animation by pressing the number keys 1 to 4.
//!
//! We use [`AnimationTransitions`] internally for bone animations to achieve smooth transitions,
//! but there is an issue where interpolation fails if the initial poses of the source and target VRMAs differ.
//! If anyone has a good solution, please feel free to open an issue or submit a PR.
//!
//! 1. `VRMA_01.vrma`
//! 2. `VRMA_02.vrma`
//! 3. `VRMA_03.vrma`
//! 4. `different_pose.vrma`

use bevy::animation::RepeatAnimation;
use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::time::Duration;

#[derive(Component, Default)]
struct VrmaNo<const I: usize>;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmaPlugin))
        .add_systems(Startup, (spawn_directional_light, spawn_camera, spawn_vrm))
        .add_systems(
            Update,
            (
                play_vrma::<1>.run_if(input_just_pressed(KeyCode::Digit1)),
                play_vrma::<2>.run_if(input_just_pressed(KeyCode::Digit2)),
                play_vrma::<3>.run_if(input_just_pressed(KeyCode::Digit3)),
                play_vrma::<4>.run_if(input_just_pressed(KeyCode::Digit4)),
            ),
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
    commands.spawn((Camera3d::default(), Transform::from_xyz(0., 0.8, 2.5)));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands
        .spawn(VrmHandle(asset_server.load("vrm/AliciaSolid.vrm")))
        .with_children(|cmd| {
            cmd.spawn((
                VrmaNo::<1>,
                VrmaHandle(asset_server.load("vrma/VRMA_01.vrma")),
            ));
            cmd.spawn((
                VrmaNo::<2>,
                VrmaHandle(asset_server.load("vrma/VRMA_02.vrma")),
            ));
            cmd.spawn((
                VrmaNo::<3>,
                VrmaHandle(asset_server.load("vrma/VRMA_03.vrma")),
            ));
            cmd.spawn((
                VrmaNo::<4>,
                VrmaHandle(asset_server.load("vrma/different_pose.vrma")),
            ));
        });
}

fn play_vrma<const I: usize>(
    mut commands: Commands,
    vrmas: Query<Entity, With<VrmaNo<I>>>,
) {
    let Ok(vrma_entity) = vrmas.single() else {
        return;
    };
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::from_millis(300),
        vrma: vrma_entity,
        reset_spring_bones: false,
    });
}
