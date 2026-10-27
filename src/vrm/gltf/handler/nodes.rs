//! `on_gltf_node`: the per-node components, while the node is still being
//! spawned.
//!
//! At this point the node's mesh entities already exist as its children (they are
//! spawned in `crates/bevy_gltf/src/loader/mod.rs` before the hook
//! runs), but the scene is not finished, so nothing here may
//! depend on a node that has not been visited yet. Node constraints are
//! therefore only *parsed* here and resolved in
//! [`scene::resolve_constraints`](super::scene).

use bevy::ecs::world::EntityWorldMut;
use bevy::math::Dir3;
use serde::Deserialize;

use super::VrmLoadState;
use super::root::EXT_NODE_CONSTRAINT;
use crate::error::vrm_warn;
use crate::prelude::{PendingNodeConstraint, VrmBone, VrmConstraintKind, VrmNodeIndex};

pub(crate) fn process_node(
    state: &mut VrmLoadState,
    gltf_node: &gltf::Node,
    entity: &mut EntityWorldMut,
) {
    let index = gltf_node.index();
    state.node_entities.insert(index, entity.id());
    entity.insert(VrmNodeIndex(index));

    if let Some(bone) = state
        .bone_nodes
        .iter()
        .filter(|(_, node)| **node == index)
        .map(|(bone, _)| bone.as_str())
        .min()
    {
        entity.insert(VrmBone::from(bone));
    }

    let Some(extensions) = gltf_node.extensions() else {
        return;
    };
    let Some(extension) = extensions.get(EXT_NODE_CONSTRAINT) else {
        return;
    };
    match serde_json::from_value::<NodeConstraintExt>(extension.clone()) {
        Ok(parsed) => {
            let Some((kind, source, weight)) = parsed.into_pending(index) else {
                return;
            };
            entity.insert(PendingNodeConstraint {
                source_node: source,
                weight,
                kind,
            });
        }
        Err(error) => {
            vrm_warn!(format!(
                "VRM: malformed `{EXT_NODE_CONSTRAINT}` on node {index}, skipping it: {error}"
            ));
        }
    }
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct NodeConstraintExt {
    constraint: ConstraintSpec,
}

/// Per the specification, exactly one of the three fields is set.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
struct ConstraintSpec {
    rotation: Option<RotationConstraint>,
    roll: Option<RollConstraint>,
    aim: Option<AimConstraint>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct RotationConstraint {
    source: usize,
    #[serde(default = "default_weight")]
    weight: f32,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct RollConstraint {
    source: usize,
    /// `X` | `Y` | `Z`
    roll_axis: String,
    #[serde(default = "default_weight")]
    weight: f32,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct AimConstraint {
    source: usize,
    /// `PositiveX` | `NegativeX` | …
    aim_axis: String,
    #[serde(default = "default_weight")]
    weight: f32,
}

/// `VRMC_node_constraint.*.weight` defaults to `1.0`.
fn default_weight() -> f32 {
    1.0
}

/// Parses a constraint axis into a typed [`Dir3`]; an invalid axis drops the
/// constraint with a warning.
fn parse_roll_axis(axis: &str) -> Option<Dir3> {
    match axis {
        "X" => Some(Dir3::X),
        "Y" => Some(Dir3::Y),
        "Z" => Some(Dir3::Z),
        _ => None,
    }
}

fn parse_aim_axis(axis: &str) -> Option<Dir3> {
    match axis {
        "PositiveX" => Some(Dir3::X),
        "NegativeX" => Some(Dir3::NEG_X),
        "PositiveY" => Some(Dir3::Y),
        "NegativeY" => Some(Dir3::NEG_Y),
        "PositiveZ" => Some(Dir3::Z),
        "NegativeZ" => Some(Dir3::NEG_Z),
        _ => None,
    }
}

impl NodeConstraintExt {
    fn into_pending(
        self,
        node_index: usize,
    ) -> Option<(VrmConstraintKind, usize, f32)> {
        let constraint = self.constraint;
        if let Some(rotation) = constraint.rotation {
            return Some((
                VrmConstraintKind::Rotation,
                rotation.source,
                rotation.weight,
            ));
        }
        if let Some(roll) = constraint.roll {
            let Some(roll_axis) = parse_roll_axis(&roll.roll_axis) else {
                vrm_warn!(format!(
                    "VRM node_constraint: invalid `rollAxis` `{}` on node {node_index}, \
                     skipping the constraint",
                    roll.roll_axis
                ));
                return None;
            };
            return Some((
                VrmConstraintKind::Roll { roll_axis },
                roll.source,
                roll.weight,
            ));
        }
        if let Some(aim) = constraint.aim {
            let Some(aim_axis) = parse_aim_axis(&aim.aim_axis) else {
                vrm_warn!(format!(
                    "VRM node_constraint: invalid `aimAxis` `{}` on node {node_index}, \
                     skipping the constraint",
                    aim.aim_axis
                ));
                return None;
            };
            return Some((VrmConstraintKind::Aim { aim_axis }, aim.source, aim.weight));
        }
        vrm_warn!(format!(
            "VRM: empty `{EXT_NODE_CONSTRAINT}` on node {node_index}, skipping it"
        ));
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;
    use bevy::prelude::*;

    fn app() -> App {
        crate::tests::test_app()
    }

    /// `gltf::Node` borrows the document it came from, so a real one has to be
    /// parsed rather than constructed.
    fn node_zero() -> gltf::Gltf {
        gltf::Gltf::from_slice_without_validation(
            br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"name":"Hips"}]}"#,
        )
        .expect("the fixture should parse")
    }

    #[test]
    fn a_node_gets_its_index_and_its_bone() -> TestResult {
        let mut app = app();
        let mut state = VrmLoadState {
            bone_nodes: [("hips".to_owned(), 0)].into_iter().collect(),
            ..Default::default()
        };
        let gltf = node_zero();

        let id = {
            let mut node = app.world_mut().spawn_empty();
            process_node(
                &mut state,
                &gltf.document.nodes().next().unwrap(),
                &mut node,
            );
            node.id()
        };

        assert_eq!(app.world().get::<VrmNodeIndex>(id), Some(&VrmNodeIndex(0)));
        assert_eq!(
            app.world().get::<VrmBone>(id).map(|bone| bone.0.as_str()),
            Some("hips")
        );
        assert_eq!(state.node_entities.get(&0), Some(&id));
        assert!(app.world().get::<PendingNodeConstraint>(id).is_none());
        success!()
    }

    /// `gltf::Node::default()` is not usable — it borrows a document — so the
    /// constraint parsing is exercised through its own entry point.
    #[test]
    fn constraint_axes_are_parsed_and_weight_defaults_to_one() -> TestResult {
        let extension: NodeConstraintExt =
            serde_json::from_str(r#"{"constraint":{"roll":{"source":2,"rollAxis":"Z"}}}"#)?;
        assert_eq!(
            extension.into_pending(7),
            Some((VrmConstraintKind::Roll { roll_axis: Dir3::Z }, 2, 1.0))
        );

        let extension: NodeConstraintExt = serde_json::from_str(
            r#"{"constraint":{"aim":{"source":2,"aimAxis":"NegativeY","weight":0.25}}}"#,
        )?;
        assert_eq!(
            extension.into_pending(7),
            Some((
                VrmConstraintKind::Aim {
                    aim_axis: Dir3::NEG_Y
                },
                2,
                0.25
            ))
        );

        let extension: NodeConstraintExt =
            serde_json::from_str(r#"{"constraint":{"rotation":{"source":9}}}"#)?;
        assert_eq!(
            extension.into_pending(7),
            Some((VrmConstraintKind::Rotation, 9, 1.0))
        );

        success!()
    }

    #[test]
    fn an_invalid_axis_drops_only_that_constraint() -> TestResult {
        let extension: NodeConstraintExt =
            serde_json::from_str(r#"{"constraint":{"aim":{"source":2,"aimAxis":"Sideways"}}}"#)?;
        assert_eq!(extension.into_pending(7), None);

        let extension: NodeConstraintExt =
            serde_json::from_str(r#"{"constraint":{"roll":{"source":2,"rollAxis":"W"}}}"#)?;
        assert_eq!(extension.into_pending(7), None);

        let extension: NodeConstraintExt = serde_json::from_str(r#"{"constraint":{}}"#)?;
        assert_eq!(extension.into_pending(7), None);

        success!()
    }
}
