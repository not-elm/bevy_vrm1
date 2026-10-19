//!  This module handles the retargeting of expressions from a VRM model to a mascot model.

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

type FieldId = Hashed<(TypeId, usize)>;

type ExpressionWeightCurve =
    AnimatableCurve<ExpressionWeightProperty, AnimatableKeyframeCurve<f32>>;

/// The expressions a `.vrma` animates, as `(expression name, glTF node name)`.
#[derive(Component, Deref, Reflect, Default)]
pub(crate) struct VrmaExpressionRegistry(HashMap<String, Name>);

impl VrmaExpressionRegistry {
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
                .filter_map(|(expression, target)| {
                    let node_handle = nodes.get(target.node)?;
                    let node = node_assets.get(node_handle)?;
                    Some((expression.clone(), Name::new(node.name.clone())))
                })
                .collect(),
        )
    }
}

/// Moves each expression track of `clip` onto the VRM root as an
/// [`ExpressionWeightProperty`] curve whose weight is the source `translation.x`.
pub(crate) fn retarget_expression_curves(
    clip: &mut AnimationClip,
    vrma: Entity,
    registry: &VrmaExpressionRegistry,
    searcher: &ChildSearcher,
    source_targets: &Query<&AnimationTargetId>,
) {
    let translation = translation_field_id();
    let root = vrm_root_animation_target();

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

/// Resets the weights of the expressions `vrma` animates to zero.
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

fn translation_field_id() -> FieldId {
    let field = animated_field!(Transform::translation);
    let EvaluatorId::ComponentField(id) = field.evaluator_id() else {
        unreachable!("`animated_field!` always yields an `EvaluatorId::ComponentField`")
    };
    *id
}

fn animates_field(
    curve: &VariableCurve,
    id: FieldId,
) -> bool {
    matches!(curve.0.evaluator_id(), EvaluatorId::ComponentField(other) if *other == id)
}

/// Samples `curve`'s `translation.x` into a weight curve; the `[0, 1]` clamp is
/// applied when the weight is distributed to the morph targets.
fn expression_weight_curve(
    expression: &str,
    curve: &VariableCurve,
) -> Option<ExpressionWeightCurve> {
    let domain = curve.0.domain();
    let (start, end) = (domain.start(), domain.end());
    let duration = end - start;
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
    use crate::vrma::{VrmAnimationClipHandle, VrmAnimationSourceClip, Vrma};
    use bevy::animation::{AnimatedBy, AnimationPlugin};

    /// Non-zero `y`/`z`, so a retarget reading the wrong component fails.
    const SCRATCH_Y: f32 = 0.125;
    const SCRATCH_Z: f32 = 0.375;

    /// A retargeted weight on the VRM root reaches the morph target through the bind pass.
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

    /// An app with the expression bind pass on the edges production uses.
    fn setup_app(with_animation: bool) -> App {
        let mut app = test_app();
        if with_animation {
            app.add_plugins(AnimationPlugin);
        } else {
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

    /// The destination avatar as `handler::scene::setup_animation` writes it.
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

    /// A `.vrma` child holding one expression track (`translation.x` is the weight)
    /// on the source node named `source_node`.
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

    /// Three frames, so the evaluation is guaranteed to see the new graph.
    fn run_frames(app: &mut App) {
        for _ in 0..3 {
            app.update();
        }
    }

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

    #[test]
    fn two_avatars_retargeted_from_one_clip_keep_independent_weights() -> TestResult {
        let mut app = setup_app(true);
        let (a, root_a) = spawn_avatar(app.world_mut());
        let (b, root_b) = spawn_avatar(app.world_mut());
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

    #[test]
    fn a_reloaded_expression_is_re_retargeted_from_the_pristine_source() -> TestResult {
        let mut app = setup_app(false);
        let (vrm, _) = spawn_avatar(app.world_mut());
        app.world_mut().entity_mut(vrm).insert(VrmGeneration(1));
        let (vrma, working) =
            spawn_expression_vrma(app.world_mut(), vrm, "happy", &[(0.0, 0.25), (1.0, 0.75)])?;
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

    fn bind_expression(
        app: &mut App,
        vrm: Entity,
        mesh: Entity,
    ) {
        bind_expressions(app, vrm, mesh, &[("happy", ExpressionCategory::Other)]);
    }

    #[test]
    fn a_playing_clip_overrides_a_user_trigger_on_the_same_expression() -> TestResult {
        let mut app = setup_app(true);
        app.add_plugins(VrmExpressionPlugin);
        let (vrm, root_bone) = spawn_avatar(app.world_mut());
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
        play(&mut app, root_bone, vrma);
        run_frames(&mut app);
        assert_eq!(weight_of(&app, vrm, "happy"), 1.0);

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

        set_weights(app.world_mut(), vrm, &[("happy", 0.9), ("aa", 0.3)]);

        app.world_mut()
            .commands()
            .entity(vrma)
            .trigger(PlayVrma::new);
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

    /// Starts a clip without a transition.
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
