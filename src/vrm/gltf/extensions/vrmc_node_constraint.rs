use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// `VRMC_node_constraint.*.weight` defaults to `1.0`.
fn default_weight() -> f32 {
    1.0
}

/// `VRMC_node_constraint.constraint.rotation`.
///
/// `source` is required; `weight` is optional and defaults to `1.0`.
#[derive(Serialize, Deserialize, Debug, Clone, Reflect)]
#[reflect(Serialize, Deserialize)]
pub struct Rotation {
    pub source: usize,
    #[serde(default = "default_weight")]
    pub weight: f32,
}

/// `VRMC_node_constraint.constraint.roll`.
///
/// `source` and `rollAxis` are both required. `rollAxis` is kept as a
/// `String` and not an enum: it is validated where it is consumed, because a
/// file with an unrecognised axis should lose that one constraint rather than
/// the whole node's `VRMC_node_constraint` extension. `weight` is optional and
/// defaults to `1.0`.
#[derive(Serialize, Deserialize, Debug, Clone, Reflect)]
#[reflect(Serialize, Deserialize)]
pub struct Roll {
    #[serde(rename = "rollAxis")]
    pub roll_axis: String,
    pub source: usize,
    #[serde(default = "default_weight")]
    pub weight: f32,
}

/// `VRMC_node_constraint.constraint.aim`.
///
/// `source` and `aimAxis` are both required. As with [`Roll::roll_axis`],
/// `aimAxis` is kept as a `String` and validated where it is consumed.
/// `weight` is optional and defaults to `1.0`.
#[derive(Serialize, Deserialize, Debug, Clone, Reflect)]
#[reflect(Serialize, Deserialize)]
pub struct Aim {
    #[serde(rename = "aimAxis")]
    pub aim_axis: String,
    pub source: usize,
    #[serde(default = "default_weight")]
    pub weight: f32,
}

/// `VRMC_node_constraint.constraint`.
///
/// The schema adds `"oneOf": [{"required": ["roll"]}, {"required": ["aim"]},
/// {"required": ["rotation"]}]`, so a valid object names exactly one
/// constraint. That is deliberately not enforced here: the three fields are
/// independent `Option`s and a node that carries none, or more than one, is
/// skipped when the constraint is applied instead of being rejected at load
/// time. `VRMC_node_constraint.specVersion` and `constraint` are both required
/// and are enforced.
#[derive(Serialize, Deserialize, Debug, Clone, Reflect)]
#[reflect(Serialize, Deserialize)]
pub struct Constraint {
    pub rotation: Option<Rotation>,
    pub roll: Option<Roll>,
    pub aim: Option<Aim>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Reflect)]
#[reflect(Serialize, Deserialize)]
pub struct VrmcNodeConstraint {
    pub constraint: Constraint,
    #[serde(rename = "specVersion")]
    pub spec_version: String,
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::extensions::vrmc_node_constraint::VrmcNodeConstraint;

    #[test]
    fn node_constraint_weights_default_to_one() -> TestResult {
        for json in [
            r#"{"specVersion":"1.0","constraint":{"rotation":{"source":1}}}"#,
            r#"{"specVersion":"1.0","constraint":{"roll":{"rollAxis":"X","source":1}}}"#,
            r#"{"specVersion":"1.0","constraint":{"aim":{"aimAxis":"PositiveY","source":1}}}"#,
        ] {
            let constraint: VrmcNodeConstraint = serde_json::from_str(json)?;

            assert_eq!(constraint.spec_version, "1.0");
            if let Some(rotation) = &constraint.constraint.rotation {
                assert_eq!(rotation.weight, 1.0);
                assert_eq!(rotation.source, 1);
            }
            if let Some(roll) = &constraint.constraint.roll {
                assert_eq!(roll.weight, 1.0);
                assert_eq!(roll.roll_axis, "X");
            }
            if let Some(aim) = &constraint.constraint.aim {
                assert_eq!(aim.weight, 1.0);
                assert_eq!(aim.aim_axis, "PositiveY");
            }
        }

        success!()
    }

    #[test]
    fn node_constraint_keeps_an_explicit_weight() -> TestResult {
        let constraint: VrmcNodeConstraint = serde_json::from_str(
            r#"{"specVersion":"1.0","constraint":{"rotation":{"source":1,"weight":0.5}}}"#,
        )?;

        assert_eq!(
            constraint.constraint.rotation.as_ref().map(|r| r.weight),
            Some(0.5)
        );

        success!()
    }

    #[test]
    fn reject_node_constraint_without_a_spec_version_or_constraint() {
        for invalid in [
            r#"{"constraint":{"rotation":{"source":1}}}"#,
            r#"{"specVersion":"1.0"}"#,
            r#"{"specVersion":"1.0","constraint":null}"#,
        ] {
            assert!(
                serde_json::from_str::<VrmcNodeConstraint>(invalid).is_err(),
                "`{invalid}` should be rejected"
            );
        }
    }

    #[test]
    fn reject_node_constraint_without_a_source_or_axis() {
        for invalid in [
            r#"{"specVersion":"1.0","constraint":{"rotation":{}}}"#,
            r#"{"specVersion":"1.0","constraint":{"rotation":{"weight":1.0}}}"#,
            r#"{"specVersion":"1.0","constraint":{"roll":{"source":1}}}"#,
            r#"{"specVersion":"1.0","constraint":{"roll":{"rollAxis":"X"}}}"#,
            r#"{"specVersion":"1.0","constraint":{"aim":{"source":1}}}"#,
            r#"{"specVersion":"1.0","constraint":{"aim":{"aimAxis":"PositiveY"}}}"#,
        ] {
            assert!(
                serde_json::from_str::<VrmcNodeConstraint>(invalid).is_err(),
                "`{invalid}` should be rejected"
            );
        }
    }
}
