use crate::prelude::ChildSearcher;
use crate::vrm::Vrm;
use crate::vrm::spring_bone::{SpringJointState, SpringRoot};
use crate::vrma::animation::animation_graph::{NeedsBake, VrmaBake};
use crate::vrma::animation::expressions::{VrmaExpressionRegistry, reset_expression_weights};
use crate::vrma::animation::properties::VrmExpressionWeights;
use crate::vrma::{
    VrmAnimationGraphState, VrmAnimationNodeIndex, VrmaNeedsRestore, VrmaPlaybackState,
    VrmaRetargetPending,
};
use bevy::animation::{ActiveAnimation, AnimationPlayer, RepeatAnimation};
use bevy::app::{AnimationSystems, App, Plugin, SceneSpawnerSystems, SpawnScene};
use bevy::prelude::*;
use std::time::Duration;

/// The trigger event to play the Vrma's animation.
///
/// You need to emit this via [`On`] with the target entity of the VRMA you want to play the animation on.
///
/// If there are multiple VRMA entities, the animation of all other VRMAs will be stopped except for the one specified in the trigger.
#[derive(EntityEvent, Debug, Reflect)]
pub struct PlayVrma {
    #[event_target]
    pub vrma: Entity,

    /// Repetition behavior of an animation.
    pub repeat: RepeatAnimation,

    /// A time until the existing animation fades out.
    pub transition_duration: Duration,

    /// If true, resets all `SpringBone` velocities on the parent VRM entity
    /// to prevent bouncing caused by sudden bone movements during animation transitions.
    pub reset_spring_bones: bool,
}

impl PlayVrma {
    /// Creates a new `PlayVrma` event with default settings.
    ///
    /// Default repeat is [`RepeatAnimation::Never`] and transition duration is 300 milliseconds.
    pub fn new(entity: Entity) -> Self {
        Self {
            vrma: entity,
            repeat: RepeatAnimation::Never,
            transition_duration: Duration::from_millis(300),
            reset_spring_bones: false,
        }
    }
}

/// The trigger event to stop the Vrma's animation.
///You need to emit this via [`On`] with the target entity of the VRMA you want to stop the animation on.
#[derive(EntityEvent, Debug)]
pub struct StopVrma {
    pub entity: Entity,
}

pub(super) struct VrmaAnimationPlayPlugin;

impl Plugin for VrmaAnimationPlayPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<PlayVrma>()
            .add_observer(apply_play_vrma)
            .add_observer(apply_stop_vrma)
            .add_systems(Last, snapshot_vrma_playback)
            .add_systems(
                SpawnScene,
                snapshot_vrma_playback.before(SceneSpawnerSystems::WorldInstanceSpawn),
            )
            .add_systems(
                PostUpdate,
                restore_vrma_playback
                    .after(VrmaBake)
                    .before(AnimationSystems),
            );
    }
}

fn apply_play_vrma(
    trigger: On<PlayVrma>,
    mut commands: Commands,
    mut players: Query<(
        &mut Transform,
        &mut AnimationPlayer,
        Option<&mut AnimationTransitions>,
    )>,
    searcher: ChildSearcher,
    parents: Query<&ChildOf>,
    childrens: Query<&Children>,
    vrmas: Query<(
        &VrmAnimationNodeIndex,
        Has<VrmaRetargetPending>,
        Has<NeedsBake>,
    )>,
    expression_registries: Query<&VrmaExpressionRegistry>,
    mut roots: Query<&mut VrmExpressionWeights>,
    spring_roots: Query<&SpringRoot>,
    mut joint_states: Query<&mut SpringJointState>,
    mut playback: Query<&mut VrmaPlaybackState>,
) {
    let vrma_entity = trigger.event_target();
    let Ok(ChildOf(vrm_entity)) = parents.get(vrma_entity) else {
        return;
    };
    let Ok((node_index, retarget_pending, needs_bake)) = vrmas.get(vrma_entity) else {
        return;
    };
    // An unbaked clip is started by the restore once it is ready.
    let ready = !retarget_pending && !needs_bake;
    let live = if ready {
        play_humanoid_bone_animation(
            *vrm_entity,
            node_index.0,
            trigger.repeat,
            trigger.transition_duration,
            &searcher,
            &mut players,
        )
    } else {
        None
    };
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
    if trigger.reset_spring_bones {
        reset_spring_bone_velocities(*vrm_entity, &spring_roots, &mut joint_states, &childrens);
    }
    let played = live.is_some();
    if let Ok(mut state) = playback.get_mut(vrma_entity) {
        state.active = true;
        state.animation = Some(live.unwrap_or_else(|| {
            let mut animation = ActiveAnimation::default();
            animation.set_repeat(trigger.repeat);
            animation
        }));
    }
    if played {
        commands.entity(vrma_entity).remove::<VrmaNeedsRestore>();
    } else {
        commands.entity(vrma_entity).insert(VrmaNeedsRestore);
    }
}

/// Recursively traverses descendants of `entity` to find all [`SpringRoot`] components
/// and resets the velocity of their [`SpringJointState`]s.
fn reset_spring_bone_velocities(
    entity: Entity,
    spring_roots: &Query<&SpringRoot>,
    joint_states: &mut Query<&mut SpringJointState>,
    children: &Query<&Children>,
) {
    let Ok(entity_children) = children.get(entity) else {
        return;
    };
    for child in entity_children.into_iter().copied() {
        if let Ok(root) = spring_roots.get(child) {
            for &joint in root.joints.iter() {
                if let Ok(mut state) = joint_states.get_mut(joint) {
                    state.reset_velocity();
                }
            }
        }
        reset_spring_bone_velocities(child, spring_roots, joint_states, children);
    }
}

fn play_humanoid_bone_animation(
    vrm: Entity,
    node_index: AnimationNodeIndex,
    repeat: RepeatAnimation,
    transition_duration: Duration,
    searcher: &ChildSearcher,
    players: &mut Query<(
        &mut Transform,
        &mut AnimationPlayer,
        Option<&mut AnimationTransitions>,
    )>,
) -> Option<ActiveAnimation> {
    let root_bone = searcher.find_root_bone(vrm)?;
    let Ok((_, mut player, Some(mut transitions))) = players.get_mut(root_bone) else {
        return None;
    };
    let live = transitions
        .play(&mut player, node_index, transition_duration)
        .set_repeat(repeat);
    Some(*live)
}

fn apply_stop_vrma(
    trigger: On<StopVrma>,
    mut rig_entities: Query<&mut AnimationPlayer>,
    vrmas: Query<&VrmAnimationNodeIndex>,
    parents: Query<&ChildOf>,
    expression_registries: Query<&VrmaExpressionRegistry>,
    mut roots: Query<&mut VrmExpressionWeights>,
    searcher: ChildSearcher,
    mut playback: Query<&mut VrmaPlaybackState>,
) {
    let vrma_entity = trigger.event_target();
    let Ok(ChildOf(vrm)) = parents.get(vrma_entity) else {
        return;
    };
    if let Ok(mut state) = playback.get_mut(vrma_entity) {
        state.active = false;
        state.animation = None;
    }
    let Ok(node_index) = vrmas.get(vrma_entity) else {
        return;
    };
    if let Some(root_bone) = searcher.find_root_bone(*vrm)
        && let Ok(mut player) = rig_entities.get_mut(root_bone)
    {
        player.stop(node_index.0);
    }
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
}

/// Records each VRMA's live playback and the root bone's transitions, so a
/// rebuilt rig can restore them.
fn snapshot_vrma_playback(
    searcher: ChildSearcher,
    avatars: Query<(Entity, &Children), With<Vrm>>,
    mut vrmas: Query<(
        Option<&VrmAnimationNodeIndex>,
        Option<&mut VrmaPlaybackState>,
        Has<VrmaNeedsRestore>,
    )>,
    players: Query<&AnimationPlayer>,
    transitions: Query<&AnimationTransitions>,
    mut states: Query<&mut VrmAnimationGraphState>,
) {
    for (vrm, children) in avatars.iter() {
        let Some(root_bone) = searcher.find_root_bone(vrm) else {
            continue;
        };
        let Ok(player) = players.get(root_bone) else {
            continue;
        };
        if let Ok(live) = transitions.get(root_bone)
            && let Ok(mut state) = states.get_mut(vrm)
        {
            state.transitions = live.clone();
        }
        for child in children.iter() {
            let Ok((Some(index), Some(mut state), pending_restore)) = vrmas.get_mut(child) else {
                continue;
            };
            if pending_restore {
                continue;
            }
            match player.animation(index.0) {
                Some(active) => {
                    state.active = true;
                    state.animation = Some(*active);
                }
                None => {
                    state.active = false;
                    state.animation = None;
                }
            }
        }
    }
}

/// Re-applies a recorded playback to a rebuilt player once the clip is baked.
fn restore_vrma_playback(
    mut commands: Commands,
    searcher: ChildSearcher,
    parents: Query<&ChildOf>,
    vrmas: Query<
        (Entity, &VrmaPlaybackState, &VrmAnimationNodeIndex),
        (
            With<VrmaNeedsRestore>,
            Without<NeedsBake>,
            Without<VrmaRetargetPending>,
        ),
    >,
    mut players: Query<&mut AnimationPlayer>,
) {
    for (vrma, state, index) in vrmas.iter() {
        let Ok(parent) = parents.get(vrma) else {
            commands.entity(vrma).remove::<VrmaNeedsRestore>();
            continue;
        };
        let Some(root_bone) = searcher.find_root_bone(parent.parent()) else {
            continue;
        };
        let Ok(mut player) = players.get_mut(root_bone) else {
            continue;
        };
        if state.active {
            let live = player.play(index.0);
            *live = state.animation.unwrap_or_default();
        }
        commands.entity(vrma).remove::<VrmaNeedsRestore>();
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrma::animation::animation_graph::NeedsBake;
    use crate::vrma::animation::play::VrmaAnimationPlayPlugin;
    use crate::vrma::{
        VrmAnimationNodeIndex, VrmaNeedsRestore, VrmaPlaybackState, VrmaRetargetPending,
    };
    use bevy::animation::graph::AnimationGraph;
    use bevy::animation::{
        ActiveAnimation, AnimationClip, AnimationPlugin, AnimationTargetId, RepeatAnimation,
        advance_animations, animated_field,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy_test_helper::system::SystemExt;
    use std::time::Duration;

    #[test]
    fn test_play_vrma() {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);

        let vrm = app.world_mut().spawn_empty().id();
        let vrma = app.world_mut().spawn(VrmAnimationNodeIndex::default()).id();
        app.world_mut().commands().entity(vrm).add_child(vrma);

        app.world_mut().commands().entity(vrm).with_child((
            Name::new(Vrm::ROOT_BONE),
            Transform::default(),
            AnimationPlayer::default(),
            AnimationTransitions::default(),
        ));

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(PlayVrma::new);
        app.update();

        app.world_mut()
            .run_system_once(|player: Query<&AnimationPlayer>| {
                let player = player.single().expect("Failed to find AnimationPlayer");
                assert!(!player.all_finished());
            })
            .unwrap();
    }

    #[test]
    fn test_stop_vrma() {
        let mut app = test_app();
        app.add_plugins(VrmaAnimationPlayPlugin);

        let vrm = app.world_mut().spawn_empty().id();
        let vrma = app.world_mut().spawn(VrmAnimationNodeIndex::default()).id();
        app.world_mut().commands().entity(vrm).add_child(vrma);

        app.world_mut().commands().entity(vrm).with_child((
            Name::new(Vrm::ROOT_BONE),
            AnimationPlayer::default(),
            AnimationTransitions::default(),
        ));

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(PlayVrma::new);
        app.update();

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(|entity| StopVrma { entity });
        app.update();

        app.world_mut()
            .run_system_once(|player: Query<&AnimationPlayer>| {
                let player = player.single().expect("Failed to find AnimationPlayer");
                assert!(player.all_finished());
            })
            .unwrap();
    }

    /// Returns `(app, root_bone, vrma, node)`.
    fn reload_fixture() -> (App, Entity, Entity, AnimationNodeIndex) {
        let mut app = test_app();
        let vrm = app.world_mut().spawn(Vrm).id();
        let root_bone = app
            .world_mut()
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                ChildOf(vrm),
            ))
            .id();
        let node = AnimationNodeIndex::new(1);
        let vrma = app
            .world_mut()
            .spawn((
                Vrma,
                ChildOf(vrm),
                VrmAnimationNodeIndex(node),
                VrmaPlaybackState::default(),
            ))
            .id();
        (app, root_bone, vrma, node)
    }

    /// `Debug` is the only observable covering the private fields of `ActiveAnimation`.
    #[test]
    fn a_reload_restores_every_field_of_the_live_animation() -> TestResult {
        let mut app = test_app();
        app.add_plugins(AnimationPlugin);

        let target = AnimationTargetId::from_name(&Name::new("destination_spine"));
        let rotation = Quat::from_rotation_x(0.5);
        let mut clip = AnimationClip::default();
        clip.add_curve_to_target(
            target,
            AnimatableCurve::new(
                animated_field!(Transform::rotation),
                AnimatableKeyframeCurve::new([(0.0, rotation), (1.0, rotation)]).unwrap(),
            ),
        );
        let clip = app
            .world_mut()
            .resource_mut::<Assets<AnimationClip>>()
            .add(clip);
        let (graph, nodes) = AnimationGraph::from_clips([clip]);
        let node = nodes[0];
        let graph = app
            .world_mut()
            .resource_mut::<Assets<AnimationGraph>>()
            .add(graph);

        let vrm = app.world_mut().spawn(Vrm).id();
        let root_bone = app
            .world_mut()
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                AnimationGraphHandle(graph),
                ChildOf(vrm),
            ))
            .id();
        let vrma = app
            .world_mut()
            .spawn((
                Vrma,
                ChildOf(vrm),
                VrmAnimationNodeIndex(node),
                VrmaPlaybackState::default(),
            ))
            .id();

        {
            let mut root = app.world_mut().entity_mut(root_bone);
            let mut player = root.get_mut::<AnimationPlayer>().unwrap();
            player
                .play(node)
                .set_repeat(RepeatAnimation::Count(2))
                .set_speed(2.0)
                .set_seek_time(0.1)
                .set_weight(0.7);
        }
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(1.0));
        app.world_mut().run_system_once(advance_animations).unwrap();

        let live = *app
            .world()
            .get::<AnimationPlayer>(root_bone)
            .unwrap()
            .animation(node)
            .expect("the clip is playing");
        assert!(live.completions() >= 1, "the live clip completed once");
        assert!(!live.is_finished(), "a Count(2) clip is not finished yet");

        app.world_mut()
            .run_system_once(super::snapshot_vrma_playback)
            .unwrap();
        let snapshot = app
            .world()
            .get::<VrmaPlaybackState>(vrma)
            .unwrap()
            .clone()
            .animation
            .expect("the snapshot keeps the whole live animation");
        assert_eq!(format!("{snapshot:?}"), format!("{live:?}"));

        let new_root_bone = app
            .world_mut()
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                ChildOf(vrm),
            ))
            .id();
        app.world_mut().entity_mut(root_bone).despawn();
        app.world_mut().entity_mut(vrma).insert(VrmaNeedsRestore);

        app.world_mut()
            .run_system_once(super::restore_vrma_playback)
            .unwrap();

        let restored = *app
            .world()
            .get::<AnimationPlayer>(new_root_bone)
            .unwrap()
            .animation(node)
            .expect("the clip is playing on the new player");
        assert_eq!(
            format!("{restored:?}"),
            format!("{live:?}"),
            "every field, including the private progress counters, survives"
        );
        assert_eq!(restored.completions(), live.completions());
        assert_eq!(restored.is_finished(), live.is_finished());
        success!()
    }

    #[test]
    fn the_snapshot_clears_a_clip_that_left_the_player() -> TestResult {
        let (mut app, root_bone, vrma, node) = reload_fixture();
        {
            let mut root_bone_entity = app.world_mut().entity_mut(root_bone);
            let mut player = root_bone_entity.get_mut::<AnimationPlayer>().unwrap();
            player.play(node);
            player.stop(node);
        }
        app.world_mut().entity_mut(vrma).insert(VrmaPlaybackState {
            active: true,
            animation: Some(ActiveAnimation::default()),
        });

        app.world_mut()
            .run_system_once(super::snapshot_vrma_playback)
            .unwrap();

        let state = app.world().get::<VrmaPlaybackState>(vrma).unwrap();
        assert!(
            !state.active,
            "a clip no longer in the player is not active"
        );
        assert!(
            state.animation.is_none(),
            "and it carries no live animation"
        );
        success!()
    }

    #[test]
    fn the_snapshot_keeps_a_pending_restore() -> TestResult {
        let (mut app, _root_bone, vrma, _node) = reload_fixture();
        let mut animation = ActiveAnimation::default();
        animation.set_seek_time(0.4);
        app.world_mut().entity_mut(vrma).insert((
            VrmaPlaybackState {
                active: true,
                animation: Some(animation),
            },
            VrmaNeedsRestore,
        ));

        app.world_mut()
            .run_system_once(super::snapshot_vrma_playback)
            .unwrap();

        let state = app.world().get::<VrmaPlaybackState>(vrma).unwrap();
        assert!(state.active, "a half-built player must not clear the state");
        assert!((state.animation.unwrap().seek_time() - 0.4).abs() < 1e-4);
        success!()
    }

    #[test]
    fn the_restore_waits_for_the_bake() -> TestResult {
        let (mut app, root_bone, vrma, node) = reload_fixture();
        app.world_mut().entity_mut(vrma).insert((
            VrmaPlaybackState {
                active: true,
                animation: Some(ActiveAnimation::default()),
            },
            VrmaNeedsRestore,
            NeedsBake,
        ));

        app.world_mut()
            .run_system_once(super::restore_vrma_playback)
            .unwrap();

        assert!(
            app.world()
                .get::<AnimationPlayer>(root_bone)
                .unwrap()
                .animation(node)
                .is_none(),
            "an unbaked clip is not played"
        );
        assert!(
            app.world().get::<VrmaNeedsRestore>(vrma).is_some(),
            "the restore is retried once the bake has run"
        );
        success!()
    }

    #[test]
    fn the_restore_blocks_an_unretargeted_clip() -> TestResult {
        let (mut app, root_bone, vrma, node) = reload_fixture();
        app.world_mut().entity_mut(vrma).insert((
            VrmaPlaybackState {
                active: true,
                animation: Some(ActiveAnimation::default()),
            },
            VrmaNeedsRestore,
            VrmaRetargetPending,
        ));

        app.world_mut()
            .run_system_once(super::restore_vrma_playback)
            .unwrap();

        assert!(
            app.world()
                .get::<AnimationPlayer>(root_bone)
                .unwrap()
                .animation(node)
                .is_none(),
            "an unretargeted clip is not played"
        );
        assert!(
            app.world().get::<VrmaNeedsRestore>(vrma).is_some(),
            "the restore stays queued for after the retarget"
        );
        success!()
    }

    #[test]
    fn a_stop_before_the_restore_wins() -> TestResult {
        let (mut app, root_bone, vrma, node) = reload_fixture();
        app.add_observer(super::apply_stop_vrma);
        app.world_mut().entity_mut(vrma).insert((
            VrmaPlaybackState {
                active: true,
                animation: Some(ActiveAnimation::default()),
            },
            VrmaNeedsRestore,
        ));
        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(|entity| StopVrma { entity });
        app.update();
        app.world_mut()
            .run_system_once(super::restore_vrma_playback)
            .unwrap();

        assert!(
            app.world()
                .get::<AnimationPlayer>(root_bone)
                .unwrap()
                .animation(node)
                .is_none(),
            "a stopped clip is not restarted"
        );
        assert!(
            app.world().get::<VrmaNeedsRestore>(vrma).is_none(),
            "the marker is cleared once the rig is reachable"
        );
        success!()
    }
}
