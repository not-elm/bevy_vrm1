use crate::prelude::ChildSearcher;
use crate::vrm::Vrm;
use crate::vrm::spring_bone::{SpringJointState, SpringRoot};
use crate::vrma::animation::expressions::{VrmaExpressionRegistry, reset_expression_weights};
use crate::vrma::animation::properties::VrmExpressionWeights;
use crate::vrma::{VrmAnimationNodeIndex, VrmaNeedsRestore, VrmaPlaybackState};
use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle};
use bevy::animation::{AnimationPlayer, RepeatAnimation};
use bevy::app::{AnimationSystems, App, Plugin};
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
            // The snapshot runs after the whole frame so it holds the latest
            // live time/speed/weight before the engine can replace the rig; the
            // restore runs after the graph rebuild's commands have been applied
            // and before bevy evaluates the new player.
            .add_systems(Last, snapshot_vrma_playback)
            .add_systems(PostUpdate, restore_vrma_playback.before(AnimationSystems));
    }
}

fn apply_play_vrma(
    trigger: On<PlayVrma>,
    mut players: Query<(
        &mut Transform,
        &mut AnimationPlayer,
        Option<&mut AnimationTransitions>,
    )>,
    searcher: ChildSearcher,
    parents: Query<&ChildOf>,
    childrens: Query<&Children>,
    vrmas: Query<&VrmAnimationNodeIndex>,
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
    let Ok(node_index) = vrmas.get(vrma_entity) else {
        return;
    };
    play_humanoid_bone_animation(
        *vrm_entity,
        node_index.0,
        trigger.repeat,
        trigger.transition_duration,
        &searcher,
        &mut players,
    );
    // The expression tracks are nodes of the same clip, so the same player and
    // the same `repeat` drive them. What still needs doing is the neutral face
    // the transition blends out of: without it a clip that fades in over 300 ms
    // fades out of whatever the previous one left behind.
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
    if trigger.reset_spring_bones {
        reset_spring_bone_velocities(*vrm_entity, &spring_roots, &mut joint_states, &childrens);
    }
    // `active` is authoritative and is what a rig replacement replays; the
    // remaining fields are refreshed from the live animation each frame.
    if let Ok(mut state) = playback.get_mut(vrma_entity) {
        state.active = true;
        state.paused = false;
        state.seek_time = 0.0;
        state.speed = 1.0;
        state.weight = 1.0;
        state.repeat = trigger.repeat;
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
) {
    let Some(root_bone) = searcher.find_root_bone(vrm) else {
        return;
    };
    let Ok((_, mut player, Some(mut transitions))) = players.get_mut(root_bone) else {
        return;
    };
    transitions
        .play(&mut player, node_index, transition_duration)
        .set_repeat(repeat);
}

fn apply_stop_vrma(
    trigger: On<StopVrma>,
    mut players: Query<&mut AnimationPlayer>,
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
    // Cleared before any readiness check: a stop issued while the rig is being
    // replaced must not be lost and then replayed by the restore.
    if let Ok(mut state) = playback.get_mut(vrma_entity) {
        state.active = false;
    }
    let Ok(node_index) = vrmas.get(vrma_entity) else {
        return;
    };
    // The player is on the avatar's root bone — the same entity
    // `play_humanoid_bone_animation` starts it on — and the node it holds plays
    // the bone curves *and* the expression curves of this very `.vrma`, so one
    // `stop` covers the face as well as the body.
    if let Some(root_bone) = searcher.find_root_bone(*vrm)
        && let Ok(mut player) = players.get_mut(root_bone)
    {
        player.stop(node_index.0);
    }
    // A finished animation would otherwise leave the face frozen at its last
    // weight, since the weight lives in a map nothing else decays.
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
}

/// Records each VRMA's live playback and graph mask every frame.
///
/// A reattachment has to restore what the *old* player and graph were doing,
/// and the engine destroys both together with the old rig, so the snapshot must
/// be taken while they still exist. Only an avatar with a live root-bone player
/// and graph is read: while a rig is being replaced there is neither, and the
/// last snapshot must survive that gap unchanged.
///
/// The mask is read from the current graph node every frame because it is a
/// property of the graph asset, not of the VRMA; a rebuild recreates the node,
/// so the desired mask has to be captured somewhere that outlives the graph.
fn snapshot_vrma_playback(
    searcher: ChildSearcher,
    avatars: Query<(Entity, &Children), With<Vrm>>,
    mut vrmas: Query<(
        Option<&VrmAnimationNodeIndex>,
        Option<&mut VrmaPlaybackState>,
    )>,
    players: Query<&AnimationPlayer>,
    graph_handles: Query<&AnimationGraphHandle>,
    graphs: Option<Res<Assets<AnimationGraph>>>,
) {
    // A headless app without bevy's `AnimationPlugin` has no graph store; there
    // is no live graph to snapshot then.
    let Some(graphs) = graphs else {
        return;
    };
    for (vrm, children) in avatars.iter() {
        // No root bone, player or graph means the rig is absent (mid-replace):
        // leave every snapshot exactly as the last live frame left it.
        let Some(root_bone) = searcher.find_root_bone(vrm) else {
            continue;
        };
        let Ok(player) = players.get(root_bone) else {
            continue;
        };
        let Ok(graph_handle) = graph_handles.get(root_bone) else {
            continue;
        };
        let Some(graph) = graphs.get(graph_handle.0.id()) else {
            continue;
        };
        for child in children.iter() {
            let Ok((Some(index), Some(mut state))) = vrmas.get_mut(child) else {
                continue;
            };
            if let Some(node) = graph.graph.node_weight(index.0) {
                state.mask = node.mask;
            }
            if let Some(active) = player.animation(index.0) {
                state.active = true;
                state.paused = active.is_paused();
                state.seek_time = active.seek_time();
                state.speed = active.speed();
                state.repeat = active.repeat_mode();
                state.weight = active.weight();
            }
        }
    }
}

/// Re-applies a recorded playback to a freshly rebuilt graph and player.
///
/// Queued by the graph build for every clip that was active, so a rig
/// replacement — and a clip-set change on the same rig — keeps the `.vrma`
/// running at its old time, speed, repeat mode, weight and mask instead of the
/// app's [`LoadedVrma`](crate::prelude::LoadedVrma) observer restarting it.
///
/// Runs after the graph rebuild's commands have been applied (the node index
/// and graph handle are inserted through `Commands`) and before bevy evaluates
/// the new player, so the very first evaluated frame is the restored one.
fn restore_vrma_playback(
    mut commands: Commands,
    searcher: ChildSearcher,
    parents: Query<&ChildOf>,
    vrmas: Query<(Entity, &VrmaPlaybackState, &VrmAnimationNodeIndex), With<VrmaNeedsRestore>>,
    mut players: Query<&mut AnimationPlayer>,
    graph_handles: Query<&AnimationGraphHandle>,
    mut graphs: Option<ResMut<Assets<AnimationGraph>>>,
) {
    for (vrma, state, index) in vrmas.iter() {
        let Ok(parent) = parents.get(vrma) else {
            commands.entity(vrma).remove::<VrmaNeedsRestore>();
            continue;
        };
        let Some(root_bone) = searcher.find_root_bone(parent.parent()) else {
            // The new rig is not reachable yet; retry on a later frame.
            continue;
        };
        let Ok(mut player) = players.get_mut(root_bone) else {
            continue;
        };
        let node = index.0;
        // The graph build creates every node with mask `0`, so the captured
        // mask has to be written back onto the node before it is played.
        if let Ok(graph_handle) = graph_handles.get(root_bone)
            && let Some(graphs) = graphs.as_mut()
            && let Some(mut graph) = graphs.get_mut(graph_handle.0.id())
            && let Some(node_weight) = graph.graph.node_weight_mut(node)
        {
            node_weight.mask = state.mask;
        }
        if state.active {
            let active = player.play(node);
            active.set_seek_time(state.seek_time);
            active.set_speed(state.speed);
            active.set_repeat(state.repeat);
            active.set_weight(state.weight);
            if state.paused {
                active.pause();
            }
        }
        commands.entity(vrma).remove::<VrmaNeedsRestore>();
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrma::animation::play::VrmaAnimationPlayPlugin;
    use crate::vrma::{VrmAnimationNodeIndex, VrmaNeedsRestore, VrmaPlaybackState};
    use bevy::animation::graph::AnimationGraph;
    use bevy::animation::{AnimationClip, AnimationPlugin, RepeatAnimation};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy_test_helper::system::SystemExt;

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

    /// A recycled headless app with bevy's animation assets, a VRM root bone
    /// carrying a one-node graph and a VRMA child. Returns
    /// `(app, clip, graph, root_bone, vrma, node)`.
    fn reload_fixture() -> (
        App,
        Handle<AnimationClip>,
        Handle<AnimationGraph>,
        Entity,
        Entity,
        AnimationNodeIndex,
    ) {
        let mut app = test_app();
        app.add_plugins(AnimationPlugin);
        let clip = app
            .world_mut()
            .resource_mut::<Assets<AnimationClip>>()
            .add(AnimationClip::default());
        let (graph, nodes) = AnimationGraph::from_clips([clip.clone()]);
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
                AnimationGraphHandle(graph.clone()),
                ChildOf(vrm),
            ))
            .id();
        let vrma = app
            .world_mut()
            .spawn((
                Vrma,
                ChildOf(vrm),
                VrmAnimationNodeIndex(node),
                VrmaPlaybackState {
                    active: true,
                    paused: false,
                    seek_time: 0.0,
                    speed: 1.0,
                    repeat: RepeatAnimation::Never,
                    weight: 1.0,
                    mask: 0,
                },
                VrmaNeedsRestore,
            ))
            .id();
        (app, clip, graph, root_bone, vrma, node)
    }

    /// The restore writes the recorded time, speed, repeat, weight, mask and
    /// paused flag onto the new player and graph node, and clears the marker.
    #[test]
    fn a_reload_restores_time_speed_repeat_weight_mask_and_pause() -> TestResult {
        let (mut app, clip, graph, root_bone, vrma, node) = reload_fixture();
        app.world_mut().entity_mut(vrma).insert(VrmaPlaybackState {
            active: true,
            paused: true,
            seek_time: 0.25,
            speed: 2.0,
            repeat: RepeatAnimation::Forever,
            weight: 0.5,
            mask: 0b101,
        });

        app.world_mut()
            .run_system_once(super::restore_vrma_playback)
            .unwrap();
        app.world_mut().flush();

        let player = app.world().get::<AnimationPlayer>(root_bone).unwrap();
        let active = player.animation(node).expect("the clip is playing again");
        assert!(active.is_paused(), "the paused flag is restored");
        assert!((active.seek_time() - 0.25).abs() < 1e-4);
        assert_eq!(active.speed(), 2.0);
        assert_eq!(active.repeat_mode(), RepeatAnimation::Forever);
        assert!((active.weight() - 0.5).abs() < 1e-4);

        let graphs = app.world().resource::<Assets<AnimationGraph>>();
        assert_eq!(
            graphs
                .get(&graph)
                .unwrap()
                .graph
                .node_weight(node)
                .unwrap()
                .mask,
            0b101,
            "the graph node mask is restored"
        );
        assert!(
            app.world().get::<VrmaNeedsRestore>(vrma).is_none(),
            "the restore marker is cleared"
        );
        assert!(
            app.world()
                .resource::<Assets<AnimationClip>>()
                .get(&clip)
                .is_some()
        );
        success!()
    }

    /// The snapshot reads the live player and the live graph node, so the state
    /// the restore later replays is the real one.
    #[test]
    fn the_snapshot_captures_live_playback_and_mask() -> TestResult {
        let (mut app, _clip, graph, root_bone, vrma, node) = reload_fixture();
        {
            let mut root_bone_entity = app.world_mut().entity_mut(root_bone);
            let mut player = root_bone_entity.get_mut::<AnimationPlayer>().unwrap();
            player
                .play(node)
                .set_seek_time(0.5)
                .set_speed(1.5)
                .set_repeat(RepeatAnimation::Forever)
                .set_weight(0.75)
                .pause();
        }
        {
            let mut graphs = app.world_mut().resource_mut::<Assets<AnimationGraph>>();
            graphs
                .get_mut(&graph)
                .unwrap()
                .graph
                .node_weight_mut(node)
                .unwrap()
                .mask = 0b11;
        }

        app.world_mut()
            .run_system_once(super::snapshot_vrma_playback)
            .unwrap();

        let state = app.world().get::<VrmaPlaybackState>(vrma).unwrap();
        assert!(state.active, "an active animation is recorded as active");
        assert!(state.paused);
        assert!((state.seek_time - 0.5).abs() < 1e-4);
        assert_eq!(state.speed, 1.5);
        assert_eq!(state.repeat, RepeatAnimation::Forever);
        assert!((state.weight - 0.75).abs() < 1e-4);
        assert_eq!(state.mask, 0b11);
        success!()
    }
}
