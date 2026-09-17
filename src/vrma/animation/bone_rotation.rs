use crate::prelude::*;
use crate::vrm::RestWorldTransform;
use crate::vrm::humanoid_bone::HumanoidBoneRegistry;
use bevy::animation::AnimationTargetId;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// Per-bone retarget transformations keyed by the owning VRMA entity.
/// Graph node indices can change when a graph is rebuilt.
#[derive(Component, Default, Clone, Debug, Deref, DerefMut)]
pub(crate) struct RetargetRotationTable(pub HashMap<Entity, Transformation>);

/// Removes application placement from the rest snapshot, retaining transforms
/// inside the imported model. Legacy rigs without a prefix use their raw rest.
pub(crate) fn strip_world_prefix(
    rest_g: &RestGlobalTransform,
    prefix: Option<&RestWorldTransform>,
) -> Transform {
    match prefix {
        Some(prefix) => rest_g.0.reparented_to(&prefix.0),
        None => rest_g.0.compute_transform(),
    }
}

pub(crate) fn compute_rotation_transformations(
    vrma: Entity,
    vrm_entity: Entity,
    root_bone: Entity,
    registry: &HumanoidBoneRegistry,
    searcher: &ChildSearcher,
    bones: &Query<(&RestTransform, &RestGlobalTransform, &AnimationTargetId)>,
    model_rests: &Query<&RestWorldTransform>,
) -> Vec<(Entity, Entity, Transformation)> {
    let src_prefix = model_rests.get(vrma).ok();
    let dist_prefix = model_rests.get(vrm_entity).ok();
    let mut result = Vec::new();
    for (bone, name) in registry.iter() {
        let Some(vrma_bone_entity) = searcher.find_from_name(vrma, name) else {
            continue;
        };
        let Some(rig_bone_entity) = searcher.find_by_bone_name(root_bone, bone) else {
            continue;
        };
        let Some((rest, rest_g, _)) = bones.get(rig_bone_entity).ok() else {
            continue;
        };
        let Some((vrma_rest, vrma_rest_g, _)) = bones.get(vrma_bone_entity).ok() else {
            continue;
        };
        // The compatibility equations use rests in each imported model's
        // T-pose space, independent of where the application placed the models.
        let src_rest_g = strip_world_prefix(vrma_rest_g, src_prefix);
        let dist_rest_g = strip_world_prefix(rest_g, dist_prefix);
        let transformation = Transformation::new(
            vrma_rest.0.rotation,
            src_rest_g.rotation,
            rest.0.rotation,
            dist_rest_g.rotation,
        );
        result.push((rig_bone_entity, vrma, transformation));
    }
    result
}

#[derive(Debug, Copy, Clone, Reflect)]
pub(crate) struct Transformation {
    src_rest: Quat,
    src_rest_g: Quat,
    dist_rest: Quat,
    dist_rest_g: Quat,
}

impl Transformation {
    pub(crate) fn new(
        src_rest: Quat,
        src_rest_g: Quat,
        dist_rest: Quat,
        dist_rest_g: Quat,
    ) -> Self {
        Self {
            src_rest,
            src_rest_g,
            dist_rest,
            dist_rest_g,
        }
    }

    pub fn transform(
        &self,
        src_pose: Quat,
    ) -> Quat {
        // Non-normative VRMA pose compatibility guidance:
        // https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm_animation-1.0/how_to_transform_human_pose.md
        let normalized_local_rotation =
            self.src_rest_g * self.src_rest.inverse() * src_pose * self.src_rest_g.inverse();
        self.dist_rest * self.dist_rest_g.inverse() * normalized_local_rotation * self.dist_rest_g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retarget_is_independent_of_placement_at_each_rest_snapshot() {
        let src_local = Quat::from_rotation_z(0.6);
        let src_model = Transform::from_rotation(Quat::from_rotation_x(-0.3) * src_local);
        let dst_local = Quat::from_rotation_x(0.4);
        let dst_model = Transform::from_rotation(Quat::from_rotation_z(-0.2) * dst_local);
        let normalized_pose = Quat::from_rotation_y(0.8);
        let src_pose =
            src_local * src_model.rotation.inverse() * normalized_pose * src_model.rotation;
        let expected =
            dst_local * dst_model.rotation.inverse() * normalized_pose * dst_model.rotation;

        for (src_yaw, dst_yaw) in [(0.0, 0.0), (std::f32::consts::FRAC_PI_2, 0.0), (-0.7, 2.1)] {
            let src_prefix = RestWorldTransform(GlobalTransform::from(
                Transform::from_xyz(7.0, -3.0, 1.0).with_rotation(Quat::from_rotation_y(src_yaw)),
            ));
            let dst_prefix = RestWorldTransform(GlobalTransform::from(
                Transform::from_xyz(-4.0, 6.0, 2.0).with_rotation(Quat::from_rotation_y(dst_yaw)),
            ));
            let src_rest = RestGlobalTransform(src_prefix.0.mul_transform(src_model));
            let dst_rest = RestGlobalTransform(dst_prefix.0.mul_transform(dst_model));
            let transformation = Transformation::new(
                src_local,
                strip_world_prefix(&src_rest, Some(&src_prefix)).rotation,
                dst_local,
                strip_world_prefix(&dst_rest, Some(&dst_prefix)).rotation,
            );
            assert!(transformation.transform(src_pose).angle_between(expected) < 0.001);
            assert!(transformation.transform(src_local).angle_between(dst_local) < 0.001);
        }
    }

    #[test]
    fn stripping_placement_preserves_internal_model_transforms() {
        let model = Transform::from_xyz(0.2, 1.4, -0.1).with_rotation(Quat::from_rotation_z(0.5));
        let prefix = RestWorldTransform(GlobalTransform::from(
            Transform::from_xyz(4.0, 8.0, -3.0)
                .with_rotation(Quat::from_rotation_y(1.2))
                .with_scale(Vec3::splat(2.0)),
        ));
        let rest = RestGlobalTransform(prefix.0.mul_transform(model));
        let stripped = strip_world_prefix(&rest, Some(&prefix));
        assert!((stripped.translation - model.translation).length() < 0.0001);
        assert!(stripped.rotation.angle_between(model.rotation) < 0.001);
        assert!((stripped.scale - model.scale).length() < 0.0001);
    }
}
