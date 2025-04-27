use crate::vrma::animation::{AnimationPlayerEntityTo, VrmAnimationGraph};
use crate::vrma::VrmaEntity;
use bevy::animation::AnimationPlayer;
use bevy::ecs::system::SystemParam;
use bevy::prelude::Query;

#[derive(SystemParam)]
pub struct VrmaPlayer<'w, 's> {
    vrma: Query<'w, 's, (&'static AnimationPlayerEntityTo, &'static VrmAnimationGraph)>,
    animation_players: Query<'w, 's, &'static mut AnimationPlayer>,
}

impl VrmaPlayer<'_, '_> {
    pub fn play(
        &mut self,
        vrma: VrmaEntity,
        is_repeat: bool,
    ) {
        let Ok((AnimationPlayerEntityTo(player_entity), graph)) = self.vrma.get(vrma.0) else {
            return;
        };
        let Ok(mut player) = self.animation_players.get_mut(*player_entity) else {
            return;
        };
        player.stop_all();
        for node in &graph.nodes {
            let controller = player.play(*node);
            if is_repeat {
                controller.repeat();
            }
        }
    }

    pub fn stop(
        &mut self,
        vrma: VrmaEntity,
    ) {
        let Ok((AnimationPlayerEntityTo(player_entity), _)) = self.vrma.get(vrma.0) else {
            return;
        };

        let Ok(mut player) = self.animation_players.get_mut(*player_entity) else {
            return;
        };
        player.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::system_param::vrm_animation_players::VrmaPlayer;
    use crate::tests::{test_app, TestResult};
    use crate::vrma::animation::{AnimationPlayerEntityTo, VrmAnimationGraph};
    use crate::vrma::VrmaEntity;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::{
        AnimationNodeIndex, AnimationPlayer, Commands, Component, Entity, Query, With,
    };
    use bevy::utils::default;

    #[derive(Component)]
    struct Target;

    #[test]
    fn run_players() -> TestResult {
        let mut app = test_app();
        app.world_mut().run_system_once(|mut commands: Commands| {
            let p1 = commands.spawn((Target, AnimationPlayer::default())).id();
            commands.spawn(AnimationPlayer::default());

            commands.spawn((
                AnimationPlayerEntityTo(p1),
                VrmAnimationGraph {
                    nodes: vec![AnimationNodeIndex::new(1)],
                    ..default()
                },
            ));
        })?;
        app.update();

        app.world_mut().run_system_once(
            |mut players: VrmaPlayer, entity: Query<Entity, With<AnimationPlayerEntityTo>>| {
                players.play(VrmaEntity(entity.single().unwrap()), false);
            },
        )?;
        app.update();

        app.world_mut()
            .run_system_once(|target: Query<&AnimationPlayer, With<Target>>| {
                assert!(!target.single().unwrap().all_finished());
            })?;
        success!()
    }

    #[test]
    fn stop_all() -> TestResult {
        let mut app = test_app();
        app.world_mut().run_system_once(|mut commands: Commands| {
            let p1 = commands.spawn((Target, AnimationPlayer::default())).id();

            commands.spawn((
                AnimationPlayerEntityTo(p1),
                VrmAnimationGraph {
                    nodes: vec![AnimationNodeIndex::new(1)],
                    ..default()
                },
            ));
        })?;
        app.update();

        app.world_mut().run_system_once(
            |mut players: VrmaPlayer, entity: Query<Entity, With<AnimationPlayerEntityTo>>| {
                players.play(VrmaEntity(entity.single().unwrap()), false);
            },
        )?;
        app.update();

        app.world_mut().run_system_once(
            |mut players: VrmaPlayer, entity: Query<Entity, With<AnimationPlayerEntityTo>>| {
                players.stop(VrmaEntity(entity.single().unwrap()));
            },
        )?;
        app.update();

        app.world_mut()
            .run_system_once(|target: Query<&AnimationPlayer, With<Target>>| {
                assert!(target.single().unwrap().all_finished());
            })?;
        success!()
    }
}
