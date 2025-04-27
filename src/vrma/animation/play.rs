use crate::system_param::vrm_animation_players::VrmaPlayer;
use crate::vrm::Vrm;
use crate::vrma::retarget::CurrentRetargeting;
use crate::vrma::{RetargetSource, Vrma, VrmaEntity};
use bevy::app::{App, Plugin};
use bevy::log::info;
use bevy::prelude::{Children, Commands, Entity, Event, Query, Reflect, Trigger, With, Without};

/// The trigger event to play the Vrma's animation.
#[derive(Event, Debug, Reflect)]
pub struct PlayVrma {
    /// The entity of the Vrma to play the animation.
    pub vrma: VrmaEntity,

    /// Whether to loop the animation.
    pub repeat: bool,
}

#[derive(Event, Debug, Reflect)]
pub struct StopVrma;

pub struct VrmaAnimationPlayPlugin;

impl Plugin for VrmaAnimationPlayPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<PlayVrma>()
            .register_type::<StopVrma>()
            .add_event::<PlayVrma>()
            .add_event::<StopVrma>()
            .add_observer(observe_play_animation)
            .add_observer(observe_stop_animation);
    }
}

fn observe_play_animation(
    trigger: Trigger<PlayVrma>,
    mut commands: Commands,
    mut vrma_player: VrmaPlayer,
    children: Query<&Children, With<Vrm>>,
    vrma: Query<Entity, With<Vrma>>,
    entities: Query<(Option<&Children>, Option<&RetargetSource>), Without<Vrm>>,
) {
    let Ok(children) = children.get(trigger.target()) else {
        return;
    };
    for child in children.iter() {
        let Ok(vrma_entity) = vrma.get(*child) else {
            continue;
        };
        let vrma_entity = VrmaEntity(vrma_entity);
        if trigger.vrma == vrma_entity {
            vrma_player.play(vrma_entity, trigger.repeat);
            foreach_children(
                &mut commands,
                vrma_entity.0,
                &entities,
                &|commands, entity, target_source| {
                    if target_source.is_some() {
                        commands.entity(entity).insert(CurrentRetargeting);
                    }
                },
            );
        } else {
            vrma_player.stop(vrma_entity);
            foreach_children(
                &mut commands,
                vrma_entity.0,
                &entities,
                &|commands, entity, target_source| {
                    if target_source.is_some() {
                        commands.entity(entity).remove::<CurrentRetargeting>();
                    }
                },
            );
        }
    }
}

fn observe_stop_animation(
    trigger: Trigger<StopVrma>,
    mut commands: Commands,
    mut vrma_player: VrmaPlayer,
    children: Query<&Children, With<Vrm>>,
    vrma: Query<Entity, With<Vrma>>,
    entities: Query<(Option<&Children>, Option<&RetargetSource>), Without<Vrm>>,
) {
    let Ok(children) = children.get(trigger.target()) else {
        return;
    };

    for child in children {
        let Ok(vrma_entity) = vrma.get(*child) else {
            continue;
        };
        vrma_player.stop(VrmaEntity(vrma_entity));
        foreach_children(
            &mut commands,
            trigger.observer(),
            &entities,
            &|commands, entity, retargeting_marker| {
                if retargeting_marker.is_some() {
                    commands.entity(entity).remove::<CurrentRetargeting>();
                }
            },
        );
    }
}

fn foreach_children(
    commands: &mut Commands,
    entity: Entity,
    entities: &Query<(Option<&Children>, Option<&RetargetSource>), Without<Vrm>>,
    f: &impl Fn(&mut Commands, Entity, Option<&RetargetSource>),
) {
    let Ok((children, bone_to)) = entities.get(entity) else {
        return;
    };
    f(commands, entity, bone_to);
    if let Some(children) = children {
        for child in children.iter() {
            foreach_children(commands, *child, entities, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{test_app, TestResult};
    use crate::vrm::Vrm;
    use crate::vrma::animation::play::{PlayVrma, StopVrma, VrmaAnimationPlayPlugin};
    use crate::vrma::animation::setup::setup_vrma_player;
    use crate::vrma::animation::VrmAnimationGraph;
    use crate::vrma::{Vrma, VrmaEntity};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy::utils::default;

    #[derive(Component)]
    struct Vrma1;

    #[derive(Component)]
    struct Vrma2;

    #[derive(Component)]
    struct AnimationPlayer1;

    #[derive(Component)]
    struct AnimationPlayer2;

    #[test]
    fn play_vrma() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);
        app.world_mut().run_system_once(|mut commands: Commands| {
            AnimationClip::default();

            commands.spawn(Vrm).with_children(|cmd| {
                cmd.spawn((
                    Vrma,
                    VrmAnimationGraph {
                        nodes: vec![0.into()],
                        ..default()
                    },
                ))
                .with_child(AnimationPlayer::default());
            });
        })?;
        app.world_mut().run_system_once(setup_vrma_player)?;
        app.world_mut().run_system_once(
            |mut commands: Commands,
             vrm: Query<Entity, With<Vrm>>,
             vrma: Query<Entity, With<Vrma>>| {
                commands.entity(vrm.single().unwrap()).trigger(PlayVrma {
                    vrma: VrmaEntity(vrma.single().unwrap()),
                    repeat: false,
                });
            },
        )?;
        app.update();

        assert!(!app
            .world_mut()
            .query::<&AnimationPlayer>()
            .single(app.world())?
            .all_finished());
        Ok(())
    }

    #[test]
    fn stop_others() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);
        app.world_mut().run_system_once(|mut commands: Commands| {
            commands.spawn(Vrm).with_children(|cmd| {
                cmd.spawn((
                    Vrma1,
                    Vrma,
                    VrmAnimationGraph {
                        nodes: vec![0.into()],
                        ..default()
                    },
                ))
                .with_child((AnimationPlayer1, AnimationPlayer::default()));

                cmd.spawn((
                    Vrma,
                    Vrma2,
                    VrmAnimationGraph {
                        nodes: vec![0.into()],
                        ..default()
                    },
                ))
                .with_child((AnimationPlayer2, AnimationPlayer::default()));
            });
        })?;
        app.world_mut().run_system_once(setup_vrma_player)?;
        app.world_mut().run_system_once(
            |mut commands: Commands,
             vrm: Query<Entity, With<Vrm>>,
             vrma: Query<Entity, With<Vrma1>>| {
                commands.entity(vrm.single().unwrap()).trigger(PlayVrma {
                    vrma: VrmaEntity(vrma.single().unwrap()),
                    repeat: false,
                });
            },
        )?;
        app.world_mut().run_system_once(
            |mut commands: Commands,
             vrm: Query<Entity, With<Vrm>>,
             vrma: Query<Entity, With<Vrma2>>| {
                commands.entity(vrm.single().unwrap()).trigger(PlayVrma {
                    vrma: VrmaEntity(vrma.single().unwrap()),
                    repeat: false,
                });
            },
        )?;
        app.update();

        assert!(app
            .world_mut()
            .query_filtered::<&AnimationPlayer, With<AnimationPlayer1>>()
            .single(app.world())?
            .all_finished());
        assert!(!app
            .world_mut()
            .query_filtered::<&AnimationPlayer, With<AnimationPlayer2>>()
            .single(app.world())?
            .all_finished());
        Ok(())
    }

    #[test]
    fn stop_vrma() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);
        app.world_mut().run_system_once(|mut commands: Commands| {
            commands.spawn(Vrm).with_children(|cmd| {
                cmd.spawn((
                    Vrma,
                    VrmAnimationGraph {
                        nodes: vec![0.into()],
                        ..default()
                    },
                ))
                .with_child((AnimationPlayer1, AnimationPlayer::default()));
            });
        })?;
        app.world_mut().run_system_once(setup_vrma_player)?;
        app.world_mut().run_system_once(
            |mut commands: Commands,
             vrm: Query<Entity, With<Vrm>>,
             vrma: Query<Entity, With<Vrma>>| {
                commands.entity(vrm.single().unwrap()).trigger(PlayVrma {
                    vrma: VrmaEntity(vrma.single().unwrap()),
                    repeat: false,
                });
            },
        )?;
        app.world_mut().run_system_once(
            |mut commands: Commands, vrm: Query<Entity, With<Vrm>>| {
                commands.entity(vrm.single().unwrap()).trigger(StopVrma);
            },
        )?;
        app.update();

        assert!(app
            .world_mut()
            .query_filtered::<&AnimationPlayer, With<AnimationPlayer1>>()
            .single(app.world())?
            .all_finished());
        Ok(())
    }
}
