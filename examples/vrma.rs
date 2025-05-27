use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy_vrm1::vrm::loader::VrmHandle;
use bevy_vrm1::vrm::{Vrm, VrmPlugin};
use bevy_vrm1::vrma::animation::play::PlayVrma;
use bevy_vrm1::vrma::animation::AnimationPlayerEntityTo;
use bevy_vrm1::vrma::{VrmaDuration, VrmaEntity, VrmaHandle, VrmaPlugin};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, VrmPlugin, VrmaPlugin))
        .add_event::<ChangeAnimation>()
        .add_systems(Startup, (spawn_camera, spawn_vrm))
        .add_systems(
            Update,
            (
                change_animation.run_if(added_animation_player),
                detect_animation_finish.run_if(resource_exists::<VrmaTimer>),
            ),
        )
        .run();
}

/// TODO: Provides a way to wait for the preparation of animation settings.
fn added_animation_player(players: Query<Entity, Added<AnimationPlayerEntityTo>>) -> bool {
    !players.is_empty()
}

#[derive(Resource)]
struct VrmaTimer(Timer);

#[derive(Default, Resource)]
struct Animations {
    current_index: usize,
    animations: Vec<Entity>,
}

#[derive(Event)]
struct ChangeAnimation;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(0., 1., 2.5)));
}

fn spawn_vrm(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let mut animations = Animations::default();
    let mut vrma = |index: usize, cmd: &mut RelatedSpawnerCommands<ChildOf>| {
        let entity = cmd
            .spawn(VrmaHandle(
                asset_server.load(format!("vrma/VRMA_0{index}.vrma")),
            ))
            .id();
        animations.animations.push(entity);
    };

    commands
        .spawn(VrmHandle(asset_server.load("models/AliciaSolid.vrm")))
        .with_children(|cmd| {
            vrma(1, cmd);
            vrma(2, cmd);
            vrma(3, cmd);
            vrma(4, cmd);
            vrma(5, cmd);
            vrma(6, cmd);
            vrma(7, cmd);
        });
    commands.insert_resource(animations);
}

fn change_animation(
    mut commands: Commands,
    animations: ResMut<Animations>,
    vrm: Query<Entity, With<Vrm>>,
    vrma: Query<&VrmaDuration>,
) {
    let current = animations.animations[animations.current_index];
    let Ok(duration) = vrma.get(current) else {
        return;
    };
    let Ok(vrm_entity) = vrm.single() else {
        return;
    };
    commands.entity(vrm_entity).trigger(PlayVrma {
        vrma: VrmaEntity(current),
        repeat: true,
    });
    commands.insert_resource(VrmaTimer(Timer::new(duration.0, TimerMode::Once)));
}

fn detect_animation_finish(
    mut timer: ResMut<VrmaTimer>,
    mut animations: ResMut<Animations>,
    time: Res<Time>,
) {
    if timer.0.tick(time.delta()).just_finished() {
        animations.current_index = (animations.current_index + 1) % animations.animations.len();
    }
}
