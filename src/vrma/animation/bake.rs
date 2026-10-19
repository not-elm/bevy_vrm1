//! Clip baking via evaluator pipeline sampling.
//!
//! Samples existing [`VariableCurve`] values through the evaluator pipeline
//! (`create_evaluator -> apply -> commit`), applies retarget transformations,
//! and builds new standard [`AnimatableKeyframeCurve`] instances so that
//! custom curve wrappers are no longer needed at runtime.

use crate::vrma::animation::bone_rotation::Transformation as RotationTransformation;
use crate::vrma::animation::bone_translation::Transformation as TranslationTransformation;
use bevy::animation::{AnimationEntityMut, animated_field};
use bevy::prelude::*;

/// Samples per second when baking curves.
pub(crate) const SAMPLE_RATE: f32 = 120.0;

/// Bake a rotation [`VariableCurve`]: sample the original curve via the
/// evaluator pipeline, apply the retarget transformation to each sample,
/// and rebuild as a standard [`AnimatableKeyframeCurve<Quat>`].
///
/// Returns `None` if baking fails (e.g. domain is degenerate, curve has
/// fewer than 2 resulting samples, or evaluator errors occur).
pub(crate) fn bake_rotation_curve(
    curve: &VariableCurve,
    transformation: &RotationTransformation,
    world: &mut World,
) -> Option<VariableCurve> {
    let domain = curve.0.domain();
    let start = domain.start();
    let end = domain.end();
    let duration = end - start;
    if duration <= 0.0 || !duration.is_finite() {
        return None;
    }

    let sample_count = ((duration * SAMPLE_RATE).ceil() as usize).max(2);
    let dt = duration / (sample_count - 1) as f32;

    // Spawn a dummy entity with Transform for the evaluator to write into.
    let dummy = world.spawn(Transform::default()).id();

    let mut evaluator = curve.0.create_evaluator();
    let graph_node = AnimationNodeIndex::new(0);

    let mut keyframes: Vec<(f32, Quat)> = Vec::with_capacity(sample_count);

    // Cache the query state outside the loop for efficiency.
    let mut query = world.query::<AnimationEntityMut>();

    for i in 0..sample_count {
        let t = (start + dt * i as f32).min(end);

        // Reset the dummy entity's rotation before sampling.
        if let Some(mut tf) = world.get_mut::<Transform>(dummy) {
            tf.rotation = Quat::IDENTITY;
        }

        // Step 1: apply() pushes the sampled value onto the evaluator stack.
        if curve.0.apply(&mut *evaluator, t, 1.0, graph_node).is_err() {
            world.despawn(dummy);
            return None;
        }

        // Step 2: commit() pops from the stack and writes to the entity's Transform.
        {
            let Ok(entity_mut) = query.get_mut(world, dummy) else {
                world.despawn(dummy);
                return None;
            };
            if evaluator.commit(entity_mut).is_err() {
                world.despawn(dummy);
                return None;
            }
        }

        // Step 3: Read the sampled rotation and apply the retarget transformation.
        let sampled_rotation = world
            .get::<Transform>(dummy)
            .map(|tf| tf.rotation)
            .unwrap_or(Quat::IDENTITY);

        let retargeted = transformation.transform(sampled_rotation);
        keyframes.push((t, retargeted));
    }

    world.despawn(dummy);

    // Build a new standard AnimatableKeyframeCurve from the baked keyframes.
    let baked_curve = AnimatableKeyframeCurve::new(keyframes).ok()?;
    Some(VariableCurve::new(AnimatableCurve::new(
        animated_field!(Transform::rotation),
        baked_curve,
    )))
}

/// Bake a translation [`VariableCurve`] for hips: sample the original curve
/// via the evaluator pipeline, apply the retarget transformation to each
/// sample, and rebuild as a standard [`AnimatableKeyframeCurve<Vec3>`].
///
/// Returns `None` if baking fails.
pub(crate) fn bake_translation_curve(
    curve: &VariableCurve,
    transformation: &TranslationTransformation,
    world: &mut World,
) -> Option<VariableCurve> {
    let domain = curve.0.domain();
    let start = domain.start();
    let end = domain.end();
    let duration = end - start;
    if duration <= 0.0 || !duration.is_finite() {
        return None;
    }

    let sample_count = ((duration * SAMPLE_RATE).ceil() as usize).max(2);
    let dt = duration / (sample_count - 1) as f32;

    // Spawn a dummy entity with Transform for the evaluator to write into.
    let dummy = world.spawn(Transform::default()).id();

    let mut evaluator = curve.0.create_evaluator();
    let graph_node = AnimationNodeIndex::new(0);

    let mut keyframes: Vec<(f32, Vec3)> = Vec::with_capacity(sample_count);

    // Cache the query state outside the loop for efficiency.
    let mut query = world.query::<AnimationEntityMut>();

    for i in 0..sample_count {
        let t = (start + dt * i as f32).min(end);

        // Reset the dummy entity's translation before sampling.
        if let Some(mut tf) = world.get_mut::<Transform>(dummy) {
            tf.translation = Vec3::ZERO;
        }

        // Step 1: apply() pushes the sampled value onto the evaluator stack.
        if curve.0.apply(&mut *evaluator, t, 1.0, graph_node).is_err() {
            world.despawn(dummy);
            return None;
        }

        // Step 2: commit() pops from the stack and writes to the entity's Transform.
        {
            let Ok(entity_mut) = query.get_mut(world, dummy) else {
                world.despawn(dummy);
                return None;
            };
            if evaluator.commit(entity_mut).is_err() {
                world.despawn(dummy);
                return None;
            }
        }

        // Step 3: Read the sampled translation and apply the retarget transformation.
        let sampled_translation = world
            .get::<Transform>(dummy)
            .map(|tf| tf.translation)
            .unwrap_or(Vec3::ZERO);

        let retargeted = transformation.transform(sampled_translation);
        keyframes.push((t, retargeted));
    }

    world.despawn(dummy);

    // Build a new standard AnimatableKeyframeCurve from the baked keyframes.
    let baked_curve = AnimatableKeyframeCurve::new(keyframes).ok()?;
    Some(VariableCurve::new(AnimatableCurve::new(
        animated_field!(Transform::translation),
        baked_curve,
    )))
}
