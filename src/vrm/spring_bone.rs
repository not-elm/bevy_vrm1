pub mod registry;
mod setup;
mod update;

use crate::vrm::spring_bone::registry::SpringBoneRegistryPlugin;
use crate::vrm::spring_bone::setup::SpringBoneSetupPlugin;
use crate::vrm::spring_bone::update::SpringBoneUpdatePlugin;
use bevy::app::App;
use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// The component that holds the spring bone state of each Joint
///
/// Implement the method described in the  [Official documentation](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_springBone-1.0/README.ja.md#%E5%88%9D%E6%9C%9F%E5%8C%96)
#[derive(Component, Reflect, Debug, Serialize, Deserialize, Default, PartialEq)]
#[reflect(Component, Serialize, Deserialize, Default)]
pub struct SpringJointState {
    prev_tail: Vec3,
    current_tail: Vec3,
    bone_axis: Vec3,
    bone_length: f32,
    initial_local_matrix: Mat4,
    initial_local_rotation: Quat,
}

#[derive(Component, Reflect, Debug, Serialize, Deserialize, Clone, Eq, PartialEq, Default)]
#[reflect(Component, Serialize, Deserialize, Default)]
pub struct SpringRoot {
    /// Represents a list of entity of spring joints belonging to the spring chain.
    /// This component is inserted into the root entity of the chain.
    pub joints: SpringJoints,

    pub colliders: SpringColliders,

    /// If the spring chain has a center node,
    /// The inertia of the spring bone is evaluated in the [`Center Space`](https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_springBone-1.0#center-space).
    pub center_node: SpringCenterNode,
}

#[derive(Reflect, Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, Deref)]
#[reflect(Serialize, Deserialize, Default)]
pub struct SpringJoints(pub Vec<Entity>);

#[derive(Reflect, Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, Deref)]
#[reflect(Serialize, Deserialize, Default)]
pub struct SpringColliders(pub Vec<Entity>);

#[derive(Reflect, Eq, PartialEq, Debug, Serialize, Deserialize, Clone, Default, Deref)]
#[reflect(Serialize, Deserialize, Default)]
pub struct SpringCenterNode(pub Option<Entity>);

#[derive(Component, Reflect, Debug, Serialize, Deserialize, Copy, Clone, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct SpringJointProps {
    pub drag_force: f32,
    pub gravity_dir: Vec3,
    pub gravity_power: f32,
    pub hit_radius: f32,
    pub stiffness: f32,
}

pub struct VrmSpringBonePlugin;

impl Plugin for VrmSpringBonePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<SpringRoot>()
            .register_type::<SpringJointState>()
            .add_plugins((
                SpringBoneSetupPlugin,
                SpringBoneRegistryPlugin,
                SpringBoneUpdatePlugin,
            ));
    }
}
