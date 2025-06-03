use crate::vrma::animation::{VrmAnimationGraph, VrmaAnimationPlayers};
use crate::vrma::LoadedVrma;
use bevy::app::{App, Update};
use bevy::prelude::*;

/// At the timing when the spawn of the Vrma's animation player is completed,
/// register the animation graph and associate the Player's entity with the root entity.
/// register the animation graph and associate the Player's entity with the root entity.
pub struct VrmaAnimationSetupPlugin;

impl Plugin for VrmaAnimationSetupPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, (setup_vrma_player, trigger_loaded_vrma));

        #[cfg(feature = "reflect")]
        {
            app.register_type::<InitializedAnimationPlayers>();
        }
    }
}

#[derive(Component, Default)]
#[cfg_attr(
    feature = "reflect",
    derive(Reflect, serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(
    feature = "reflect",
    reflect(Component, Serialize, Deserialize, Default)
)]
struct InitializedAnimationPlayers;

pub(crate) fn setup_vrma_player(
    mut commands: Commands,
    mut vrma: Query<(&mut VrmaAnimationPlayers, &VrmAnimationGraph)>,
    players: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
) {
    players.iter().for_each(|player_entity| {
        let mut entity = player_entity;
        loop {
            if let Ok((mut players, animation_graph)) = vrma.get_mut(entity) {
                players.push(player_entity);
                commands
                    .entity(player_entity)
                    .insert(AnimationGraphHandle(animation_graph.handle.clone()));
                commands.entity(entity).insert(InitializedAnimationPlayers);
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

fn trigger_loaded_vrma(
    mut commands: Commands,
    vrma: Query<(Entity, &ChildOf), Added<InitializedAnimationPlayers>>,
) {
    for (vrma_entity, child_of) in vrma.iter() {
        commands.entity(vrma_entity).trigger(LoadedVrma {
            vrm: child_of.parent(),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{test_app, TestResult};
    use crate::vrma::animation::setup::{setup_vrma_player, InitializedAnimationPlayers};
    use crate::vrma::animation::{VrmAnimationGraph, VrmaAnimationPlayers};

    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::{AnimationPlayer, Commands};

    #[test]
    fn setup_animation_player() -> TestResult {
        let mut app = test_app();
        app.world_mut().run_system_once(|mut commands: Commands| {
            let vrma = commands
                .spawn((
                    VrmAnimationGraph::default(),
                    VrmaAnimationPlayers::default(),
                ))
                .with_child(AnimationPlayer::default())
                .id();
            commands.spawn_empty().add_child(vrma);
        })?;
        app.world_mut().run_system_once(setup_vrma_player)?;
        assert!(app
            .world_mut()
            .query::<&InitializedAnimationPlayers>()
            .single(app.world_mut())
            .is_ok());
        Ok(())
    }
}
