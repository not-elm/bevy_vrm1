//! Clip baking via evaluator pipeline sampling.
//!
//! Samples existing [`VariableCurve`] values through the evaluator pipeline
//! (`create_evaluator -> apply -> commit`), applies retarget transformations,
//! and builds new standard [`AnimatableKeyframeCurve`] instances so that
//! custom curve wrappers are no longer needed at runtime.

use crate::vrma::animation::bone_rotation::Transformation as RotationTransformation;
use crate::vrma::animation::bone_translation::Transformation as TranslationTransformation;
use bevy::animation::{AnimationEntityMut, animated_field};
use bevy::curve::{ConstantCurve, Interval};
use bevy::prelude::*;

/// Samples per second when baking curves.
const SAMPLE_RATE: f32 = 120.0;

/// The most samples one baked curve takes, so a keyframe at a huge timestamp
/// cannot request an unbounded allocation; a longer curve is sampled sparser.
const MAX_SAMPLES: usize = 1 << 16;

/// The number of evenly spaced samples baking `duration` seconds takes.
pub(crate) fn sample_count(duration: f32) -> usize {
    ((duration * SAMPLE_RATE).ceil() as usize).clamp(2, MAX_SAMPLES)
}

/// Where and how often to sample a curve's `domain`, or `None` for a domain
/// that cannot be baked.
///
/// `bevy_gltf` builds a one-keyframe channel as a constant over all time
/// (`ConstantCurve` on `Interval::EVERYWHERE`), which has no duration to
/// resample. It is sampled once and baked back into a constant, rather than
/// skipped: a skipped curve keeps its source-space value and would be applied to
/// the destination bone without the retarget.
fn sampling(domain: Interval) -> Option<Sampling> {
    if !domain.is_bounded() {
        return Some(Sampling {
            start: 0.0,
            end: 0.0,
            count: 1,
            dt: 0.0,
        });
    }
    let (start, end) = (domain.start(), domain.end());
    let duration = end - start;
    if duration <= 0.0 || !duration.is_finite() {
        return None;
    }
    let count = sample_count(duration);
    Some(Sampling {
        start,
        end,
        count,
        dt: duration / (count - 1) as f32,
    })
}

/// See [`sampling`]. A single sample stands for a curve that is constant over
/// all time.
struct Sampling {
    start: f32,
    end: f32,
    count: usize,
    dt: f32,
}

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
    let Sampling {
        start,
        end,
        count: sample_count,
        dt,
    } = sampling(curve.0.domain())?;
    let constant = sample_count == 1;

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

    if constant {
        let (_, rotation) = *keyframes.first()?;
        return Some(VariableCurve::new(AnimatableCurve::new(
            animated_field!(Transform::rotation),
            ConstantCurve::new(Interval::EVERYWHERE, rotation),
        )));
    }
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
    let Sampling {
        start,
        end,
        count: sample_count,
        dt,
    } = sampling(curve.0.domain())?;
    let constant = sample_count == 1;

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

    if constant {
        let (_, translation) = *keyframes.first()?;
        return Some(VariableCurve::new(AnimatableCurve::new(
            animated_field!(Transform::translation),
            ConstantCurve::new(Interval::EVERYWHERE, translation),
        )));
    }
    // Build a new standard AnimatableKeyframeCurve from the baked keyframes.
    let baked_curve = AnimatableKeyframeCurve::new(keyframes).ok()?;
    Some(VariableCurve::new(AnimatableCurve::new(
        animated_field!(Transform::translation),
        baked_curve,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_count_is_bounded() {
        assert_eq!(sample_count(1.0), 120);
        assert_eq!(
            sample_count(0.001),
            2,
            "a curve takes at least its two ends"
        );
        assert_eq!(sample_count(1.0e30), MAX_SAMPLES);
        assert_eq!(sample_count(f32::MAX), MAX_SAMPLES);
    }

    /// The value a baked curve takes at `t`, read back through the evaluator.
    fn sampled(
        curve: &VariableCurve,
        t: f32,
        world: &mut World,
    ) -> Transform {
        let dummy = world.spawn(Transform::default()).id();
        let mut evaluator = curve.0.create_evaluator();
        curve
            .0
            .apply(&mut *evaluator, t, 1.0, AnimationNodeIndex::new(0))
            .expect("the curve applies");
        let mut query = world.query::<AnimationEntityMut>();
        evaluator
            .commit(query.get_mut(world, dummy).expect("the dummy exists"))
            .expect("the curve commits");
        let transform = *world.get::<Transform>(dummy).expect("the dummy exists");
        world.despawn(dummy);
        transform
    }

    /// `bevy_gltf` loads a one-keyframe channel as a `ConstantCurve` over all
    /// time. Skipping it would leave the source-space value on the destination
    /// bone; it is retargeted like any other curve and stays constant.
    #[test]
    fn a_constant_channel_is_retargeted_too() {
        let mut world = World::new();
        let pose = Quat::from_rotation_x(0.5);
        let rotation = VariableCurve::new(AnimatableCurve::new(
            animated_field!(Transform::rotation),
            ConstantCurve::new(Interval::EVERYWHERE, pose),
        ));
        let rotation_transformation = RotationTransformation::new(
            Quat::from_rotation_z(0.3),
            Quat::from_rotation_y(0.2),
            Quat::from_rotation_x(-0.4),
            Quat::from_rotation_z(0.1),
        );

        let baked = bake_rotation_curve(&rotation, &rotation_transformation, &mut world)
            .expect("a constant channel bakes");

        assert!(
            !baked.0.domain().is_bounded(),
            "it stays constant over all time"
        );
        let expected = rotation_transformation.transform(pose);
        for t in [0.0, 3.0] {
            let got = sampled(&baked, t, &mut world).rotation;
            assert!(
                got.abs_diff_eq(expected, 1e-5) || got.abs_diff_eq(-expected, 1e-5),
                "{got:?} at {t}, expected {expected:?}"
            );
        }

        let hips = Vec3::new(0.0, 1.0, 0.2);
        let translation = VariableCurve::new(AnimatableCurve::new(
            animated_field!(Transform::translation),
            ConstantCurve::new(Interval::EVERYWHERE, hips),
        ));
        let translation_transformation = TranslationTransformation {
            src_rest_local: Vec3::new(0.0, 1.0, 0.0),
            src_rest_g: Vec3::new(0.0, 1.0, 0.0),
            dist_rest_local: Vec3::new(0.0, 0.8, 0.0),
            dist_rest_g: Vec3::new(0.0, 0.8, 0.0),
        };

        let baked = bake_translation_curve(&translation, &translation_transformation, &mut world)
            .expect("a constant hips channel bakes");

        assert!(
            sampled(&baked, 1.0, &mut world)
                .translation
                .abs_diff_eq(translation_transformation.transform(hips), 1e-5)
        );
    }
}
