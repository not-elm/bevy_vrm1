use crate::macros::marker_component;
use crate::system_param::child_searcher::ChildSearcher;
use crate::vrm::humanoid_bone::{Hips, HumanoidBoneRegistry, HumanoidBonesAttached};
use crate::vrm::{BoneRestGlobalTransform, VrmHipsBoneTo};
use crate::vrma::retarget::{CurrentRetargeting, RetargetBindingSystemSet};
use crate::vrma::{RetargetSource, RetargetTo};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub struct VrmaRetargetingBonePlugin;

impl Plugin for VrmaRetargetingBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<RetargetedHumanBones>()
            .register_type::<RetargetBoneTo>()
            // For some reason, it might not retarget unless the system runs on `PreUpdate`.
            .add_systems(PreUpdate, retarget_bones_to_vrm)
            .add_systems(Update, bind_bone_rotations.in_set(RetargetBindingSystemSet));
    }
}

#[derive(Debug, Component, Reflect)]
struct RetargetBoneTo(pub Entity);

marker_component!(
    /// A marker component that indicates that initialization of humanoid bones has been completed.
    ///
    /// This is attached to the VRM entity.
    RetargetedHumanBones
);

pub fn retarget_bones_to_vrm(
    par_commands: ParallelCommands,
    bones: Query<
        (Entity, &RetargetTo, &HumanoidBoneRegistry),
        (Without<RetargetedHumanBones>, With<HumanoidBonesAttached>),
    >,
    hips: Query<&VrmHipsBoneTo>,
    _names: Query<&Name>,
    searcher: ChildSearcher,
) {
    bones
        .par_iter()
        .for_each(|(entity, retarget, humanoid_bones)| {
            let Ok(dist_hips) = hips.get(retarget.0) else {
                return;
            };
            for (bone, _) in humanoid_bones.iter() {
                let Some(src_bone_entity) = searcher.find_from_bone_name(entity, bone) else {
                    continue;
                };
                let Some(dist_bone_entity) = searcher.find_from_bone_name(dist_hips.0, bone) else {
                    #[cfg(feature = "log")]
                    {
                        let dist_name = _names.get(retarget.0).unwrap();
                        error!("[Bone] {dist_name}'s {bone} not found");
                    }
                    continue;
                };
                par_commands.command_scope(|mut commands| {
                    commands
                        .entity(src_bone_entity)
                        .insert((RetargetSource, RetargetBoneTo(dist_bone_entity)));
                });
            }

            par_commands.command_scope(|mut commands| {
                commands.entity(entity).insert(RetargetedHumanBones);
            });
        });
}

fn bind_bone_rotations(
    par_commands: ParallelCommands,
    sources: Query<
        (
            &RetargetBoneTo,
            &Transform,
            &BoneRestGlobalTransform,
            Option<&Hips>,
        ),
        (Changed<Transform>, With<CurrentRetargeting>),
    >,
    dist_bones: Query<(&Transform, &BoneRestGlobalTransform)>,
) {
    sources.par_iter().for_each(
        |(retarget_bone_to, src_pose_tf, src_rest_gtf, maybe_hips)| {
            let Ok((dist_pose_tf, dist_rest_gtf)) = dist_bones.get(retarget_bone_to.0) else {
                return;
            };
            let transform = Transform {
                rotation: src_pose_tf.rotation,
                translation: if maybe_hips.is_some() {
                    calc_hips_position(
                        src_rest_gtf.0.translation(),
                        src_pose_tf.translation,
                        dist_rest_gtf.0.translation(),
                    )
                } else {
                    dist_pose_tf.translation
                },
                scale: dist_pose_tf.scale,
            };
            par_commands.command_scope(|mut commands| {
                commands.entity(retarget_bone_to.0).insert(transform);
            });
        },
    );
}

#[inline]
fn calc_scaling(
    dist_rest_global_pos: Vec3,
    source_rest_global_pos: Vec3,
) -> f32 {
    dist_rest_global_pos.y / source_rest_global_pos.y
}

#[inline]
fn calc_delta(
    source_pose_pos: Vec3,
    source_rest_global_pos: Vec3,
) -> Vec3 {
    source_pose_pos - source_rest_global_pos
}

fn calc_hips_position(
    source_rest_global_pos: Vec3,
    source_pose_pos: Vec3,
    dist_rest_global_pos: Vec3,
) -> Vec3 {
    let delta = calc_delta(source_pose_pos, source_rest_global_pos);
    let scaling = calc_scaling(dist_rest_global_pos, source_rest_global_pos);
    dist_rest_global_pos + delta * scaling
}

#[cfg(test)]
mod tests {
    use crate::tests::{test_app, TestResult};
    use crate::vrm::humanoid_bone::{HumanoidBoneRegistry, HumanoidBonesAttached};
    use crate::vrm::VrmHipsBoneTo;
    use crate::vrma::retarget::bone::{
        calc_delta, calc_scaling, retarget_bones_to_vrm, RetargetedHumanBones,
    };
    use crate::vrma::RetargetTo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::math::Vec3;
    use bevy::prelude::{Commands, Entity};

    #[test]
    fn test_scaling() {
        let scaling = calc_scaling(Vec3::splat(1.), Vec3::splat(2.));
        assert!((scaling - 0.5) < 0.001);
    }

    #[test]
    fn test_delta() {
        let delta = calc_delta(Vec3::splat(1.), Vec3::splat(2.));
        assert_eq!(delta, Vec3::splat(-1.));
    }

    #[test]
    fn has_been_attached_humanoid_bones() -> TestResult {
        let mut app = test_app();
        app.world_mut().run_system_once(|mut commands: Commands| {
            let vrm = commands.spawn(VrmHipsBoneTo(Entity::PLACEHOLDER)).id();
            commands.spawn((
                HumanoidBoneRegistry::default(),
                RetargetTo(vrm),
                HumanoidBonesAttached,
            ));
        })?;
        app.world_mut().run_system_once(retarget_bones_to_vrm)?;
        assert!(app
            .world_mut()
            .query::<&RetargetedHumanBones>()
            .single(app.world())
            .is_ok());
        Ok(())
    }
}
