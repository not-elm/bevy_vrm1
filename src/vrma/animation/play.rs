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
use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle};
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
            // The snapshot runs after the whole frame so it holds the latest
            // live time/speed/weight before the engine can replace the rig. The
            // restore runs after the bake (`VrmaBake`) so the first evaluated
            // frame of a rebuilt rig already uses destination-space curves, and
            // before bevy evaluates the new player.
            .add_systems(Last, snapshot_vrma_playback)
            // Capture edits made in Update before the spawner can replace and
            // despawn the old player in this same frame.
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
    // A clip that still owes a bake or a retarget carries source-space curves;
    // starting it now would evaluate them on the rig. Record the intent and let
    // the restore start it once the retarget has run instead.
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
    // The expression tracks are nodes of the same clip, so the same player and
    // the same `repeat` drive them. What still needs doing is the neutral face
    // the transition blends out of: without it a clip that fades in over 300 ms
    // fades out of whatever the previous one left behind.
    reset_expression_weights(vrma_entity, &expression_registries, &mut roots, &parents);
    if trigger.reset_spring_bones {
        reset_spring_bone_velocities(*vrm_entity, &spring_roots, &mut joint_states, &childrens);
    }
    let played = live.is_some();
    // `active` is authoritative and is what a rig replacement replays. When the
    // clip started live, the exact `ActiveAnimation` is recorded — fading state
    // and all — so the restore writes back what actually plays rather than
    // resetting it to a placeholder. A deferred play (not ready yet, or a rig
    // replaced in the same frame) has no live value and falls back to a starting
    // placeholder.
    if let Ok(mut state) = playback.get_mut(vrma_entity) {
        state.active = true;
        state.animation = Some(live.unwrap_or_else(|| {
            let mut animation = ActiveAnimation::default();
            animation.set_repeat(trigger.repeat);
            animation
        }));
    }
    // The live start is the source of truth for this frame: an already-queued
    // restore must not replay over it and lose the transition the user asked
    // for. Anything that could not start live — a clip that still owes a
    // retarget/bake, or an avatar with no reachable player yet — is replayed by
    // the restore instead.
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

/// Starts `node_index` through the root bone's transitions and returns the live
/// [`ActiveAnimation`] it produced, so the caller can record exactly what plays.
///
/// `None` when the avatar has no reachable root bone with a player and
/// transitions, i.e. there is nothing to start.
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
    let live = transitions.play(&mut player, node_index, transition_duration);
    live.set_repeat(repeat);
    Some(*live)
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
    // replaced must not be lost and then replayed by the restore. Clearing the
    // animation too means a stale live snapshot cannot resurrect it either.
    if let Ok(mut state) = playback.get_mut(vrma_entity) {
        state.active = false;
        state.animation = None;
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

/// Records each VRMA's live playback and the root bone's transitions every frame.
///
/// A reattachment has to restore what the *old* player was doing, and the engine
/// destroys it together with the old rig, so the snapshot must be taken while it
/// still exists. Only an avatar with a live root-bone player is read: while a rig
/// is being replaced there is none, and the last snapshot must survive that gap
/// unchanged.
///
/// The mask is left alone: the graph asset is per avatar and persistent (see
/// [`VrmAnimationGraphState`](crate::vrma::VrmAnimationGraphState)), so a node's
/// mask survives a rebuild by itself and never needs to be snapshotted.
///
/// A node that is no longer in the player means the clip was stopped — a
/// finished fade (`AnimationTransitions` expiry) removes it — and the snapshot
/// records `active = false` so a later reload cannot resurrect it. A clip whose
/// restore has not run yet is skipped: its player is half-built and must not
/// overwrite the state that is about to be replayed.
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
        // No root bone or player means the rig is absent (mid-replace): leave
        // every snapshot exactly as the last live frame left it.
        let Some(root_bone) = searcher.find_root_bone(vrm) else {
            continue;
        };
        let Ok(player) = players.get(root_bone) else {
            continue;
        };
        // Keep the transitions with the graph: a future `PlayVrma` must fade out
        // the restored main clip instead of leaving it stuck at full weight.
        if let Ok(live) = transitions.get(root_bone)
            && let Ok(mut state) = states.get_mut(vrm)
        {
            state.transitions = live.clone();
        }
        for child in children.iter() {
            let Ok((Some(index), Some(mut state), pending_restore)) = vrmas.get_mut(child) else {
                continue;
            };
            // The player is half-built: the restore has not replayed the state
            // yet, so reading the fresh player would clear `active` and lose the
            // clip a frame before it is restored.
            if pending_restore {
                continue;
            }
            match player.animation(index.0) {
                Some(active) => {
                    state.active = true;
                    // `ActiveAnimation` is `Copy`, so this is the whole live
                    // state — every field, including the private progress
                    // counters.
                    state.animation = Some(*active);
                }
                None => {
                    // The node left the player. Bevy removes a faded-out clip
                    // when the transition expires, and `stop` removes it
                    // directly; either way the clip is no longer playing and
                    // must not be restarted by a later reload.
                    state.active = false;
                    state.animation = None;
                }
            }
        }
    }
}

/// Re-applies a recorded playback to a freshly rebuilt player.
///
/// Queued by the graph build for every clip whose player was rebuilt (a new rig,
/// or a node that did not exist before) and left alone on a same-rig rebuild, so
/// a same-frame `PlayVrma`/`StopVrma`/speed change survives. The playback is
/// restarted only for a clip recorded as active; a stopped or faded-out clip
/// stays stopped.
///
/// Runs after the bake (`VrmaBake`), so no evaluated frame uses an unbaked clip,
/// and before bevy evaluates the new player. A clip that still owes a bake
/// (`NeedsBake`) or a retarget (`VrmaRetargetPending`) is left marked, so a
/// source-space clip is never played and the retry can finish it first.
///
/// The recorded [`ActiveAnimation`] is `Copy`, so `*live = snapshot` restores
/// *every* field — including the private `completions`, `elapsed`,
/// `just_completed` and `last_seek_time` no setter can reach — and nothing
/// restarts or resets.
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
            // The new rig is not reachable yet; retry on a later frame.
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

    /// A recycled headless app with a VRM root bone carrying a player and a
    /// VRMA child. Returns `(app, root_bone, vrma, node)`.
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

    /// The snapshot is a verbatim copy of the live [`ActiveAnimation`] and the
    /// restore writes it back whole, so *every* field survives a rebuild —
    /// including the private `completions`, `elapsed`, `just_completed` and
    /// `last_seek_time` that Bevy exposes no setter for. `Debug` is the only
    /// observable that covers the private fields, so the two are compared as
    /// formatted values.
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
        // One second at speed 2.0 crosses the clip's 1.0 s duration once: the
        // live animation now carries a completion, elapsed time and a previous
        // seek time, none of which have setters.
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

        // A brand new root bone with a fresh player is what a rig replacement
        // leaves behind.
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

    /// A clip that left the player — a finished fade or a direct `stop` — is
    /// recorded as inactive, so a later reload does not resurrect it.
    #[test]
    fn the_snapshot_clears_a_clip_that_left_the_player() -> TestResult {
        let (mut app, root_bone, vrma, node) = reload_fixture();
        {
            let mut root_bone_entity = app.world_mut().entity_mut(root_bone);
            let mut player = root_bone_entity.get_mut::<AnimationPlayer>().unwrap();
            player.play(node);
            // `expire_completed_transitions` removes a faded-out node the same
            // way a direct `stop` does.
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

    /// A clip whose restore has not run yet must keep its saved state: the
    /// player is half-built, and reading it would clear the clip a frame before
    /// the restore replays it.
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

    /// The restore waits for the bake: playing a clip that still owes a bake
    /// would evaluate source-space curves on the first frame.
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

    /// A clip whose retarget is still pending must not be played: its working
    /// clip is source-space, and the retry has to finish before the restore.
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

    /// A `StopVrma` that arrives after the rebuild marked the restore but before
    /// the restore ran must win: the clip stays stopped.
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
