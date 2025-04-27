use crate::vrma::animation::{AnimationPlayerEntityTo, VrmAnimationGraph};
use bevy::app::{App, Update};
use bevy::prelude::{
    Added, AnimationGraphHandle, AnimationPlayer, ChildOf, Entity, ParallelCommands, Plugin, Query,
};

/// At the timing when the spawn of the Vrma's animation player is completed,
/// register the animation graph and associate the Player's entity with the root entity.
/// register the animation graph and associate the Player's entity with the root entity.
pub struct VrmaAnimationSetupPlugin;

impl Plugin for VrmaAnimationSetupPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, (setup_vrma_player,));
    }
}

pub(crate) fn setup_vrma_player(
    par_commands: ParallelCommands,
    vrma: Query<(Entity, &VrmAnimationGraph)>,
    players: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
) {
    players.par_iter().for_each(|player_entity| {
        let mut entity = player_entity;
        loop {
            if let Ok((vrma_entity, animation_graph)) = vrma.get(entity) {
                par_commands.command_scope(|mut commands| {
                    commands
                        .entity(vrma_entity)
                        .insert(AnimationPlayerEntityTo(player_entity));
                    commands
                        .entity(player_entity)
                        .insert(AnimationGraphHandle(animation_graph.handle.clone()));
                });
                break;
            }

            if let Ok(child_of) = parents.get(entity) {
                entity = child_of.parent();
            } else {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::tests::{test_app, TestResult};
    use crate::vrma::animation::setup::setup_vrma_player;
    use crate::vrma::animation::{AnimationPlayerEntityTo, VrmAnimationGraph};

    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::{AnimationPlayer, Commands};

    #[test]
    fn setup_animation_player() -> TestResult {
        let mut app = test_app();
        app.world_mut().run_system_once(|mut commands: Commands| {
            commands
                .spawn(VrmAnimationGraph::default())
                .with_child(AnimationPlayer::default());
        })?;
        app.world_mut().run_system_once(setup_vrma_player)?;
        assert!(app
            .world_mut()
            .query::<&AnimationPlayerEntityTo>()
            .single(app.world_mut())
            .is_ok());
        Ok(())
    }
}
