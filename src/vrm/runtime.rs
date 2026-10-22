//! The runtime half of the glTF-handler pipeline.
//!
//! The load-time handler writes node constraints into the scene `WorldAsset` as
//! [`ConstraintExecutionOrder`] (on the scene root) plus [`VrmNodeConstraint`]
//! (on each destination), so the evaluation *order* is data rather than schedule
//! configuration. [`apply_node_constraints`] is the whole runtime surface:
//! one ordered pass over that list, driven by no registry and no
//! `Changed<Transform>` trigger.
//!
//! # Why this is the only node-constraint evaluator
//!
//! The three constraints `VRMC_node_constraint` defines are read straight off
//! the glTF nodes by the handler (`handler::nodes::resolve_constraints`), which
//! resolves every reference while the entities that satisfy it are still in the
//! loader's world. There is nothing left for a runtime system to look up by
//! name, so there is no second evaluator to keep consistent with this one.

use crate::system_set::VrmSystemSets;
use crate::vrm::RestTransform;
use crate::vrm::components::{ConstraintExecutionOrder, VrmConstraintKind, VrmNodeConstraint};
use bevy::app::{AnimationSystems, App, Plugin};
use bevy::prelude::*;
use bevy::transform::TransformSystems;

/// How far [`compute_global`] walks up a parent chain before giving up.
///
/// `ChildOf` cycles are impossible through bevy's own hierarchy API, and
/// bevy's parallel propagation uses the same bound as "larger than any
/// reasonable tree depth" (`bevy_transform/src/systems.rs`). Exceeding it
/// only skips one constraint, which is the right failure mode for a
/// malformed scene.
const MAX_ANCESTOR_WALK: usize = 10_000;

/// Adds the runtime systems for scenes built by the glTF extension handler.
pub struct VrmGltfRuntimePlugin;

impl Plugin for VrmGltfRuntimePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        // `VrmSystemSets::Constraints` is the VRM spec's constraint step: it is
        // what `VrmSystemSets::PropagateAfterConstraints` is chained after
        // (`src/vrm.rs`) and what `VrmSystemSets::GazeControl` looks
        // downstream of, so putting the pass here gets the constraints'
        // `GlobalTransform` published before gaze control reads the head and
        // eyes — the established manual-propagation order, unchanged.
        //
        // `.after(AnimationSystems)` because the sources are humanoid bones
        // driven by `AnimationPlayer`.
        // `.before(TransformSystems::Propagate)` because this system writes
        // `Transform` and bevy's propagation reads it; the constraint step
        // must be done before the engine publishes globals, and bevy already
        // orders `AnimationSystems` the same way
        // (`bevy_animation/src/lib.rs`).
        app.add_systems(
            PostUpdate,
            apply_node_constraints
                .in_set(VrmSystemSets::Constraints)
                .after(AnimationSystems)
                .before(TransformSystems::Propagate),
        );

        // `VrmPlugin`'s manual propagation pass (`src/vrm.rs`) is only
        // constrained to run after `VrmSystemSets::Constraints`, and bevy's own
        // `TransformSystems::Propagate` is unordered against it, so which of
        // the two publishes a constrained bone first was up to the executor.
        //
        // That matters because the manual pass is what makes the constraints
        // visible to `VrmSystemSets::GazeControl`, which reads `GlobalTransform`
        // (`src/vrm/look_at.rs`), and because bevy's parallel propagation
        // skips any root whose `TransformTreeChanged` is not set
        // (`bevy_transform/src/systems.rs`); only bevy's `mark_dirty_trees`
        // sets it, and it runs inside `TransformSystems::Propagate`
        // (`bevy_transform/src/plugins.rs`). Ordering the manual pass after
        // it therefore guarantees both that the tree is marked and that the
        // globals are published, every frame.
        //
        // The edge is additive: the pass is idempotent, and it also resolves the
        // pre-existing `GlobalTransform` ambiguity between the two propagations.
        app.configure_sets(
            PostUpdate,
            VrmSystemSets::PropagateAfterConstraints.after(TransformSystems::Propagate),
        );
    }
}

/// Evaluates one scene's node constraints in the topological order the handler
/// computed at load time.
///
/// Every destination is recomputed from its rest pose on every frame, so a
/// constraint never feeds its own previous output back into itself, and a
/// source that is itself a destination is fresh for its dependents: the write
/// happens through `&mut Transform` inside this loop, before the next entry is
/// read. That is the entire reason the order is a precomputed list.
pub fn apply_node_constraints(
    orders: Query<&ConstraintExecutionOrder>,
    constraints: Query<&VrmNodeConstraint>,
    rests: Query<&RestTransform>,
    parents: Query<&ChildOf>,
    mut transforms: Query<&mut Transform>,
) {
    for order in orders.iter() {
        for &destination in order.destinations() {
            // A destination that has been despawned (or not spawned yet) is not
            // an error: the list is load-time data that outlives the entities
            // it names, so a stale entry is skipped and the walk continues.
            let Some(rotation) =
                constraint_rotation(destination, &constraints, &rests, &parents, &transforms)
            else {
                continue;
            };
            // Written only when it moved: `VrmSystemSets::DetermineRedraw` reads
            // `Changed<Transform>`, so a constant pose must not request a redraw.
            if let Ok(mut transform) = transforms.get_mut(destination)
                && transform.rotation != rotation
            {
                transform.rotation = rotation;
            }
        }
    }
}

/// The destination's new local rotation, or [`None`] to leave it as it is.
fn constraint_rotation(
    destination: Entity,
    constraints: &Query<&VrmNodeConstraint>,
    rests: &Query<&RestTransform>,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Option<Quat> {
    let constraint = constraints.get(destination).ok()?;
    let weight = constraint.weight;
    let destination_rest = rests.get(destination).ok()?.0;
    // The spec's `slerp(rest, .., 0.0)`: a faded-out constraint returns the
    // bone to rest rather than freezing it in the last driven pose.
    if weight == 0.0 {
        return Some(destination_rest.rotation);
    }

    match constraint.kind {
        VrmConstraintKind::Rotation => {
            let delta = source_pose(constraint.source, rests, transforms)?.1;
            Some(
                destination_rest
                    .rotation
                    .slerp(destination_rest.rotation * delta, weight),
            )
        }
        VrmConstraintKind::Roll { roll_axis } => {
            let (source_rest, delta) = source_pose(constraint.source, rests, transforms)?;
            roll_rotation(
                destination_rest.rotation,
                source_rest,
                delta,
                roll_axis,
                weight,
            )
        }
        VrmConstraintKind::Aim { aim_axis } => aim_rotation(
            destination,
            destination_rest.rotation,
            aim_axis,
            constraint.source,
            weight,
            parents,
            transforms,
        ),
    }
}

/// The source's rest rotation, and its rotation relative to that rest pose in
/// its own local frame — the spec's `deltaSrcQuat = srcRestQuat^-1 * srcQuat`.
fn source_pose(
    source: Entity,
    rests: &Query<&RestTransform>,
    transforms: &Query<&mut Transform>,
) -> Option<(Quat, Quat)> {
    let rest = rests.get(source).ok()?.0.rotation;
    let local = transforms.get(source).ok()?.rotation;
    Some((rest, rest.inverse() * local))
}

/// `VRMC_node_constraint-1.0`, roll constraint.
fn roll_rotation(
    destination_rest: Quat,
    source_rest: Quat,
    source_delta: Quat,
    roll_axis: Dir3,
    weight: f32,
) -> Option<Quat> {
    // Spec pseudocode:
    //   deltaSrcQuatInParent = srcRestQuat * deltaSrcQuat * srcRestQuat^-1
    //   deltaSrcQuatInDst    = dstRestQuat^-1 * deltaSrcQuatInParent * dstRestQuat
    // The first line conjugates by the *source's* rest rotation;
    // conjugating by `destination_rest` instead would rotate the delta through
    // the wrong basis.
    let delta_in_parent = source_rest * source_delta * source_rest.inverse();
    let delta_in_destination = destination_rest.inverse() * delta_in_parent * destination_rest;

    let axis = roll_axis.as_vec3();
    // `from_rotation_arc` needs unit inputs. The product is unit for unit
    // quaternions, but a `Transform` whose rotation was written as something
    // else must not turn into a NaN quaternion here.
    let to_vec = (delta_in_destination * axis).normalize_or_zero();
    if to_vec == Vec3::ZERO {
        return None;
    }
    let from_to = Quat::from_rotation_arc(axis, to_vec);

    Some(destination_rest.slerp(
        destination_rest * from_to.inverse() * delta_in_destination,
        weight,
    ))
}

/// `VRMC_node_constraint-1.0`, aim constraint.
fn aim_rotation(
    destination: Entity,
    destination_rest: Quat,
    aim_axis: Dir3,
    source: Entity,
    weight: f32,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Option<Quat> {
    // The spec's `dstParentWorldQuat`. A destination without a parent is a
    // root, and for a root that is the identity.
    let destination_parent = match parents.get(destination) {
        Ok(child_of) => compute_global(child_of.0, parents, transforms)?,
        Err(_) => GlobalTransform::IDENTITY,
    };
    let destination_parent_rotation = destination_parent.rotation();
    let source_global = compute_global(source, parents, transforms)?;
    let destination_local = *transforms.get(destination).ok()?;
    let destination_global = destination_parent.mul_transform(destination_local);

    // `toVec = (srcWorldPos - dstWorldPos).normalized`. A bare `Vec3::normalize()`
    // here would be `NaN` when the source and the destination coincide — a
    // configuration a sleeve bone and a hand bone reach whenever the arm is at
    // the origin — and the `NaN` would be written into the scene. There is no
    // direction to aim at, so the constraint is skipped instead.
    let to_vec =
        (source_global.translation() - destination_global.translation()).normalize_or_zero();
    if to_vec == Vec3::ZERO {
        return None;
    }

    // `fromVec = aimAxis.applyQuaternion(dstParentWorldQuat * dstRestQuat)`: the
    // rest pose's aim axis in world space. The destination's *current* rotation
    // is deliberately not used, or the constraint would chase its own output.
    let from_vec =
        (destination_parent_rotation * destination_rest * aim_axis.as_vec3()).normalize_or_zero();
    if from_vec == Vec3::ZERO {
        return None;
    }
    let from_to = Quat::from_rotation_arc(from_vec, to_vec);

    Some(destination_rest.slerp(
        destination_parent_rotation.inverse()
            * from_to
            * destination_parent_rotation
            * destination_rest,
        weight,
    ))
}

/// Accumulates an entity's world transform by walking up its parent chain.
///
/// The stored [`GlobalTransform`] component is deliberately not consulted. At
/// this point in `PostUpdate` bevy has not propagated yet —
/// `TransformSystems::Propagate` runs after `AnimationSystems` and
/// `VrmSystemSets::Constraints` is unordered against it — so the stored
/// `GlobalTransform` is whatever
/// `VrmSystemSets::PropagateAfterConstraints` published at the end of the
/// *previous* frame. Reading it here would break two things:
///
/// * `VrmSystemSets::PropagateAfterConstraints` publishes once, after the whole
///   set, so a source that is itself a constrained destination evaluated earlier
///   in `ConstraintExecutionOrder` would still read its pre-constraint pose.
///   Honouring the topological order means the *source* has to be read at the
///   moment its dependent runs, which no `GlobalTransform` can express.
/// * Whether the value happened to be one frame stale or current would depend
///   on how the scheduler interleaved the two sets, so the same rig could
///   behave differently run to run.
///
/// The chain is composed as affine matrices, exactly as [`GlobalTransform`]
/// itself is: `Transform::mul_transform` keeps rotation and per-axis scale
/// apart, which drops the shear a non-uniformly scaled ancestor above a rotated
/// bone produces, so the positions it gives would not be where the bones render.
fn compute_global(
    entity: Entity,
    parents: &Query<&ChildOf>,
    transforms: &Query<&mut Transform>,
) -> Option<GlobalTransform> {
    let mut global = GlobalTransform::from(*transforms.get(entity).ok()?);
    let mut current = entity;
    for _ in 0..MAX_ANCESTOR_WALK {
        let Ok(child_of) = parents.get(current) else {
            return Some(global);
        };
        current = child_of.parent();
        global = GlobalTransform::from(*transforms.get(current).ok()?) * global;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::transform::TransformPlugin;
    use bevy::transform::systems::{propagate_parent_transforms, sync_simple_transforms};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

    /// A destination entity for the schedule test's probes to find.
    #[derive(Resource)]
    struct Watched(Entity);

    /// What the probes saw.
    #[derive(Resource, Default)]
    struct Observed {
        after_propagate: Option<Quat>,
        at_gaze: Option<GlobalTransform>,
    }

    /// Stands in for the source bone a `AnimationPlugin` graph drives.
    #[derive(Component)]
    struct Animated;

    fn run_constraints(app: &mut App) {
        app.world_mut()
            .run_system_once(apply_node_constraints)
            .expect("Failed to run apply_node_constraints");
        app.world_mut().flush();
    }

    fn rotation(
        app: &App,
        entity: Entity,
    ) -> Quat {
        app.world().get::<Transform>(entity).unwrap().rotation
    }

    /// Rotates in place: the translation matters to the aim constraint, so it
    /// must survive.
    fn set_rotation(
        app: &mut App,
        entity: Entity,
        rotation: Quat,
    ) {
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .expect("the entity has a Transform")
            .rotation = rotation;
    }

    /// `q` and `-q` are the same rotation, so compare up to sign.
    fn rotation_is(
        actual: Quat,
        expected: Quat,
    ) -> bool {
        actual.abs_diff_eq(expected, 1e-5) || actual.abs_diff_eq(-expected, 1e-5)
    }

    fn rest(rotation: Quat) -> RestTransform {
        RestTransform(Transform::from_rotation(rotation))
    }

    /// The roll pseudocode from `VRMC_node_constraint-1.0`, transcribed so the
    /// test's oracle is independent of the implementation.
    fn spec_roll(
        source_rest: Quat,
        source_delta: Quat,
        destination_rest: Quat,
        axis: Dir3,
        weight: f32,
    ) -> Quat {
        let delta_in_parent = source_rest * source_delta * source_rest.inverse();
        let delta_in_destination = destination_rest.inverse() * delta_in_parent * destination_rest;
        let to_vec = delta_in_destination * axis.as_vec3();
        let from_to = Quat::from_rotation_arc(axis.as_vec3(), to_vec);
        destination_rest.slerp(
            destination_rest * from_to.inverse() * delta_in_destination,
            weight,
        )
    }

    #[test]
    fn rotation_constraint_blends_by_weight() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 0.5,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));
        run_constraints(&mut app);

        // `slerp(rest, rest * delta, 0.5)` is half of the source's 90 degrees.
        assert!(
            rotation_is(
                rotation(&app, destination),
                Quat::from_rotation_x(FRAC_PI_2 / 2.0)
            ),
            "got {:?}",
            rotation(&app, destination)
        );
        success!()
    }

    #[test]
    fn a_zero_weight_constraint_returns_the_destination_to_rest() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let destination_rest = Quat::from_rotation_y(FRAC_PI_3);
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(destination_rest),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));
        run_constraints(&mut app);
        assert!(!rotation_is(rotation(&app, destination), destination_rest));

        // Fading the constraint out is the spec's `slerp(rest, .., 0.0)`: the
        // bone goes back to rest instead of freezing in the last driven pose.
        app.world_mut()
            .get_mut::<VrmNodeConstraint>(destination)
            .unwrap()
            .weight = 0.0;
        run_constraints(&mut app);

        assert!(
            rotation_is(rotation(&app, destination), destination_rest),
            "got {:?}",
            rotation(&app, destination)
        );
        success!()
    }

    /// `Changed<Transform>` is what `VrmSystemSets::DetermineRedraw` reads, so a
    /// constraint whose result did not move must not touch the destination.
    #[test]
    fn an_unchanged_constraint_does_not_mark_the_destination_changed() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((
                Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_2)),
                rest(Quat::IDENTITY),
            ))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        let last_changed = |app: &App| {
            app.world()
                .entity(destination)
                .get_ref::<Transform>()
                .unwrap()
                .last_changed()
        };
        run_constraints(&mut app);
        let driven = last_changed(&app);
        run_constraints(&mut app);

        assert_eq!(last_changed(&app), driven);
        success!()
    }

    /// `deltaSrcQuatInParent` conjugates by the *source's* rest rotation. The
    /// two rest rotations are deliberately different, and the assertion that
    /// the results differ is what makes this test able to fail if the bases are
    /// swapped.
    #[test]
    fn roll_constraint_conjugates_through_the_source_rest_pose() -> TestResult {
        let mut app = test_app();
        let source_rest = Quat::IDENTITY;
        let destination_rest = Quat::from_rotation_y(FRAC_PI_2);
        let source_delta = Quat::from_rotation_x(FRAC_PI_2);
        let roll_axis = Dir3::Z;

        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(source_rest)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(destination_rest),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Roll { roll_axis },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        set_rotation(&mut app, source, source_delta);
        run_constraints(&mut app);

        let expected = spec_roll(source_rest, source_delta, destination_rest, roll_axis, 1.0);
        // The same formula with the destination's rest rotation where the
        // source's belongs: the bug the roll basis must not reintroduce.
        let wrong_basis = spec_roll(
            destination_rest,
            source_delta,
            destination_rest,
            roll_axis,
            1.0,
        );
        assert!(
            !rotation_is(expected, wrong_basis),
            "the two bases must give different results or this test proves nothing"
        );
        assert!(
            rotation_is(rotation(&app, destination), expected),
            "expected {:?}, got {:?}",
            expected,
            rotation(&app, destination)
        );
        success!()
    }

    /// The aim axis must end up pointing at the source **in world space**, so
    /// the destination's parent rotation has to be conjugated in. The parent is
    /// rotated 90 degrees about `Y` on purpose: treating
    /// `dstParentWorldQuat` as the identity would leave the axis at 90 degrees
    /// off and fail the alignment check.
    #[test]
    fn aim_constraint_points_its_axis_at_the_source_in_world_space() -> TestResult {
        let mut app = test_app();
        let parent = app
            .world_mut()
            .spawn(Transform::from_rotation(Quat::from_rotation_y(FRAC_PI_2)))
            .id();
        let aim_axis = Dir3::NEG_Z;

        let source = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 0.0, -1.0), rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                ChildOf(parent),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Aim { aim_axis },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        run_constraints(&mut app);

        let world = app.world();
        let parent_rotation = world.get::<Transform>(parent).expect("parent").rotation;
        let world_rotation =
            parent_rotation * world.get::<Transform>(destination).expect("dst").rotation;
        // The parent is a root, so its local rotation is its global rotation.
        assert_eq!(parent_rotation, Quat::from_rotation_y(FRAC_PI_2));

        let aimed = world_rotation * aim_axis.as_vec3();
        let expected =
            (world.get::<Transform>(source).expect("src").translation).normalize_or_zero();
        assert!(
            aimed.abs_diff_eq(expected, 1e-5),
            "aim axis points at {aimed:?}, the source is at {expected:?}"
        );
        success!()
    }

    /// A non-uniformly scaled ancestor above a rotated bone shears its subtree.
    /// The aim has to use the world position the bones render at, i.e. the
    /// affine composition `GlobalTransform` uses: composing `Transform`s
    /// rotation-and-scale-apart puts this source at `(0, 2, 0)` instead of
    /// `(0, 1, 0)`, above the destination instead of below it.
    #[test]
    fn aim_constraint_uses_the_affine_world_position_under_non_uniform_scale() -> TestResult {
        let mut app = test_app();
        let scaled_tf = Transform::from_scale(Vec3::new(2.0, 1.0, 1.0));
        let bone_tf = Transform::from_rotation(Quat::from_rotation_z(FRAC_PI_2));
        let source_tf = Transform::from_xyz(1.0, 0.0, 0.0);
        let scaled = app.world_mut().spawn(scaled_tf).id();
        let bone = app.world_mut().spawn((bone_tf, ChildOf(scaled))).id();
        let source = app
            .world_mut()
            .spawn((source_tf, rest(Quat::IDENTITY), ChildOf(bone)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 1.5, 0.0),
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Aim { aim_axis: Dir3::Y },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        run_constraints(&mut app);

        let rendered_source = (GlobalTransform::from(scaled_tf)
            * GlobalTransform::from(bone_tf)
            * GlobalTransform::from(source_tf))
        .translation();
        assert!(rendered_source.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-5));
        let expected = (rendered_source - Vec3::new(0.0, 1.5, 0.0)).normalize();
        let aimed = rotation(&app, destination) * Vec3::Y;
        assert!(
            aimed.abs_diff_eq(expected, 1e-4),
            "aim axis points at {aimed:?}, the rendered source is along {expected:?}"
        );
        success!()
    }

    /// Normalizing a zero-length vector would produce the `NaN` quaternion this
    /// test pins the absence of.
    #[test]
    fn aim_constraint_skips_a_destination_coincident_with_its_source() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::from_xyz(1.0, 0.0, 0.0), rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::from_xyz(1.0, 0.0, 0.0),
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Aim {
                        aim_axis: Dir3::NEG_Z,
                    },
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));

        let held = Quat::from_rotation_x(FRAC_PI_3);
        set_rotation(&mut app, destination, held);

        run_constraints(&mut app);

        let rotation = rotation(&app, destination);
        assert!(
            rotation.is_finite(),
            "the aim constraint wrote {rotation:?}"
        );
        assert_eq!(rotation, held, "a zero-length to-vector must be skipped");
        success!()
    }

    /// The whole point of the load-time topological sort: `tail`'s source is
    /// `middle`, and `middle` is itself constrained. Reading `middle` before
    /// its own constraint ran gives a different answer than reading it after.
    #[test]
    fn the_execution_order_decides_whether_a_dependent_sees_a_fresh_source() -> TestResult {
        let mut app = test_app();
        let driver = app
            .world_mut()
            .spawn((
                Transform::from_rotation(Quat::from_rotation_x(FRAC_PI_2)),
                rest(Quat::IDENTITY),
            ))
            .id();
        let middle = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source: driver,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let tail = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source: middle,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let order = app.world_mut().spawn_empty().id();
        app.world_mut()
            .entity_mut(order)
            .insert(ConstraintExecutionOrder(vec![middle, tail]));

        run_constraints(&mut app);

        let driven = Quat::from_rotation_x(FRAC_PI_2);
        assert!(rotation_is(rotation(&app, middle), driven));
        // `middle` was written before `tail` read it, so `tail` inherits the
        // driver's rotation rather than `middle`'s rest pose.
        assert!(
            rotation_is(rotation(&app, tail), driven),
            "got {:?}",
            rotation(&app, tail)
        );

        // Reversed order, poses reset: `tail` now reads `middle` before its own
        // constraint, so it stays at its rest rotation.
        set_rotation(&mut app, middle, Quat::IDENTITY);
        set_rotation(&mut app, tail, Quat::IDENTITY);
        app.world_mut()
            .entity_mut(order)
            .insert(ConstraintExecutionOrder(vec![tail, middle]));

        run_constraints(&mut app);

        assert!(
            rotation_is(rotation(&app, tail), Quat::IDENTITY),
            "got {:?}",
            rotation(&app, tail)
        );
        assert!(rotation_is(rotation(&app, middle), driven));
        success!()
    }

    #[test]
    fn a_destination_that_is_no_longer_in_the_world_is_skipped() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let gone = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        // The dead destination comes first: the walk has to continue, not bail.
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![gone, destination]));

        app.world_mut().despawn(gone);

        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));
        run_constraints(&mut app);

        assert!(rotation_is(
            rotation(&app, destination),
            Quat::from_rotation_x(FRAC_PI_2)
        ));
        success!()
    }

    /// Nothing but the execution order starts this system: a `VrmNodeConstraint`
    /// with no order listing its entity is left alone. The order is what
    /// `handler::scene::resolve_constraints` computed at load time, and it is the
    /// only input that says a destination is part of a scene.
    #[test]
    fn only_the_execution_order_drives_this_system() -> TestResult {
        let mut app = test_app();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY)))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::IDENTITY,
                rest(Quat::IDENTITY),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        set_rotation(&mut app, source, Quat::from_rotation_x(FRAC_PI_2));

        // No entity carries `ConstraintExecutionOrder` yet.
        run_constraints(&mut app);
        assert_eq!(
            rotation(&app, destination),
            Quat::IDENTITY,
            "a destination the execution order does not name must not move"
        );

        // The execution order is the only thing that starts it.
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));
        run_constraints(&mut app);
        assert!(rotation_is(
            rotation(&app, destination),
            Quat::from_rotation_x(FRAC_PI_2)
        ));
        success!()
    }

    fn animate_source(mut sources: Query<&mut Transform, With<Animated>>) {
        for mut transform in &mut sources {
            transform.rotation = Quat::from_rotation_x(FRAC_PI_2);
        }
    }

    fn observe_after_propagate(
        watched: Res<Watched>,
        transforms: Query<&Transform>,
        mut observed: ResMut<Observed>,
    ) {
        observed.after_propagate = transforms.get(watched.0).ok().map(|tf| tf.rotation);
    }

    fn observe_at_gaze(
        watched: Res<Watched>,
        globals: Query<&GlobalTransform>,
        mut observed: ResMut<Observed>,
    ) {
        observed.at_gaze = globals.get(watched.0).ok().copied();
    }

    /// Pins the two edges the plugin adds, plus the crate's manual propagation:
    /// the constraint must see `AnimationSystems` output, and by the time
    /// `TransformSystems::Propagate` has run the rotation is already the
    /// constrained one. The second probe stands in for `GazeControl`, which
    /// `VrmPlugin` orders after `VrmSystemSets::PropagateAfterConstraints`, and
    /// reads `GlobalTransform`.
    #[test]
    fn constraints_run_after_animation_and_before_propagation() -> TestResult {
        fn register(app: &mut App) {
            app.add_plugins((TransformPlugin, VrmGltfRuntimePlugin));
            // Stands in for `AnimationPlugin`'s output.
            app.add_systems(PostUpdate, animate_source.in_set(AnimationSystems));
            app.add_systems(
                PostUpdate,
                observe_after_propagate.after(TransformSystems::Propagate),
            );
            // Installed by `VrmPlugin` exactly like this (`src/vrm.rs`).
            app.add_systems(
                PostUpdate,
                (sync_simple_transforms, propagate_parent_transforms)
                    .chain()
                    .in_set(VrmSystemSets::PropagateAfterConstraints)
                    .after(VrmSystemSets::Constraints)
                    .before(VrmSystemSets::GazeControl),
            );
            app.add_systems(
                PostUpdate,
                observe_at_gaze
                    .in_set(VrmSystemSets::GazeControl)
                    .after(VrmSystemSets::PropagateAfterConstraints),
            );
        }

        let mut app = test_app();
        register(&mut app);
        app.init_resource::<Observed>();

        let root = app
            .world_mut()
            .spawn(Transform::from_xyz(0.0, 1.0, 0.0))
            .id();
        let source = app
            .world_mut()
            .spawn((Transform::IDENTITY, rest(Quat::IDENTITY), Animated))
            .id();
        let destination = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 0.0, 0.5),
                rest(Quat::IDENTITY),
                ChildOf(root),
                VrmNodeConstraint {
                    source,
                    weight: 1.0,
                    kind: VrmConstraintKind::Rotation,
                },
            ))
            .id();
        app.world_mut()
            .spawn(ConstraintExecutionOrder(vec![destination]));
        app.insert_resource(Watched(destination));

        app.update();

        let expected = Quat::from_rotation_x(FRAC_PI_2);
        let observed = app.world().resource::<Observed>();
        assert!(
            observed
                .after_propagate
                .is_some_and(|seen| rotation_is(seen, expected)),
            "the constraint had not run when TransformSystems::Propagate finished: {:?}",
            observed.after_propagate
        );

        let global = observed
            .at_gaze
            .expect("GazeControl must see the destination's GlobalTransform");
        assert!(
            rotation_is(global.rotation(), expected),
            "the manual propagation published {:?}",
            global.rotation()
        );
        assert!(
            global
                .translation()
                .abs_diff_eq(Vec3::new(0.0, 1.0, 0.5), 1e-5),
            "the manual propagation published {:?}",
            global.translation()
        );
        success!()
    }
}
