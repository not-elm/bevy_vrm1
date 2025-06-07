use crate::system_set::VrmSystemSets;
use crate::vrm::gltf::extensions::vrmc_spring_bone::ColliderShape;
use crate::vrm::spring_bone::{SpringJointProps, SpringJointState, SpringRoot};
use bevy::app::App;
use bevy::math::Vec3;
use bevy::prelude::*;
use bevy::time::Time;

pub struct SpringBoneUpdatePlugin;

impl Plugin for SpringBoneUpdatePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(
            Update,
            update_spring_bones.in_set(VrmSystemSets::SpringBone),
        );
    }
}

fn update_spring_bones(
    mut transforms: Query<(&mut Transform, &mut GlobalTransform)>,
    mut joints: Query<(&ChildOf, &mut SpringJointState, &SpringJointProps)>,
    spring_roots: Query<&SpringRoot>,
    colliders: Query<&ColliderShape>,
    time: Res<Time>,
) {
    let delta_time = time.delta_secs();
    for spring_root in spring_roots.iter() {
        for joint in spring_root.joints.iter().copied() {
            let Ok((child_of, mut state, props)) = joints.get_mut(joint) else {
                continue;
            };
            let parent_gtf = transforms
                .get(child_of.parent())
                .map(|(_, gtf)| *gtf)
                .unwrap_or_default();
            let parent_global_rotation = parent_gtf.to_scale_rotation_translation().1;
            let Ok(head_global_pos) = transforms.get(joint).map(|(_, gtf)| gtf.translation())
            else {
                continue;
            };

            let inertia = (state.current_tail - state.prev_tail) * (1. - props.drag_force);
            let stiffness = delta_time
                * (parent_global_rotation
                    * state.initial_local_rotation
                    * state.bone_axis
                    * props.stiffness);
            let external = delta_time * props.gravity_dir * props.gravity_power;

            let next_tail = state.current_tail + inertia + stiffness + external;
            let mut next_tail =
                head_global_pos + (next_tail - head_global_pos).normalize() * state.bone_length;

            collision(
                &mut next_tail,
                spring_root.colliders.iter().copied(),
                props.hit_radius,
                head_global_pos,
                state.bone_length,
                &transforms,
                &colliders,
            );

            state.prev_tail = state.current_tail;
            state.current_tail = next_tail;

            let to = (parent_gtf.compute_matrix() * state.initial_local_matrix)
                .inverse()
                .transform_point3(next_tail)
                .normalize();

            let Ok((mut tf, mut gtf)) = transforms.get_mut(joint) else {
                continue;
            };

            tf.rotation =
                state.initial_local_rotation * Quat::from_rotation_arc(state.bone_axis, to);
            *gtf = parent_gtf.mul_transform(*tf);
        }
    }
}

fn collision(
    next_tail: &mut Vec3,
    collider_entities: impl Iterator<Item = Entity>,
    joint_radius: f32,
    head_global_pos: Vec3,
    bone_length: f32,
    transforms: &Query<(&mut Transform, &mut GlobalTransform)>,
    colliders: &Query<&ColliderShape>,
) {
    for collider in collider_entities {
        let Ok(collider_shape) = colliders.get(collider) else {
            continue;
        };
        let Ok((_, collider_gtf)) = transforms.get(collider) else {
            continue;
        };
        collider_shape.calc_collision(
            next_tail,
            collider_gtf,
            head_global_pos,
            joint_radius,
            bone_length,
        );
    }
}
