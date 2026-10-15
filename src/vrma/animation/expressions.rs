//! Retargeting a `.vrma`'s expression tracks onto the load-time expression shape.
//!
//! # What a `.vrma` says
//!
//! `VRMC_vrm_animation.expressions` maps a glTF **node** to each expression the
//! clip animates, split into `preset` and `custom`. The animation of such a node
//! is a plain glTF `translation` channel, and the spec fixes the encoding: **the
//! X component of the translation is the expression's weight**, and a value
//! outside `[0, 1]` must be clamped into it.
//!
//! So a `.vrma` "expression track" is a `Vec3` curve whose `y` and `z` are
//! unused, which is exactly why it cannot be played as-is. Retargeting it means
//! turning one `Transform::translation` curve into one `f32` curve bound to
//! `ExpressionWeightProperty`, which writes the avatar root's
//! `VrmExpressionWeights`. `apply_expression_morph_binds`, scheduled by
//! `VrmGltfPlugin`, then distributes those weights into the bound `MorphWeights`
//! — there is no per-expression entity and no separate runtime expression pass.
//!
//! # Where the clamp of `[0, 1]` happens
//!
//! The spec requires an out-of-range weight to be clamped into `[0, 1]`, and
//! `apply_expression_morph_binds` does exactly that for every weight it reads,
//! animated or triggered (`expression_output_weight`). The keyframes written here
//! are therefore left verbatim: clamping them would introduce a kink at the
//! crossing, and a resampled kink is smeared across the surrounding samples —
//! which is precisely where an `isBinary` expression's `0.5` threshold would then
//! be decided on a value the file never wrote.
//!
//! # Where the curves come from
//!
//! [`retarget_expression_curves`] runs per `.vrma` instance, inside the same
//! `RequestUpdateAnimationClips` step that moves the humanoid bone curves onto
//! the avatar (`crate::vrma::animation::animation_graph`). Every destination
//! curve it writes therefore ends up in the *same* `AnimationClip` asset, which
//! is what `AnimationGraph::from_clips` turns into a single clip node and
//! `PlayVrma` plays with one `RepeatAnimation` — expression and bone playback
//! cannot drift apart, and there is no second player to keep in sync.
//!
//! # Per-avatar isolation
//!
//! Three independent things make two avatars playing the same `.vrma` file keep
//! separate faces:
//!
//! * the destination target is [`vrm_root_animation_target`], a *component* on
//!   each avatar root (`handler::scene::setup_animation`), so bevy resolves it
//!   per instance instead of sharing one evaluated value;
//! * what lands there is keyed by `ExpressionWeightProperty::new(name)`'s
//!   interned name, and the destination is each avatar's own
//!   `VrmExpressionWeights` map;
//! * the graph, the player and the per-instance cloned clip all live on the
//!   avatar, so one avatar stopping its clip cannot freeze another's face.
//!
//! The one thing that *is* shared is the curve's immutable keyframe data, which
//! is exactly what a shared `AnimationClip` asset is for.
//!
//! # Precedence against user triggers
//!
//! `SetExpressions` / `ModifyExpressions` write `VrmExpressionWeights` from
//! wherever the caller triggers them — in practice `Update`. bevy's
//! `animate_targets` runs later, in `PostUpdate`, and writes the same map. The
//! animation therefore wins **while its clip is playing**, and the bind pass
//! (`add_expression_bind_pass`, ordered `.after(AnimationSystems)`) sees the
//! animated value. A trigger is still how a user overrides the face: name only
//! the expressions the clip does not drive, or stop the clip — see
//! [`reset_expression_weights`].

use crate::prelude::ChildSearcher;
use crate::vrma::animation::bake::SAMPLE_RATE;
use crate::vrma::animation::properties::{
    ExpressionWeightProperty, VrmExpressionWeights, vrm_root_animation_target,
};
use crate::vrma::gltf::extensions::VrmaExtensions;
use bevy::animation::animation_curves::{AnimationCurve, EvaluatorId};
use bevy::animation::{AnimationClip, AnimationTargetId, VariableCurve, animated_field};
use bevy::asset::{Assets, Handle};
use bevy::gltf::GltfNode;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::platform::hash::Hashed;
use bevy::prelude::*;
use core::any::TypeId;

/// The pre-hashed `(TypeId, field index)` bevy uses to name an animated field.
type FieldId = Hashed<(TypeId, usize)>;

/// The destination curve of one expression: a weight keyframe curve addressed by
/// expression name.
type ExpressionWeightCurve =
    AnimatableCurve<ExpressionWeightProperty, AnimatableKeyframeCurve<f32>>;

/// The expressions a `.vrma` animates, as `(expression name, glTF node name)`.
///
/// This is the source half of the expression retarget, and the mirror image of
/// `HumanoidBoneRegistry`: a `.vrma` carries node *references*, not names, so the
/// source nodes of its own scene are looked up by the name
/// `VRMC_vrm_animation.expressions` resolves to. Nothing about the *destination*
/// avatar's expressions is stored here — the load-time pipeline resolved those
/// by glTF node index while the `.vrm` loaded and wrote them into the scene
/// asset.
#[derive(Component, Deref, Reflect, Default)]
pub(crate) struct VrmaExpressionRegistry(HashMap<String, Name>);

impl VrmaExpressionRegistry {
    /// Resolves the node names `VRMC_vrm_animation.expressions` refers to,
    /// dropping every entry whose node is missing from the file or not yet
    /// loaded.
    ///
    /// Preset and custom expressions are merged into one map: the destination is
    /// keyed by expression name only, and the spec forbids a custom expression
    /// from reusing a preset name, so the two families cannot collide.
    pub fn new(
        extensions: &VrmaExtensions,
        node_assets: &Assets<GltfNode>,
        nodes: &[Handle<GltfNode>],
    ) -> Self {
        let Some(expressions) = extensions.vrmc_vrm_animation.expressions.as_ref() else {
            return Self::default();
        };
        Self(
            expressions
                .preset
                .iter()
                .chain(expressions.custom.iter())
                .filter_map(|(expression, target)| {
                    let node_handle = nodes.get(target.node)?;
                    let node = node_assets.get(node_handle)?;
                    Some((expression.clone(), Name::new(node.name.clone())))
                })
                .collect(),
        )
    }
}

/// Rewrites `clip`'s expression tracks so they address the avatar root.
///
/// For every expression of `registry`, the source node's `translation` curves
/// are consumed and re-emitted as one `ExpressionWeightProperty` curve each under
/// [`vrm_root_animation_target`]. A registered expression with no curves in the
/// clip contributes nothing, and the avatar's own `VrmExpressionWeights` value
/// stands.
///
/// # Why the curves are resampled rather than copied
///
/// `AnimatableCurve` erases its curve type behind the `AnimationCurve` trait,
/// which is not `Downcast` and exposes only `sample_clamped`. There is
/// therefore no way to read a source curve's keyframes, let alone the `x`
/// component of each. Sampling through the same `Curve::sample_clamped` that
/// `AnimatableCurve::apply` itself calls is exact *per sample* — it is the value
/// the evaluator would have written at that instant — and [`SAMPLE_RATE`], the
/// rate the humanoid bone retarget already resamples at, is a five-fold
/// oversample of the 24 Hz the shipped `.vrma` files use. Within one linear
/// segment of the source the reproduction is exact; only across a segment change
/// does the resampled curve round the corner, by an amount that shrinks with the
/// sample spacing.
pub(crate) fn retarget_expression_curves(
    clip: &mut AnimationClip,
    vrma: Entity,
    registry: &VrmaExpressionRegistry,
    searcher: &ChildSearcher,
    source_targets: &Query<&AnimationTargetId>,
) {
    let translation = translation_field_id();
    let root = vrm_root_animation_target();

    // Built up front: `curves()` borrows the clip immutably, so nothing is
    // written back until every source has been read.
    let mut weights: Vec<ExpressionWeightCurve> = Vec::new();
    let mut consumed: HashSet<AnimationTargetId> = HashSet::default();
    for (expression, node_name) in registry.iter() {
        let Some(node) = searcher.find_from_name(vrma, node_name.as_str()) else {
            #[cfg(feature = "log")]
            warn!(
                "[VRMA] no source node named '{}' for expression '{expression}'; \
                 its track is dropped",
                node_name.as_str(),
            );
            continue;
        };
        let Ok(target) = source_targets.get(node) else {
            continue;
        };
        let Some(curves) = clip.curves().get(target) else {
            continue;
        };
        // A source curve that produces no weight (a degenerate domain, say) is
        // left where it is rather than consumed: dropping it would lose a track
        // the retarget could not reproduce.
        let mut taken = false;
        for curve in curves {
            if !animates_field(curve, translation) {
                continue;
            }
            if let Some(weight) = expression_weight_curve(expression, curve) {
                weights.push(weight);
                taken = true;
            }
        }
        if taken {
            consumed.insert(*target);
        }
    }
    if weights.is_empty() {
        return;
    }

    // The source nodes are bevy_gltf's, and nothing plays them, so the consumed
    // curves are dropped rather than moved. Only the translation curves are
    // taken, so a node that also carried another animated property keeps it.
    for target in consumed {
        if let Some(curves) = clip.curves_mut().get_mut(&target) {
            curves.retain(|curve| !animates_field(curve, translation));
            if curves.is_empty() {
                clip.curves_mut().remove(&target);
            }
        }
    }
    for weight in weights {
        clip.add_curve_to_target(root, weight);
    }
}

/// Returns every expression of `vrma` to a neutral face.
///
/// This is the load-time shape's version of what the legacy path did by writing
/// `0.0` into each per-expression entity's `Transform::translation.x`: it zeroes
/// the avatar's weights **for the expressions this `.vrma` animates and nothing
/// else**, so a lip-sync `ModifyExpressions` on an expression the clip does not
/// touch survives playing an unrelated animation.
///
/// Called when a clip starts, so the 300 ms `AnimationTransitions` blend ramps
/// out of a neutral face rather than out of whatever the previous clip left
/// behind, and when it stops, so a finished animation does not freeze the face
/// at its last weight.
pub(crate) fn reset_expression_weights(
    vrma: Entity,
    registries: &Query<&VrmaExpressionRegistry>,
    roots: &mut Query<&mut VrmExpressionWeights>,
    parents: &Query<&ChildOf>,
) {
    let Ok(ChildOf(vrm)) = parents.get(vrma) else {
        return;
    };
    let Ok(registry) = registries.get(vrma) else {
        return;
    };
    let Ok(mut weights) = roots.get_mut(*vrm) else {
        return;
    };
    for expression in registry.keys() {
        weights.0.insert(expression.clone(), 0.0);
    }
}

/// The pre-hashed id of `Transform::translation`, which is what every expression
/// track of a `.vrma` carries.
fn translation_field_id() -> FieldId {
    let field = animated_field!(Transform::translation);
    let EvaluatorId::ComponentField(id) = field.evaluator_id() else {
        unreachable!("`animated_field!` always yields an `EvaluatorId::ComponentField`")
    };
    *id
}

/// Whether `curve` animates the field identified by `id`.
fn animates_field(
    curve: &VariableCurve,
    id: FieldId,
) -> bool {
    matches!(curve.0.evaluator_id(), EvaluatorId::ComponentField(other) if *other == id)
}

/// The `f32` weight curve `expression` is driven by, per `VRMC_vrm_animation`:
/// the source curve's `translation.x`, verbatim.
///
/// `apply_expression_morph_binds` clamps what it reads into `[0, 1]`, so an
/// out-of-range keyframe is corrected where the spec cares — on the value that
/// reaches a morph target — instead of here, where clamping would only bend the
/// curve around the crossing.
fn expression_weight_curve(
    expression: &str,
    curve: &VariableCurve,
) -> Option<ExpressionWeightCurve> {
    let domain = curve.0.domain();
    let (start, end) = (domain.start(), domain.end());
    let duration = end - start;
    // A degenerate domain has no weight to speak of, and a non-finite one would
    // ask for an unbounded keyframe vector.
    if duration <= 0.0 || !duration.is_finite() {
        return None;
    }

    let sample_count = ((duration * SAMPLE_RATE).ceil() as usize).max(2);
    let dt = duration / (sample_count - 1) as f32;
    let mut keyframes = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let t = (start + dt * i as f32).min(end);
        let sample = curve.0.sample_clamped(t).downcast::<Vec3>().ok()?;
        keyframes.push((t, sample.x));
    }

    Some(AnimatableCurve::new(
        ExpressionWeightProperty::new(expression),
        AnimatableKeyframeCurve::new(keyframes).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use bevy::app::AnimationSystems;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::morph::MorphWeights;

    use super::*;
    use crate::prelude::{Initialized, PlayVrma, SetExpressions, StopVrma, VrmSystemSets};
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::Vrm;
    use crate::vrm::expressions::{
        ExpressionCategory, ExpressionOverrideType, VrmExpressionPlugin,
    };
    use crate::vrm::gltf::extensions::VrmNode;
    use crate::vrm::instance::VrmGeneration;
    use crate::vrma::VrmAnimationNodeIndex;
    use crate::vrma::animation::animation_graph::{
        RequestUpdateAnimationGraph, VrmaAnimationGraphPlugin,
    };
    use crate::vrma::animation::mask::VrmaMask;
    use crate::vrma::animation::play::VrmaAnimationPlayPlugin;
    use crate::vrma::animation::properties::{
        ExpressionMorphBinds, ExpressionSetting, ExpressionSettings, MorphBind, MorphBindTable,
        apply_expression_morph_binds,
    };
    use crate::vrma::gltf::extensions::{VRMCVrmAnimation, VrmaExpressions, VrmaHumanoid};
    use crate::vrma::{VrmAnimationClipHandle, VrmAnimationSourceClip, Vrma};
    use bevy::animation::{AnimatedBy, AnimationPlugin};

    /// The `y` and `z` every fixture track writes. The shipped `.vrma` files
    /// park the rest of the node's rest translation there, so a retarget that
    /// read the wrong component would pass a fixture written to match the bug.
    const SCRATCH_Y: f32 = 0.125;
    const SCRATCH_Z: f32 = 0.375;

    /// The pipeline shape, end to end for one expression: the `.vrma`
    /// destination curve writes `VrmExpressionWeights`, and the bind pass moves
    /// it into the mesh's morph weights.
    ///
    /// This is the case `apply_regenerate_expression_clips` used to cover for the
    /// per-expression-entity shape: the clip's expression curves reach the
    /// avatar's morph targets without any intermediate entity.
    #[test]
    fn a_retargeted_expression_weight_reaches_the_morph_target() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();
        let root = app
            .world_mut()
            .spawn((
                VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.0)])),
                ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                    "happy".to_owned(),
                    vec![MorphBind {
                        target: mesh,
                        index: 1,
                        weight: 1.0,
                    }],
                )]))),
                ExpressionSettings(HashMap::from([(
                    "happy".to_owned(),
                    ExpressionSetting {
                        is_binary: false,
                        category: ExpressionCategory::Other,
                        override_blink: ExpressionOverrideType::None,
                        override_look_at: ExpressionOverrideType::None,
                        override_mouth: ExpressionOverrideType::None,
                    },
                )])),
            ))
            .id();

        // The retarget's weight, as an animation curve or an
        // `ExpressionWeightProperty` writes it.
        app.world_mut()
            .entity_mut(root)
            .get_mut::<VrmExpressionWeights>()
            .expect("the root carries the weights")
            .0
            .insert("happy".to_owned(), 0.6);
        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .expect("the bind pass runs");

        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights()[1],
            0.6
        );

        // A user trigger on the same root wins over the retargeted weight, which
        // is what makes an expression overridable at runtime.
        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(root, "happy", 0.25));
        app.update();
        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .expect("the bind pass runs");

        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights()[1],
            0.25
        );
        success!()
    }

    /// A full-fidelity app for the retarget: bevy's own animation plugin, so the
    /// graph this crate builds is evaluated, plus the bind pass on the edges
    /// production uses.
    ///
    /// `add_expression_bind_pass` in `src/vrm/gltf.rs` is private to that module,
    /// so the two edges it declares are spelled out here — the point of this app
    /// is that they match.
    fn setup_app(with_animation: bool) -> App {
        let mut app = test_app();
        if with_animation {
            // bevy's own plugin, so the graph this crate builds is evaluated.
            app.add_plugins(AnimationPlugin);
        } else {
            // Without that plugin there is no `Assets<AnimationClip>` /
            // `Assets<AnimationGraph>` store, and the graph-build observer needs
            // both.
            app.init_asset::<AnimationClip>()
                .init_asset::<AnimationGraph>();
        }
        app.add_plugins((VrmaAnimationGraphPlugin, VrmaAnimationPlayPlugin));
        app.add_systems(
            PostUpdate,
            apply_expression_morph_binds
                .in_set(VrmSystemSets::Expressions)
                .after(AnimationSystems)
                .after(VrmSystemSets::GazeControl),
        );
        app
    }

    /// The destination avatar exactly as `handler::scene::setup_animation`
    /// writes it: the scene root carries the synthetic target and the live
    /// weights, and the root bone — the animation root — carries the player.
    fn spawn_avatar(world: &mut World) -> (Entity, Entity) {
        let vrm = world
            .spawn((Vrm, Initialized, VrmExpressionWeights::default()))
            .id();
        let root_bone = world
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                Transform::default(),
                AnimationPlayer::default(),
                AnimationTransitions::default(),
                ChildOf(vrm),
            ))
            .id();
        world
            .entity_mut(vrm)
            .insert((vrm_root_animation_target(), AnimatedBy(root_bone)));
        (vrm, root_bone)
    }

    /// A `.vrma` child holding one expression track on the source node named
    /// `source_node`: the spec's shape, a `translation` curve whose `x` is the
    /// weight. `y`/`z` are deliberately non-zero, because the shipped files use
    /// them as scratch and a retarget that read the wrong component would look
    /// fine against a test fixture written to match the bug.
    ///
    /// The source node hangs off the `.vrma` entity, where `bevy_gltf` instantiates
    /// the file's own scene — and where the retarget looks for it, scoped to the
    /// `.vrma` so two instances of one file cannot resolve each other's nodes.
    fn spawn_expression_vrma(
        world: &mut World,
        vrm: Entity,
        expression: &str,
        values: &[(f32, f32)],
    ) -> TestResult<(Entity, Handle<AnimationClip>)> {
        let source_target = source_target_id();
        let keyframes = values
            .iter()
            .map(|(time, weight)| (*time, Vec3::new(*weight, SCRATCH_Y, SCRATCH_Z)))
            .collect::<Vec<_>>();
        let mut clip = AnimationClip::default();
        clip.add_curve_to_target(
            source_target,
            AnimatableCurve::new(
                animated_field!(Transform::translation),
                AnimatableKeyframeCurve::new(keyframes)?,
            ),
        );
        let handle = world.resource_mut::<Assets<AnimationClip>>().add(clip);
        let vrma = world
            .spawn((
                Vrma,
                Initialized,
                VrmAnimationClipHandle(handle.clone()),
                registry_of(&[(expression, "source_node")]),
                ChildOf(vrm),
            ))
            .id();
        world.spawn((Name::new("source_node"), source_target, ChildOf(vrma)));
        Ok((vrma, handle))
    }

    fn registry_of(entries: &[(&str, &str)]) -> VrmaExpressionRegistry {
        VrmaExpressionRegistry(
            entries
                .iter()
                .map(|(expression, node)| ((*expression).to_owned(), Name::new((*node).to_owned())))
                .collect(),
        )
    }

    fn weight_of(
        app: &App,
        vrm: Entity,
        expression: &str,
    ) -> f32 {
        app.world()
            .get::<VrmExpressionWeights>(vrm)
            .expect("the avatar root carries the expression weights")
            .0
            .get(expression)
            .copied()
            .unwrap_or(0.0)
    }

    /// Overwrites individual weights on the avatar root, the way a user trigger
    /// (or a previous clip) would leave them.
    fn set_weights(
        world: &mut World,
        vrm: Entity,
        weights: &[(&str, f32)],
    ) {
        let mut root = world.entity_mut(vrm);
        let mut component = root
            .get_mut::<VrmExpressionWeights>()
            .expect("the avatar root carries the expression weights");
        for (name, weight) in weights {
            component.0.insert((*name).to_owned(), *weight);
        }
    }

    fn request_graph(
        app: &mut App,
        vrm: Entity,
    ) {
        app.world_mut().trigger(RequestUpdateAnimationGraph { vrm });
        app.world_mut().flush();
        app.update();
    }

    /// Three frames: the asset event a tracked `Assets::get_mut` queues is
    /// drained at the end of one `PostUpdate` and turned into threaded masks in
    /// the next, and `animate_targets` is unordered against that rebuild, so the
    /// third is the first whose evaluation is guaranteed to see the new graph.
    fn run_frames(app: &mut App) {
        for _ in 0..3 {
            app.update();
        }
    }

    /// The clip the retarget produced: the weight curves are on the root target
    /// and nothing is left on the source node.
    fn retargeted_curves(
        app: &App,
        clip: &Handle<AnimationClip>,
    ) -> Vec<VariableCurve> {
        let curves = app
            .world()
            .resource::<Assets<AnimationClip>>()
            .get(clip)
            .expect("the clip asset exists")
            .curves();
        assert!(
            !curves.keys().any(|target| *target == source_target_id()),
            "the source node's track must not be left on the source node"
        );
        curves
            .get(&vrm_root_animation_target())
            .expect("the expression track must address the avatar root")
            .clone()
    }

    fn source_target_id() -> AnimationTargetId {
        AnimationTargetId::from_name(&Name::new("source_node"))
    }

    /// Samples a retargeted weight curve. An `ExpressionWeightProperty` curve is
    /// valued in `f32`, so the downcast is the assertion that the retarget used
    /// that property rather than some other.
    fn sample_weight(
        curve: &VariableCurve,
        time: f32,
    ) -> f32 {
        *curve
            .0
            .sample_clamped(time)
            .downcast::<f32>()
            .expect("an `ExpressionWeightProperty` curve is valued in f32")
    }

    /// The clip's expression track becomes one `ExpressionWeightProperty` curve
    /// on the **root** target, and the source node keeps nothing. The humanoid
    /// bone retarget is the same kind of move, so both land in one clip asset —
    /// which is what keeps one clip node playing both.
    #[test]
    fn an_expression_track_is_retargeted_onto_the_root_target() -> TestResult {
        let mut app = setup_app(false);
        let (vrm, _) = spawn_avatar(app.world_mut());
        let (vrma, clip) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 0.0), (1.0, 1.0)])?;

        request_graph(&mut app, vrm);

        let curves = retargeted_curves(&app, &clip);
        assert_eq!(curves.len(), 1);
        assert_eq!(
            curves[0].0.domain().start(),
            0.0,
            "the weight curve keeps the source track's domain"
        );
        assert!(
            app.world().get::<VrmAnimationNodeIndex>(vrma).is_some(),
            "the clip is still a graph node, so `PlayVrma` plays the expression too"
        );
        success!()
    }

    /// The required end-to-end capability: a `.vrma` expression curve retargets
    /// onto `ExpressionWeightProperty` for the root target, and when bevy
    /// evaluates it, `VrmExpressionWeights` moves — and then `MorphWeights`,
    /// through `apply_expression_morph_binds`, in the *same* frame.
    #[test]
    fn evaluating_a_retargeted_track_moves_the_weights_and_the_morph() -> TestResult {
        let mut app = setup_app(true);
        let (vrm, root_bone) = spawn_avatar(app.world_mut());
        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0], None)?)
            .id();
        bind_expression(&mut app, vrm, mesh);
        let (vrma, _) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 1.0), (1.0, 1.0)])?;
        request_graph(&mut app, vrm);

        // The clip holds a constant 1.0 weight from t=0, so one evaluation frame
        // is enough to see it.
        play(&mut app, root_bone, vrma);
        run_frames(&mut app);

        assert_eq!(weight_of(&app, vrm, "happy"), 1.0);
        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights()[0],
            1.0,
            "the bind pass must distribute the animated weight in the same frame"
        );
        success!()
    }

    /// Two avatars retargeted from one `.vrma` file keep independent expression
    /// weights — the capability upstream `606fe9c` was after, and the reason the
    /// destination is a per-instance `AnimationTargetId` rather than one shared
    /// evaluated value.
    ///
    /// Each avatar holds its own clone of the clip and its own graph, so the two
    /// are driven from the same *asset* but write to different roots.
    #[test]
    fn two_avatars_retargeted_from_one_clip_keep_independent_weights() -> TestResult {
        let mut app = setup_app(true);
        let (a, root_a) = spawn_avatar(app.world_mut());
        let (b, root_b) = spawn_avatar(app.world_mut());
        // The same `.vrma` file loaded twice: the same expression, the same
        // source node name, two instances.
        let (vrma_a, _) =
            spawn_expression_vrma(app.world_mut(), a, "happy", &[(0.0, 1.0), (1.0, 1.0)])?;
        let (vrma_b, _) =
            spawn_expression_vrma(app.world_mut(), b, "happy", &[(0.0, 0.25), (1.0, 0.25)])?;
        request_graph(&mut app, a);
        request_graph(&mut app, b);

        play(&mut app, root_a, vrma_a);
        play(&mut app, root_b, vrma_b);
        run_frames(&mut app);

        assert_eq!(weight_of(&app, a, "happy"), 1.0);
        assert_eq!(weight_of(&app, b, "happy"), 0.25);

        // Stopping one avatar's clip resets only that avatar's face, and must not
        // touch the other's: the destination is per-instance, not per-file.
        app.world_mut()
            .commands()
            .trigger(StopVrma { entity: vrma_a });
        app.update();
        assert_eq!(
            weight_of(&app, a, "happy"),
            0.0,
            "a stopped clip leaves a neutral face, not a frozen one"
        );
        assert_eq!(
            weight_of(&app, b, "happy"),
            0.25,
            "one avatar's clip stopping must not freeze another's face"
        );
        success!()
    }

    /// The spec representation, on the shape the retarget produces: the weight is
    /// the source curve's `translation.x` — never `y` or `z` — reproduced
    /// faithfully, so a shipped track's shape survives the retarget.
    #[test]
    fn the_weight_is_the_source_translation_x() -> TestResult {
        let mut app = setup_app(false);
        let (vrm, _) = spawn_avatar(app.world_mut());
        let (_, clip) = spawn_expression_vrma(
            app.world_mut(),
            vrm,
            "happy",
            &[(0.0, 0.25), (0.5, 0.5), (1.0, 0.75)],
        )?;
        request_graph(&mut app, vrm);

        let curves = retargeted_curves(&app, &clip);
        let weight = &curves[0];
        for (time, expected) in [(0.0, 0.25), (0.25, 0.375), (0.5, 0.5), (1.0, 0.75)] {
            let actual = sample_weight(weight, time);
            assert!(
                (actual - expected).abs() < 1e-3,
                "at t={time} the weight must be {expected}, got {actual}"
            );
        }
        for scratch in [SCRATCH_Y, SCRATCH_Z] {
            assert!(
                (sample_weight(weight, 0.5) - scratch).abs() > 0.1,
                "the weight is translation.x, never y ({SCRATCH_Y}) or z ({SCRATCH_Z})"
            );
        }
        success!()
    }

    /// A rig replacement re-clones the pristine source before retargeting, so the
    /// expression is rebuilt and not lost. The retarget *moves* the source track
    /// onto the root target, so re-running it on the working clip would find no
    /// source curve and drop the face; the pristine `VrmAnimationSourceClip` is
    /// what makes the second generation's retarget possible.
    #[test]
    fn a_reloaded_expression_is_re_retargeted_from_the_pristine_source() -> TestResult {
        let mut app = setup_app(false);
        let (vrm, _) = spawn_avatar(app.world_mut());
        app.world_mut().entity_mut(vrm).insert(VrmGeneration(1));
        let (vrma, working) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 0.25), (1.0, 0.75)])?;
        // The pristine copy the reload must clone from, taken before the first
        // retarget moves the working clip's curves off the source node.
        let source = {
            let pristine = app
                .world()
                .resource::<Assets<AnimationClip>>()
                .get(&working)
                .expect("the working clip exists")
                .clone();
            app.world_mut()
                .resource_mut::<Assets<AnimationClip>>()
                .add(pristine)
        };
        app.world_mut()
            .entity_mut(vrma)
            .insert(VrmAnimationSourceClip(source));

        request_graph(&mut app, vrm);
        assert!(
            (sample_weight(&retargeted_curves(&app, &working)[0], 0.5) - 0.5).abs() < 1e-3,
            "the first retarget writes the weight onto the root target"
        );

        app.world_mut().entity_mut(vrm).insert(VrmGeneration(2));
        request_graph(&mut app, vrm);

        let reloaded = app
            .world()
            .get::<VrmAnimationClipHandle>(vrma)
            .expect("the reloaded clip handle")
            .0
            .clone();
        assert_ne!(
            reloaded.id(),
            working.id(),
            "a new generation must re-clone the working clip from the source"
        );
        let curves = retargeted_curves(&app, &reloaded);
        assert_eq!(
            curves.len(),
            1,
            "the re-retargeted clip has the weight curve"
        );
        assert!((sample_weight(&curves[0], 0.25) - 0.375).abs() < 1e-3);
        assert!((sample_weight(&curves[0], 1.0) - 0.75).abs() < 1e-3);
        success!()
    }

    /// The spec's clamp: an out-of-range weight is clamped into `[0, 1]` on the
    /// value that reaches a morph target. The keyframes keep the file's numbers
    /// (clamping them would bend the curve, see the module docs), so this is
    /// where a file that writes `4.0` becomes `1.0`.
    #[test]
    fn an_out_of_range_animated_weight_is_clamped_when_it_is_distributed() -> TestResult {
        assert_eq!(
            distributed_weight(4.0)?,
            1.0,
            "4.0 must reach the mesh as 1.0"
        );
        assert_eq!(
            distributed_weight(-4.0)?,
            0.0,
            "-4.0 must reach the mesh as 0.0"
        );
        success!()
    }

    /// Plays one clip whose expression track is a constant `raw` weight, and
    /// reports what landed on the mesh's single morph slot.
    fn distributed_weight(raw: f32) -> TestResult<f32> {
        let mut app = setup_app(true);
        let (vrm, root_bone) = spawn_avatar(app.world_mut());
        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0], None)?)
            .id();
        bind_expression(&mut app, vrm, mesh);
        let (vrma, _) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, raw), (1.0, raw)])?;
        request_graph(&mut app, vrm);
        play(&mut app, root_bone, vrma);
        run_frames(&mut app);
        Ok(app.world().get::<MorphWeights>(mesh).unwrap().weights()[0])
    }

    /// The three components the bind pass reads, as `handler::scene`'s
    /// `build_expressions` writes them: one morph slot per declared expression,
    /// each bound 1:1 at its own index.
    fn bind_expressions(
        app: &mut App,
        vrm: Entity,
        mesh: Entity,
        expressions: &[(&str, ExpressionCategory)],
    ) {
        app.world_mut().entity_mut(vrm).insert((
            VrmExpressionWeights(
                expressions
                    .iter()
                    .map(|(name, _)| ((*name).to_owned(), 0.0))
                    .collect(),
            ),
            ExpressionMorphBinds(MorphBindTable(HashMap::from_iter(
                expressions.iter().enumerate().map(|(index, (name, _))| {
                    (
                        (*name).to_owned(),
                        vec![MorphBind {
                            target: mesh,
                            index,
                            weight: 1.0,
                        }],
                    )
                }),
            ))),
            ExpressionSettings(HashMap::from_iter(expressions.iter().map(
                |(name, category)| {
                    (
                        (*name).to_owned(),
                        ExpressionSetting {
                            is_binary: false,
                            category: *category,
                            override_blink: ExpressionOverrideType::None,
                            override_look_at: ExpressionOverrideType::None,
                            override_mouth: ExpressionOverrideType::None,
                        },
                    )
                },
            ))),
        ));
    }

    /// [`bind_expressions`] for the single-expression case these tests use.
    fn bind_expression(
        app: &mut App,
        vrm: Entity,
        mesh: Entity,
    ) {
        bind_expressions(app, vrm, mesh, &[("happy", ExpressionCategory::Other)]);
    }

    /// `SetExpressions` writes the same map the animation writes. The animation
    /// is evaluated in `PostUpdate` and the bind pass is ordered after it, so a
    /// trigger in `Update` loses to a playing clip — which is the composition
    /// rule: the animation owns an expression it is animating, and the user owns
    /// the rest.
    #[test]
    fn a_playing_clip_overrides_a_user_trigger_on_the_same_expression() -> TestResult {
        let mut app = setup_app(true);
        app.add_plugins(VrmExpressionPlugin);
        let (vrm, root_bone) = spawn_avatar(app.world_mut());
        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();
        // `happy` is the clip's, `aa` is only the user's.
        bind_expressions(
            &mut app,
            vrm,
            mesh,
            &[
                ("happy", ExpressionCategory::Other),
                ("aa", ExpressionCategory::Mouth),
            ],
        );
        let (vrma, _) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 1.0), (1.0, 1.0)])?;
        request_graph(&mut app, vrm);
        play(&mut app, root_bone, vrma);
        run_frames(&mut app);
        assert_eq!(weight_of(&app, vrm, "happy"), 1.0);

        // `SetExpressions` replaces the whole map, so `aa` leaves the map
        // entirely and `happy` is pinned to 0.1 for the frame.
        app.world_mut()
            .commands()
            .trigger(SetExpressions::from_iter(
                vrm,
                [("happy", 0.1), ("aa", 0.3)],
            ));
        app.update();

        assert_eq!(
            weight_of(&app, vrm, "happy"),
            1.0,
            "the animation is evaluated after the trigger, so it wins"
        );
        assert_eq!(
            weight_of(&app, vrm, "aa"),
            0.3,
            "an expression the clip does not drive stays the user's"
        );
        assert_eq!(
            app.world().get::<MorphWeights>(mesh).unwrap().weights(),
            &[1.0, 0.3],
            "the bind pass sees the animated weight and the user's, in one pass"
        );
        success!()
    }

    /// `PlayVrma` returns the clip's own expressions to a neutral face *before* the
    /// 300 ms transition starts, so the blend ramps out of neutral rather than out
    /// of whatever the previous clip left behind — and it touches nothing the
    /// clip does not drive, so a lip-sync override on another expression survives.
    #[test]
    fn playing_resets_only_the_clips_own_expressions() -> TestResult {
        let mut app = setup_app(true);
        let (vrm, _) = spawn_avatar(app.world_mut());
        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();
        bind_expressions(
            &mut app,
            vrm,
            mesh,
            &[
                ("happy", ExpressionCategory::Other),
                ("aa", ExpressionCategory::Mouth),
            ],
        );
        let (vrma, _) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 1.0), (1.0, 1.0)])?;
        request_graph(&mut app, vrm);

        // A previous clip left `happy` at 0.9, and the user is lip-syncing `aa`.
        set_weights(app.world_mut(), vrm, &[("happy", 0.9), ("aa", 0.3)]);

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(PlayVrma::new);
        // Read the reset before the next `animate_targets`: from that point on
        // the clip is writing the very weight `PlayVrma` just zeroed, which is
        // the precedence rule (`a_playing_clip_overrides_a_user_trigger_on_the_same_expression`).
        app.world_mut().flush();

        assert_eq!(
            weight_of(&app, vrm, "happy"),
            0.0,
            "the clip's own expression starts from neutral"
        );
        assert_eq!(
            weight_of(&app, vrm, "aa"),
            0.3,
            "an expression the clip does not drive is the user's to keep"
        );
        success!()
    }

    /// An expression is not a body part: the root target stays in no mask group,
    /// and bevy reads an unregistered target's mask as `0` — "nothing is masked
    /// out" (`bevy_animation/src/lib.rs:1123-1128`, `:1194-1201`). A layer masked
    /// to the upper body therefore still animates the face.
    #[test]
    fn a_body_mask_does_not_silence_an_expression() -> TestResult {
        let mut app = setup_app(true);
        let (vrm, root_bone) = spawn_avatar(app.world_mut());
        let (vrma, _) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 1.0), (1.0, 1.0)])?;
        request_graph(&mut app, vrm);

        let graph = app
            .world()
            .get::<AnimationGraphHandle>(root_bone)
            .expect("the graph handle is on the root bone")
            .clone();
        assert!(
            !app.world()
                .resource::<Assets<AnimationGraph>>()
                .get(&graph)
                .unwrap()
                .mask_groups
                .contains_key(&vrm_root_animation_target()),
            "an expression is not a body part"
        );

        let node = app
            .world()
            .get::<VrmAnimationNodeIndex>(vrma)
            .expect("the clip is a graph node")
            .0;
        {
            let mut graphs = app.world_mut().resource_mut::<Assets<AnimationGraph>>();
            graphs
                .get_mut(&graph)
                .unwrap()
                .graph
                .node_weight_mut(node)
                .unwrap()
                .mask = VrmaMask::UPPER_BODY;
        }
        play(&mut app, root_bone, vrma);
        run_frames(&mut app);

        assert_eq!(
            weight_of(&app, vrm, "happy"),
            1.0,
            "a layer masked to part of the body must not silence the face"
        );
        success!()
    }

    /// `reset_expression_weights` zeroes exactly the expressions the clip
    /// declares, so an unrelated user override is not clobbered — and it gives a
    /// stopped clip a neutral face instead of a frozen one.
    #[test]
    fn resetting_touches_only_the_clips_own_expressions() -> TestResult {
        let mut app = test_app();
        let vrm = app
            .world_mut()
            .spawn(VrmExpressionWeights(HashMap::from([
                ("happy".to_owned(), 0.9),
                ("aa".to_owned(), 0.4),
            ])))
            .id();
        let vrma = app
            .world_mut()
            .spawn((registry_of(&[("happy", "source_node")]), ChildOf(vrm)))
            .id();

        app.world_mut()
            .run_system_once(
                move |registries: Query<&VrmaExpressionRegistry>,
                      mut roots: Query<&mut VrmExpressionWeights>,
                      parents: Query<&ChildOf>| {
                    reset_expression_weights(vrma, &registries, &mut roots, &parents);
                },
            )
            .expect("the reset runs");

        let weights = &app.world().get::<VrmExpressionWeights>(vrm).unwrap().0;
        assert_eq!(weights["happy"], 0.0, "the clip's own expression is reset");
        assert_eq!(
            weights["aa"], 0.4,
            "an expression the clip does not drive is untouched"
        );
        success!()
    }

    /// The registry merges `preset` and `custom`, which the shipped files leave
    /// empty but the spec defines, and a `.vrma` animating a custom expression
    /// reaches the avatar like any other.
    #[test]
    fn the_registry_merges_preset_and_custom_expressions() -> TestResult {
        let mut node_assets = Assets::<GltfNode>::default();
        let happy = node_assets.add(gltf_node(0, "node_happy"));
        let custom = node_assets.add(gltf_node(1, "node_custom"));
        let extensions = extensions_of(&[("happy", 0, false), ("smile", 1, true)]);

        let registry = VrmaExpressionRegistry::new(
            &extensions,
            &node_assets,
            &[happy.clone(), custom.clone()],
        );
        assert_eq!(registry["happy"], Name::new("node_happy"));
        assert_eq!(registry["smile"], Name::new("node_custom"));
        assert_eq!(registry.len(), 2);

        // A node index the file does not have is dropped, not defaulted.
        let partial =
            VrmaExpressionRegistry::new(&extensions, &node_assets, std::slice::from_ref(&happy));
        assert_eq!(partial.len(), 1);
        assert!(!partial.contains_key("smile"));

        // A file with no `expressions` at all retargets nothing.
        let none = extensions_of(&[]);
        assert!(VrmaExpressionRegistry::new(&none, &node_assets, &[happy, custom]).is_empty());
        success!()
    }

    fn gltf_node(
        index: usize,
        name: &str,
    ) -> GltfNode {
        GltfNode {
            index,
            name: name.into(),
            children: vec![],
            mesh: None,
            skin: None,
            transform: Transform::default(),
            is_animation_root: false,
            extras: None,
        }
    }

    /// A `.vrma` extension object with `expressions` built from
    /// `(name, node index, is custom)` triples.
    fn extensions_of(entries: &[(&str, usize, bool)]) -> VrmaExtensions {
        let (preset, custom) = entries.iter().fold(
            (HashMap::new(), HashMap::new()),
            |(mut preset, mut custom), (name, node, is_custom)| {
                let target = VrmNode { node: *node };
                if *is_custom {
                    custom.insert((*name).to_owned(), target);
                } else {
                    preset.insert((*name).to_owned(), target);
                }
                (preset, custom)
            },
        );
        VrmaExtensions {
            vrmc_vrm_animation: VRMCVrmAnimation {
                expressions: Some(VrmaExpressions { preset, custom }),
                humanoid: VrmaHumanoid {
                    human_bones: HashMap::new(),
                },
                spec_version: Some("1.0".into()),
            },
        }
    }

    /// Starts a clip without a transition: `AnimationTransitions::play` ramps the
    /// weight from zero over 300 ms, which is not what these tests measure.
    fn play(
        app: &mut App,
        root_bone: Entity,
        vrma: Entity,
    ) {
        let node = app
            .world()
            .get::<VrmAnimationNodeIndex>(vrma)
            .expect("the clip is a graph node")
            .0;
        app.world_mut()
            .entity_mut(root_bone)
            .get_mut::<AnimationPlayer>()
            .expect("the root bone holds the player")
            .play(node);
    }
}
