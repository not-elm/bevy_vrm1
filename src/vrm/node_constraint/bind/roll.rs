use crate::prelude::{RestTransform, VrmSystemSets};
use crate::vrm::node_constraint::RollConstraintDestinations;
use bevy::app::AnimationSystems;
use bevy::prelude::*;

pub(crate) struct RollConstraintBindPlugin;

impl Plugin for RollConstraintBindPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(
            PostUpdate,
            bind_roll_constraints
                .in_set(VrmSystemSets::Constraints)
                .after(AnimationSystems),
        );
    }
}

fn bind_roll_constraints(
    par_commands: ParallelCommands,
    sources: Query<(&Transform, &RestTransform, &RollConstraintDestinations), Changed<Transform>>,
    dests: Query<(&Transform, &RestTransform)>,
) {
    sources
        .par_iter()
        .for_each(|(src_tf, src_rest, destinations)| {
            for dest in &destinations.0 {
                if let Ok((dest_tf, dest_rest)) = dests.get(dest.dest) {
                    let dest_rest_q = dest_rest.rotation;
                    let src_rest_q = src_rest.rotation;

                    let src_delta_quat = src_rest.rotation.inverse() * src_tf.rotation;
                    let delta_src_quat_in_parent =
                        src_rest_q * src_delta_quat * src_rest_q.inverse();
                    let delta_src_quat_in_dest =
                        dest_rest_q.inverse() * delta_src_quat_in_parent * dest_rest_q;

                    let to_vec = delta_src_quat_in_dest * dest.roll_axis.as_vec3();
                    let from_to_quat = Quat::from_rotation_arc(dest.roll_axis.as_vec3(), to_vec);

                    let new_rot = dest_rest.rotation.slerp(
                        dest_rest.rotation * from_to_quat.inverse() * delta_src_quat_in_dest,
                        dest.weight,
                    );
                    par_commands.command_scope(|mut commands| {
                        commands.entity(dest.dest).insert(Transform {
                            rotation: new_rot,
                            ..*dest_tf
                        });
                    });
                }
            }
        });
}
