//! This module defines the data structures for VRMA extensions in the GLTF format.

use crate::error::AppResult;
use crate::vrm::gltf::extensions::{VrmNode, obtain_extensions, obtain_vrmc_vrm};
use bevy::gltf::Gltf;
use bevy::platform::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct VrmaExpressions {
    pub preset: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct VrmaHumanoid {
    #[serde(rename = "humanBones")]
    pub human_bones: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VRMCVrmAnimation {
    #[serde(default)]
    pub expressions: Option<VrmaExpressions>,
    /// Optional per the `VRMC_vrm_animation` spec; a lookAt-only VRMA is valid.
    #[serde(default)]
    pub humanoid: Option<VrmaHumanoid>,
    #[serde(rename = "specVersion")]
    pub spec_version: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VrmaExtensions {
    #[serde(rename = "VRMC_vrm_animation")]
    pub vrmc_vrm_animation: VRMCVrmAnimation,
}

impl VrmaExtensions {
    pub fn new(json: &serde_json::map::Map<String, serde_json::Value>) -> AppResult<Self> {
        Ok(Self {
            vrmc_vrm_animation: serde_json::from_value(obtain_vrmc_vrm(json)?)?,
        })
    }

    pub fn from_gltf(gltf: &Gltf) -> AppResult<Self> {
        Self::new(obtain_extensions(gltf)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::success;
    use crate::tests::TestResult;

    #[test]
    fn deserialize_vrma_animation_with_only_a_spec_version() -> TestResult {
        let extensions: VrmaExtensions = serde_json::from_value(serde_json::json!({
            "VRMC_vrm_animation": { "specVersion": "1.0" }
        }))?;

        assert!(extensions.vrmc_vrm_animation.humanoid.is_none());
        assert!(extensions.vrmc_vrm_animation.expressions.is_none());
        success!()
    }

    #[test]
    fn deserialize_a_look_at_only_vrma_animation_without_humanoid() -> TestResult {
        let extensions: VrmaExtensions = serde_json::from_value(serde_json::json!({
            "VRMC_vrm_animation": {
                "specVersion": "1.0",
                "lookAt": { "type": "expression" }
            }
        }))?;

        assert!(extensions.vrmc_vrm_animation.humanoid.is_none());
        success!()
    }
}
