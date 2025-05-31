use crate::vrm::gltf::extensions::vrmc_vrm::LookAtProperties;
use crate::vrm::{Head, LeftEye, RightEye};
use bevy::app::{App, Plugin};
use bevy::prelude::*;
#[cfg(feature = "reflect")]
use serde::{Deserialize, Serialize};

/// Holds the entity of looking the target entity.
/// This component should be inserted into the root entity of the VRM.
/// TODO: 詳細な説明
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub enum LookAt {
    /// Look at the window cursor.
    Cursor,

    /// Specifiy the entity of target.
    Target(Entity),
}

pub struct LookAtPlugin;

impl Plugin for LookAtPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, (spawn_look_at_space, track_looking_target));

        #[cfg(feature = "reflect")]
        app.register_type::<LookAtSpace>();
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "reflect", derive(Reflect, Serialize, Deserialize))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
struct LookAtSpace(Entity);

/// See [`lookAt`](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/lookAt.md) for more details.
fn spawn_look_at_space(
    par_commands: ParallelCommands,
    vrms: Query<(Entity, &Head, &LookAtProperties), Added<Head>>,
    transforms: Query<&Transform>,
) {
    vrms.par_iter().for_each(|(entity, head, properties)| {
        let offset = Vec3::from_array(properties.offset_from_head_bone);
        let Ok(head_tf) = transforms.get(head.0) else {
            return;
        };
        let transform =
            Transform::from_translation(offset).with_rotation(head_tf.rotation.inverse());
        par_commands.command_scope(move |mut commands: Commands| {
            let look_at_space = commands.spawn(transform).id();
            commands.entity(head.0).add_child(look_at_space);
            commands.entity(entity).insert(LookAtSpace(look_at_space));
        });
    });
}

fn track_looking_target(
    par_commands: ParallelCommands,
    vrms: Query<(
        &LookAt,
        &LookAtProperties,
        &Head,
        &LookAtSpace,
        &LeftEye,
        &RightEye,
    )>,
    transforms: Query<&Transform>,
) {
    vrms.par_iter().for_each(
        |(look_at, properties, head, look_at_space, left_eye, right_eye)| {
            let Ok(look_at_space_tf) = transforms.get(look_at_space.0) else {
                return;
            };
            let Ok(head_tf) = transforms.get(head.0) else {
                return; // Head bone not found
            };
            let mut look_at_space_tf = *look_at_space_tf;
            look_at_space_tf.rotation = head_tf.rotation.inverse();
            let target = match look_at {
                LookAt::Cursor => {
                    todo!("Implement cursor tracking for LookAt");
                }
                LookAt::Target(target_entity) => {
                    match transforms.get(*target_entity) {
                        Ok(tf) => tf.translation,
                        Err(_) => return, // Target entity not found
                    }
                }
            };
            let (yaw, pitch) = calc_yaw_pitch(&look_at_space_tf, target);
            let Ok(left_eye_tf) = transforms.get(left_eye.0) else {
                return;
            };
            let Ok(right_eye_tf) = transforms.get(right_eye.0) else {
                return; // Right eye bone not found
            };
            let applied_left_eye_tf = apply_left_eye_bone(&left_eye_tf, properties, yaw, pitch);
            let applied_right_eye_tf = apply_right_eye_bone(&right_eye_tf, properties, yaw, pitch);
            par_commands.command_scope(move |mut commands: Commands| {
                commands.entity(left_eye.0).insert(applied_left_eye_tf);
                commands.entity(right_eye.0).insert(applied_right_eye_tf);
            });
        },
    );
}

fn calc_yaw_pitch(
    look_at_space: &Transform,
    target: Vec3,
) -> (f32, f32) {
    let local_target = look_at_space
        .compute_matrix()
        .inverse()
        .transform_point3(target);

    let z = local_target.dot(Vec3::Z);
    let x = local_target.dot(Vec3::X);
    let yaw = (x.atan2(z)).to_degrees();

    // x+y z plane
    let xz = (x * x + z * z).sqrt();
    let y = local_target.dot(Vec3::Y);
    let pitch = (-y.atan2(xz)).to_degrees();

    (yaw, pitch)
}

fn apply_left_eye_bone(
    left_eye: &Transform,
    properties: &LookAtProperties,
    yaw_degress: f32,
    pitch_degress: f32,
) -> Transform {
    let range_map_horizontal_outer = properties.range_map_horizontal_outer;
    let range_map_horizontal_inner = properties.range_map_horizontal_inner;
    let range_map_vertical_down = properties.range_map_vertical_down;
    let range_map_vertical_up = properties.range_map_vertical_up;
    let yaw = if yaw_degress > 0.0 {
        (yaw_degress.min(range_map_horizontal_outer.input_max_value)
            / range_map_horizontal_outer.input_max_value
            * range_map_horizontal_outer.output_scale)
    } else {
        -(yaw_degress
            .abs()
            .min(range_map_horizontal_inner.input_max_value)
            / range_map_horizontal_inner.input_max_value
            * range_map_horizontal_inner.output_scale)
    };

    let pitch = if pitch_degress > 0.0 {
        (pitch_degress.min(range_map_vertical_down.input_max_value)
            / range_map_vertical_down.input_max_value
            * range_map_vertical_down.output_scale)
    } else {
        -(pitch_degress
            .abs()
            .min(range_map_vertical_up.input_max_value)
            / range_map_vertical_up.input_max_value
            * range_map_vertical_up.output_scale)
    };
    left_eye.with_rotation(Quat::from_euler(EulerRot::YXZ, yaw.to_radians(), pitch.to_radians(), 0.0))
}

fn apply_right_eye_bone(
    right_eye: &Transform,
    properties: &LookAtProperties,
    yaw_degress: f32,
    pitch_degress: f32,
) -> Transform {
    let range_map_horizontal_outer = properties.range_map_horizontal_outer;
    let range_map_horizontal_inner = properties.range_map_horizontal_inner;
    let range_map_vertical_down = properties.range_map_vertical_down;
    let range_map_vertical_up = properties.range_map_vertical_up;

    let yaw = if yaw_degress > 0.0 {
        (yaw_degress.min(range_map_horizontal_inner.input_max_value)
            / range_map_horizontal_inner.input_max_value
            * range_map_horizontal_inner.output_scale)
    } else {
        -(yaw_degress
            .abs()
            .min(range_map_horizontal_outer.input_max_value)
            / range_map_horizontal_outer.input_max_value
            * range_map_horizontal_outer.output_scale)
    };

    let pitch = if pitch_degress > 0.0 {
        (pitch_degress.min(range_map_vertical_down.input_max_value)
            / range_map_vertical_down.input_max_value
            * range_map_vertical_down.output_scale)
    } else {
        -(pitch_degress
            .abs()
            .min(range_map_vertical_up.input_max_value)
            / range_map_vertical_up.input_max_value
            * range_map_vertical_up.output_scale)
    };
    
    right_eye.with_rotation(Quat::from_euler(EulerRot::YXZ, yaw.to_radians(), pitch.to_radians(), 0.0))
}
