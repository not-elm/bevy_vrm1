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
    /// Pushes `next_tail` out of the collider and re-projects it onto the bone-length sphere, handling endpoint and segment contact.
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
                let center = collider.transform_point(Vec3::from(sphere.offset));
                let r = joint_radius + sphere.radius * max_collider_scale;
                apply_sphere_collision(next_tail, center, r, head_global_pos, bone_length);
            }
            Self::Capsule(capsule) => {
                let r = joint_radius + capsule.radius * max_collider_scale;
                let start = collider.transform_point(Vec3::from(capsule.offset));
                let end = collider.transform_point(Vec3::from(capsule.tail));
                apply_capsule_collision(next_tail, start, end, r, head_global_pos, bone_length);
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

fn closest_point_on_segment(
    point: Vec3,
    start: Vec3,
    end: Vec3,
) -> Vec3 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        return start;
    }
    let t = (point - start).dot(segment) / length_squared;
    start + segment * t.clamp(0.0, 1.0)
}

fn closest_points_between_segments(
    p1: Vec3,
    q1: Vec3,
    p2: Vec3,
    q2: Vec3,
) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);
    const EPS: f32 = 1e-12;
    let (s, t) = if a <= EPS && e <= EPS {
        (0.0, 0.0)
    } else if a <= EPS {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > EPS {
                (b * f - c * e) / denom
            } else {
                0.0
            };
            s = s.clamp(0.0, 1.0);
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p1 + d1 * s, p2 + d2 * t)
}

fn apply_sphere_collision(
    next_tail: &mut Vec3,
    center: Vec3,
    r: f32,
    head_global_pos: Vec3,
    bone_length: f32,
) {
    let endpoint_delta = *next_tail - center;
    if endpoint_delta.length_squared() <= r * r {
        push_out_of_sphere(next_tail, center, r, head_global_pos, bone_length);
        return;
    }
    let closest = closest_point_on_segment(center, head_global_pos, *next_tail);
    let delta = closest - center;
    if delta.length_squared() >= r * r {
        return;
    }
    push_out_of_segment(
        next_tail,
        closest,
        center,
        delta,
        r,
        head_global_pos,
        bone_length,
    );
}

fn apply_capsule_collision(
    next_tail: &mut Vec3,
    start: Vec3,
    end: Vec3,
    r: f32,
    head_global_pos: Vec3,
    bone_length: f32,
) {
    let endpoint_closest = closest_point_on_segment(*next_tail, start, end);
    let endpoint_delta = *next_tail - endpoint_closest;
    if endpoint_delta.length_squared() <= r * r {
        push_out_of_sphere(next_tail, endpoint_closest, r, head_global_pos, bone_length);
        return;
    }
    let (closest, collider_point) =
        closest_points_between_segments(head_global_pos, *next_tail, start, end);
    let delta = closest - collider_point;
    if delta.length_squared() >= r * r {
        return;
    }
    push_out_of_segment(
        next_tail,
        closest,
        collider_point,
        delta,
        r,
        head_global_pos,
        bone_length,
    );
}

fn push_out_of_segment(
    next_tail: &mut Vec3,
    closest: Vec3,
    collider_point: Vec3,
    delta: Vec3,
    r: f32,
    head_global_pos: Vec3,
    bone_length: f32,
) {
    if (closest - head_global_pos).length_squared() <= f32::EPSILON {
        push_out_of_sphere(next_tail, collider_point, r, head_global_pos, bone_length);
        return;
    }
    let distance = delta.length();
    let Some(direction) = delta
        .try_normalize()
        .or_else(|| (head_global_pos - collider_point).try_normalize())
    else {
        return;
    };
    let displaced = *next_tail + direction * (r - distance);
    *next_tail = head_global_pos + (displaced - head_global_pos).normalize_or_zero() * bone_length;
}

fn push_out_of_sphere(
    next_tail: &mut Vec3,
    collider_center: Vec3,
    r: f32,
    head_global_pos: Vec3,
    bone_length: f32,
) {
    let delta = *next_tail - collider_center;
    let distance_squared = delta.length_squared();
    if distance_squared <= r * r {
        let Some(dir) = delta
            .try_normalize()
            .or_else(|| (head_global_pos - collider_center).try_normalize())
        else {
            return;
        };
        let pos_from_collider = collider_center + dir * r;
        let from_head = pos_from_collider - head_global_pos;
        *next_tail = if from_head.length_squared() <= f32::EPSILON {
            pos_from_collider
        } else {
            head_global_pos + from_head.normalize() * bone_length
        };
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
    use bevy::math::Vec3;
    use bevy::prelude::GlobalTransform;

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

    #[test]
    fn a_capsule_pushes_a_point_inside_it_out_along_the_segment_normal() {
        let capsule = ColliderShape::Capsule(Capsule {
            offset: [0.0, -1.0, 0.0],
            radius: 0.5,
            tail: [0.0, 1.0, 0.0],
        });
        let collider = GlobalTransform::default();
        let head_global_pos = Vec3::new(0.0, 0.0, 0.0);
        let mut next_tail = Vec3::new(0.2, 0.0, 0.0);

        capsule.apply_collision(&mut next_tail, &collider, head_global_pos, 0.0, 1.0);

        assert!(
            next_tail.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-6),
            "next_tail: {next_tail:?}"
        );
    }

    #[test]
    fn a_capsule_pushes_a_point_inside_near_its_end_out_through_the_hemisphere() {
        let capsule = ColliderShape::Capsule(Capsule {
            offset: [0.0, -1.0, 0.0],
            radius: 0.5,
            tail: [0.0, 1.0, 0.0],
        });
        let collider = GlobalTransform::default();
        let head_global_pos = Vec3::new(0.0, 0.0, 0.0);
        let mut next_tail = Vec3::new(0.1, 1.2, 0.0);

        capsule.apply_collision(&mut next_tail, &collider, head_global_pos, 0.0, 1.0);

        let delta = Vec3::new(0.1, 0.2, 0.0);
        let expected = head_global_pos
            + (Vec3::new(0.0, 1.0, 0.0) + delta.normalize() * 0.5 - head_global_pos).normalize()
                * 1.0;
        assert!(
            next_tail.abs_diff_eq(expected, 1e-6),
            "next_tail: {next_tail:?}"
        );
    }

    #[test]
    fn a_point_exactly_on_the_capsule_segment_gets_the_fallback_direction() {
        let capsule = ColliderShape::Capsule(Capsule {
            offset: [0.0, -1.0, 0.0],
            radius: 0.5,
            tail: [0.0, 1.0, 0.0],
        });
        let collider = GlobalTransform::default();
        let head_global_pos = Vec3::new(1.0, 0.0, 0.0);
        let mut next_tail = Vec3::new(0.0, 0.5, 0.0);

        capsule.apply_collision(&mut next_tail, &collider, head_global_pos, 0.0, 1.0);

        let fallback = (head_global_pos - Vec3::new(0.0, 0.5, 0.0)).normalize();
        let pos_from_collider = Vec3::new(0.0, 0.5, 0.0) + fallback * 0.5;
        let expected = head_global_pos + (pos_from_collider - head_global_pos).normalize() * 1.0;
        assert!(
            next_tail.abs_diff_eq(expected, 1e-6),
            "next_tail: {next_tail:?}"
        );
    }

    #[test]
    fn a_point_at_the_sphere_center_gets_the_fallback_direction() {
        let sphere = ColliderShape::Sphere(Sphere {
            offset: [0.0; 3],
            radius: 0.5,
        });
        let collider = GlobalTransform::default();
        let head_global_pos = Vec3::new(1.0, 0.0, 0.0);
        let mut next_tail = Vec3::new(0.0, 0.0, 0.0);

        sphere.apply_collision(&mut next_tail, &collider, head_global_pos, 0.0, 1.0);

        let fallback = (head_global_pos - Vec3::ZERO).normalize();
        let pos_from_collider = Vec3::ZERO + fallback * 0.5;
        let expected = head_global_pos + (pos_from_collider - head_global_pos).normalize() * 1.0;
        assert!(
            next_tail.abs_diff_eq(expected, 1e-6),
            "next_tail: {next_tail:?}"
        );
    }

    #[test]
    fn a_surface_point_coincident_with_the_head_stays_finite() {
        let sphere = ColliderShape::Sphere(Sphere {
            offset: [0.0; 3],
            radius: 0.5,
        });
        let collider = GlobalTransform::default();
        let head_global_pos = Vec3::new(0.5, 0.0, 0.0);
        let mut next_tail = Vec3::new(0.25, 0.0, 0.0);

        sphere.apply_collision(&mut next_tail, &collider, head_global_pos, 0.0, 1.0);

        assert!(next_tail.is_finite(), "next_tail: {next_tail:?}");
        assert!(
            next_tail.abs_diff_eq(head_global_pos, 1e-6),
            "next_tail: {next_tail:?}"
        );
    }

    #[test]
    fn a_capsule_touching_the_middle_of_a_bone_segment_pushes_its_tail_away() {
        let capsule = ColliderShape::Capsule(Capsule {
            offset: [-0.5, 0.5, 0.03],
            radius: 0.0,
            tail: [0.5, 0.5, 0.03],
        });
        let collider = GlobalTransform::default();
        let mut next_tail = Vec3::new(0.0, 1.0, 0.0);

        capsule.apply_collision(&mut next_tail, &collider, Vec3::ZERO, 0.05, 1.0);

        assert!(
            next_tail.z < -0.01 && next_tail.y > 0.99,
            "next_tail: {next_tail:?}"
        );
    }

    #[test]
    fn a_head_inside_contact_keeps_the_endpoint_behavior() {
        let sphere = ColliderShape::Sphere(Sphere {
            offset: [-0.03, -0.01, 0.0],
            radius: 0.0,
        });
        let collider = GlobalTransform::default();
        let mut next_tail = Vec3::new(0.0, 1.0, 0.0);

        sphere.apply_collision(&mut next_tail, &collider, Vec3::ZERO, 0.5, 1.0);

        assert!(
            next_tail.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6),
            "the head-side contact must fall back to the endpoint test: {next_tail:?}"
        );
    }
}
