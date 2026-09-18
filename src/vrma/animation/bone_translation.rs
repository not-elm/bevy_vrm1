use crate::prelude::RestTransform;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

#[derive(Component, Default, Clone, Debug, Deref, DerefMut)]
pub(crate) struct RetargetTranslationTable(pub HashMap<Entity, Transformation>);

pub(crate) fn compute_hips_transformation(
    src_rest: &RestTransform,
    src_rest_g: Vec3,
    dist_rest: &RestTransform,
    dist_rest_g: Vec3,
) -> Transformation {
    Transformation {
        src_rest_local: src_rest.translation,
        src_rest_g,
        dist_rest_local: dist_rest.translation,
        dist_rest_g,
    }
}

#[derive(Debug, Copy, Clone, Reflect)]
pub(crate) struct Transformation {
    pub(crate) src_rest_local: Vec3,
    pub(crate) src_rest_g: Vec3,
    pub(crate) dist_rest_local: Vec3,
    pub(crate) dist_rest_g: Vec3,
}

impl Transformation {
    pub fn transform(
        &self,
        src_pose: Vec3,
    ) -> Vec3 {
        calc_hips_position(
            self.src_rest_local,
            self.src_rest_g,
            src_pose,
            self.dist_rest_local,
            self.dist_rest_g,
        )
    }
}

/// Retargets a hips bone translation from source to target model space.
///
/// Uses **local** rest positions for delta computation and result placement
/// (matching `Transform::translation` coordinate space), and **global**
/// rest positions (in the model entity's frame, world prefix stripped) only
/// for the Y-based height scaling ratio.
#[inline]
pub(crate) fn calc_hips_position(
    src_rest_local: Vec3,
    src_rest_global: Vec3,
    src_pose: Vec3,
    dst_rest_local: Vec3,
    dst_rest_global: Vec3,
) -> Vec3 {
    let delta = src_pose - src_rest_local;
    let scaling = calc_scaling(dst_rest_global, src_rest_global);
    dst_rest_local + delta * scaling
}

#[inline]
fn calc_scaling(
    dist_rest_global_pos: Vec3,
    source_rest_global_pos: Vec3,
) -> f32 {
    if source_rest_global_pos.y.abs() < f32::EPSILON {
        return 1.0;
    }
    dist_rest_global_pos.y / source_rest_global_pos.y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::{RestGlobalTransform, RestWorldTransform};
    use crate::vrma::animation::bone_rotation::strip_world_prefix;
    use crate::vrma::animation::bone_translation::{calc_hips_position, calc_scaling};
    use bevy::math::Vec3;

    #[test]
    fn hips_height_scaling_ignores_application_placement() {
        let src_local = RestTransform(Transform::from_xyz(0.0, 0.8, 0.0));
        let dst_local = RestTransform(Transform::from_xyz(0.0, 1.2, 0.0));
        // Internal hierarchy offsets make model-space heights different from local heights.
        let src_model = Transform::from_xyz(0.0, 1.0, 0.0);
        let dst_model = Transform::from_xyz(0.0, 1.5, 0.0);
        let src_prefix =
            RestWorldTransform(GlobalTransform::from(Transform::from_xyz(2.0, 10.0, 3.0)));
        let dst_prefix =
            RestWorldTransform(GlobalTransform::from(Transform::from_xyz(-2.0, -4.0, 1.0)));
        let src_rest = RestGlobalTransform(src_prefix.0.mul_transform(src_model));
        let dst_rest = RestGlobalTransform(dst_prefix.0.mul_transform(dst_model));
        let transformation = compute_hips_transformation(
            &src_local,
            strip_world_prefix(&src_rest, Some(&src_prefix)).translation,
            &dst_local,
            strip_world_prefix(&dst_rest, Some(&dst_prefix)).translation,
        );
        let result = transformation.transform(src_local.translation + Vec3::new(0.2, 0.1, -0.4));
        assert!((result - Vec3::new(0.3, 1.35, -0.6)).length() < 0.0001);
    }

    #[test]
    fn test_scaling() {
        let scaling = calc_scaling(Vec3::splat(1.), Vec3::splat(2.));
        assert!((scaling - 0.5) < 0.001);
    }

    #[test]
    fn test_y_only_animation_no_x_shift() {
        // Source model: hips local rest at (0, 0.9, 0.01), global at (0.02, 0.9, 0.01)
        // Target model: hips local rest at (0, 1.0, 0.01), global at (0.01, 1.0, 0.01)
        // Animation: only Y changes (0, 0.95, 0.01) — no X movement
        let result = calc_hips_position(
            Vec3::new(0.0, 0.9, 0.01),  // src_rest_local
            Vec3::new(0.02, 0.9, 0.01), // src_rest_global
            Vec3::new(0.0, 0.95, 0.01), // src_pose (only Y changed)
            Vec3::new(0.0, 1.0, 0.01),  // dst_rest_local
            Vec3::new(0.01, 1.0, 0.01), // dst_rest_global
        );
        // X should remain at dst_rest_local.x (no phantom shift)
        assert!(
            (result.x - 0.0).abs() < 0.001,
            "X should not shift: {}",
            result.x
        );
        // Z should remain at dst_rest_local.z
        assert!(
            (result.z - 0.01).abs() < 0.001,
            "Z should not shift: {}",
            result.z
        );
    }

    #[test]
    fn test_local_equals_global_no_regression() {
        // When local == global (hips is root bone), result should be the same as before.
        let src_rest = Vec3::new(0.0, 0.9, 0.0);
        let dst_rest = Vec3::new(0.0, 1.0, 0.0);
        let src_pose = Vec3::new(0.0, 0.95, 0.0);
        let result = calc_hips_position(src_rest, src_rest, src_pose, dst_rest, dst_rest);
        let scaling = dst_rest.y / src_rest.y;
        let expected = dst_rest + (src_pose - src_rest) * scaling;
        assert!((result - expected).length() < 0.001);
    }
}
