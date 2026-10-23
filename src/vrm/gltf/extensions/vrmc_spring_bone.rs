use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct VRMCSpringBone {
    /// Represents the specification version of the `VRMC_springBone` extension.
    #[serde(rename = "specVersion")]
    pub spec_version: String,

    /// [Collider]
    #[serde(default)]
    pub colliders: Vec<Collider>,

    /// [`ColliderGroup`]
    #[serde(rename = "colliderGroups", default)]
    pub collider_groups: Vec<ColliderGroup>,

    /// [Spring]
    #[serde(default)]
    pub springs: Vec<Spring>,
}

impl VRMCSpringBone {
    pub fn all_joints(&self) -> Vec<SpringJoint> {
        self.springs
            .iter()
            .flat_map(|spring| spring.joints.clone())
            .collect()
    }

    pub fn spring_colliders(
        &self,
        collider_group_indices: &[usize],
    ) -> Vec<Collider> {
        collider_group_indices
            .iter()
            .filter_map(|index| self.collider_groups.get(*index))
            .flat_map(|group| group.colliders.clone())
            .flat_map(|index| self.colliders.get(index as usize).cloned())
            .collect()
    }
}

#[derive(Serialize, Deserialize)]
pub struct ColliderGroup {
    /// Group name
    pub name: Option<String>,

    /// The list of colliders belonging to this group.
    /// Each value is an index of `VRMCSpringBone::colliders`.
    pub colliders: Vec<u64>,
}

/// Represents the collision detection for spring bone.
/// It consists of the target node index and the collider shape.
#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct Collider {
    pub node: usize,
    pub shape: ColliderShape,
}

#[derive(Serialize, Deserialize)]
pub struct Spring {
    /// Spring name
    #[serde(default)]
    pub name: String,

    /// The list of joints that make up the springBone.
    pub joints: Vec<SpringJoint>,

    /// Each value is an index of `VRMCSpringBone::colliderGroups`.
    #[serde(rename = "colliderGroups")]
    pub collider_groups: Option<Vec<usize>>,

    pub center: Option<usize>,
}

/// The node of a single glTF with spring bone settings.
#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct SpringJoint {
    pub node: usize,
    #[serde(rename = "dragForce", default = "default_drag_force")]
    pub drag_force: Option<f32>,
    #[serde(rename = "gravityDir", default = "default_gravity_dir")]
    pub gravity_dir: Option<[f32; 3]>,
    #[serde(rename = "gravityPower", default = "default_gravity_power")]
    pub gravity_power: Option<f32>,
    #[serde(rename = "hitRadius", default = "default_hit_radius")]
    pub hit_radius: Option<f32>,
    #[serde(default = "default_stiffness")]
    pub stiffness: Option<f32>,
}

fn default_drag_force() -> Option<f32> {
    Some(0.5)
}

fn default_gravity_dir() -> Option<[f32; 3]> {
    Some([0.0, -1.0, 0.0])
}

fn default_gravity_power() -> Option<f32> {
    Some(0.0)
}

fn default_hit_radius() -> Option<f32> {
    Some(0.0)
}

fn default_stiffness() -> Option<f32> {
    Some(1.0)
}

/// The shape of the collision detection for [Collider]
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Component, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColliderShape {
    Sphere(Sphere),
    Capsule(Capsule),
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Sphere(Sphere::default())
    }
}

impl ColliderShape {
    /// Returns the collision vector from the collider to the target position.
    pub fn apply_collision(
        &self,
        next_tail: &mut Vec3,
        collider: &GlobalTransform,
        head_global_pos: Vec3,
        joint_radius: f32,
        bone_length: f32,
    ) {
        let (scale, _, _) = collider.to_scale_rotation_translation();
        let max_collider_scale = scale.abs().max_element();
        match self {
            Self::Sphere(sphere) => {
                let translation = collider.transform_point(Vec3::from(sphere.offset));
                let r = joint_radius + sphere.radius * max_collider_scale;
                let delta = *next_tail - translation;
                let distance_squared = delta.length_squared();
                if distance_squared > 0.0 && distance_squared <= r * r {
                    let dir = delta.normalize();
                    let pos_from_collider = translation + dir * r;
                    *next_tail = head_global_pos
                        + (pos_from_collider - head_global_pos).normalize() * bone_length;
                }
            }
            Self::Capsule(_) => {
                //TODO: Not supported yet
            }
        }
    }

    #[inline]
    pub const fn radius(&self) -> f32 {
        match self {
            Self::Sphere(sphere) => sphere.radius,
            Self::Capsule(capsule) => capsule.radius,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Component, Reflect, Default)]
#[reflect(Component, Serialize, Deserialize)]
#[serde(default)]
pub struct Sphere {
    /// Local coordinate of the sphere center
    pub offset: [f32; 3],
    /// Radius of the sphere
    pub radius: f32,
}

/// 楕円形の
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Component, Reflect, Default)]
#[reflect(Component, Serialize, Deserialize)]
#[serde(default)]
pub struct Capsule {
    /// Local coordinate of the center of the half sphere at the start point of the capsule
    pub offset: [f32; 3],
    /// Radius of the half sphere and cylinder part of the capsule
    pub radius: f32,
    /// Local coordinate of the center of the half sphere at the end point of the capsule
    pub tail: [f32; 3],
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::extensions::vrmc_spring_bone::{
        Capsule, ColliderShape, Sphere, VRMCSpringBone,
    };

    #[test]
    fn deserialize_vrmc_spring_bone() -> TestResult {
        let _spring_bone: VRMCSpringBone =
            serde_json::from_str(include_str!("vrmc_spring_bone.json"))?;
        success!()
    }

    #[test]
    fn deserialize_vrmc_spring_bone_without_optional_collections() -> TestResult {
        let spring_bone: VRMCSpringBone = serde_json::from_str(r#"{"specVersion":"1.0"}"#)?;

        assert!(spring_bone.colliders.is_empty());
        assert!(spring_bone.collider_groups.is_empty());
        assert!(spring_bone.springs.is_empty());
        success!()
    }

    #[test]
    fn reject_invalid_vrmc_spring_bone_collections() {
        for invalid in [
            "{}",
            r#"{"specVersion":"1.0","colliders":null}"#,
            r#"{"specVersion":"1.0","colliderGroups":null}"#,
            r#"{"specVersion":"1.0","springs":null}"#,
        ] {
            assert!(serde_json::from_str::<VRMCSpringBone>(invalid).is_err());
        }
    }

    #[test]
    fn spring_joint_takes_the_specification_defaults_for_omitted_properties() -> TestResult {
        let spring_bone: VRMCSpringBone =
            serde_json::from_str(r#"{"specVersion":"1.0","springs":[{"joints":[{"node":3}]}]}"#)?;
        let joint = &spring_bone.springs[0].joints[0];

        assert_eq!(joint.node, 3);
        assert_eq!(joint.drag_force, Some(0.5));
        assert_eq!(joint.gravity_dir, Some([0.0, -1.0, 0.0]));
        assert_eq!(joint.gravity_power, Some(0.0));
        assert_eq!(joint.hit_radius, Some(0.0));
        assert_eq!(joint.stiffness, Some(1.0));

        success!()
    }

    #[test]
    fn spring_joint_keeps_the_values_of_a_partially_specified_joint() -> TestResult {
        let spring_bone: VRMCSpringBone = serde_json::from_str(
            r#"{"specVersion":"1.0","springs":[{"joints":[{"node":3,"stiffness":0.75}]}]}"#,
        )?;
        let joint = &spring_bone.springs[0].joints[0];

        assert_eq!(joint.stiffness, Some(0.75));
        assert_eq!(joint.drag_force, Some(0.5));
        assert_eq!(joint.gravity_dir, Some([0.0, -1.0, 0.0]));
        assert_eq!(joint.gravity_power, Some(0.0));
        assert_eq!(joint.hit_radius, Some(0.0));

        success!()
    }

    #[test]
    fn spring_takes_the_specification_defaults_for_omitted_properties() -> TestResult {
        let spring_bone: VRMCSpringBone =
            serde_json::from_str(r#"{"specVersion":"1.0","springs":[{"joints":[{"node":3}]}]}"#)?;
        let spring = &spring_bone.springs[0];

        assert_eq!(spring.name, "");
        assert!(spring.collider_groups.is_none());
        assert!(spring.center.is_none());

        success!()
    }

    #[test]
    fn collider_shape_takes_the_specification_defaults_for_omitted_properties() -> TestResult {
        let sphere: ColliderShape = serde_json::from_str(r#"{"sphere":{}}"#)?;
        assert_eq!(sphere, ColliderShape::Sphere(Sphere::default()));
        assert_eq!(sphere.radius(), 0.0);

        let capsule: ColliderShape = serde_json::from_str(r#"{"capsule":{}}"#)?;
        assert_eq!(capsule, ColliderShape::Capsule(Capsule::default()));
        assert_eq!(capsule.radius(), 0.0);

        let offset: ColliderShape = serde_json::from_str(r#"{"sphere":{"radius":0.5}}"#)?;
        assert_eq!(
            offset,
            ColliderShape::Sphere(Sphere {
                offset: [0.0; 3],
                radius: 0.5
            })
        );

        success!()
    }
}
