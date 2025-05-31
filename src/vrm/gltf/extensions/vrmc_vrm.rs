use crate::vrm::gltf::extensions::VrmNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct VrmcVrm {
    pub expressions: Option<Expressions>,
    // #[serde(rename = "firstPerson")]
    // pub first_person: Option<FirstPerson>,
    pub humanoid: Humanoid,
    #[serde(rename = "lookAt")]
    pub look_at: Option<LookAtProperties>,
    pub meta: Option<Meta>,
    #[serde(rename = "specVersion")]
    pub spec_version: String,
}

#[derive(Serialize, Deserialize)]
pub struct Expressions {
    pub preset: HashMap<String, VrmPreset>,
}

#[derive(Serialize, Deserialize)]
pub struct VrmPreset {
    /// If this value is `true`, `weight` value greater than 0.5 is 1.0, otherwise 0.0.
    #[serde(rename = "isBinary")]
    pub is_binary: bool,
    #[serde(rename = "morphTargetBinds")]
    pub morph_target_binds: Option<Vec<MorphTargetBind>>,
    #[serde(rename = "overrideBlink")]
    pub override_blink: String,
    #[serde(rename = "overrideLookAt")]
    pub override_look_at: String,
    #[serde(rename = "overrideMouth")]
    pub override_mouth: String,
}

#[derive(Serialize, Deserialize)]
pub struct MorphTargetBind {
    pub index: usize,
    pub node: usize,
    pub weight: f32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Humanoid {
    #[serde(rename = "humanBones")]
    pub human_bones: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize)]
pub struct Struct6 {
    pub node: i64,
    #[serde(rename = "type")]
    pub r#type: String,
}

#[derive(Serialize, Deserialize)]
pub struct FirstPerson {
    #[serde(rename = "meshAnnotations")]
    pub mesh_annotations: Vec<Struct6>,
}

#[derive(Serialize, Deserialize)]
pub struct Struct5 {
    #[serde(rename = "isBinary")]
    pub is_binary: bool,
    #[serde(rename = "overrideBlink")]
    pub override_blink: String,
    #[serde(rename = "overrideLookAt")]
    pub override_look_at: String,
    #[serde(rename = "overrideMouth")]
    pub override_mouth: String,
}

#[derive(Serialize, Deserialize)]
pub struct Meta {
    #[serde(rename = "allowAntisocialOrHateUsage")]
    pub allow_antisocial_or_hate_usage: bool,
    #[serde(rename = "allowExcessivelySexualUsage")]
    pub allow_excessively_sexual_usage: bool,
    #[serde(rename = "allowExcessivelyViolentUsage")]
    pub allow_excessively_violent_usage: bool,
    #[serde(rename = "allowPoliticalOrReligiousUsage")]
    pub allow_political_or_religious_usage: bool,
    #[serde(rename = "allowRedistribution")]
    pub allow_redistribution: bool,
    pub authors: Vec<String>,
    #[serde(rename = "avatarPermission")]
    pub avatar_permission: Option<String>,
    #[serde(rename = "commercialUsage")]
    pub commercial_usage: Option<String>,
    #[serde(rename = "creditNotation")]
    pub credit_notation: Option<String>,
    #[serde(rename = "licenseUrl")]
    pub license_url: Option<String>,
    pub modification: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "otherLicenseUrl")]
    pub other_license_url: Option<String>,
    #[serde(rename = "thumbnailImage")]
    pub thumbnail_image: Option<i64>,
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Component, Debug, Clone)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component, Serialize, Deserialize))]
pub struct LookAtProperties {
    /// An offset from the head bone to the lookAt reference position (between both eyes).
    #[serde(rename = "offsetFromHeadBone")]
    pub offset_from_head_bone: [f32; 3],
    /// A range map for the horizontal inner eye movement.
    #[serde(rename = "rangeMapHorizontalInner")]
    pub range_map_horizontal_inner: RangeMap,
    /// A range map for the horizontal outer eye movement (used by Expression's `LookLeft` and `LookRight`).
    #[serde(rename = "rangeMapHorizontalOuter")]
    pub range_map_horizontal_outer: RangeMap,
    /// A range map for the vertical down eye movement.
    #[serde(rename = "rangeMapVerticalDown")]
    pub range_map_vertical_down: RangeMap,
    /// A range map for the vertical up eye movement.
    #[serde(rename = "rangeMapVerticalUp")]
    pub range_map_vertical_up: RangeMap,
    /// `bone` or `expression` to look at.
    #[serde(rename = "type")]
    pub r#type: LookAtType,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(feature = "reflect", reflect(Serialize, Deserialize))]
pub struct RangeMap{
    #[serde(rename = "inputMaxValue")]
    pub input_max_value: f32,
    #[serde(rename = "outputScale")]
    pub output_scale: f32,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(feature = "reflect", reflect(Serialize, Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum LookAtType{
    Bone,
    Expression,
}
