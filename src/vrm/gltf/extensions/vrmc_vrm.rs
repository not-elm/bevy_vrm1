use crate::vrm::gltf::extensions::VrmNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct VrmcVrm {
    pub expressions: Option<Expressions>,
    #[serde(rename = "firstPerson", default)]
    pub first_person: Option<FirstPerson>,
    pub humanoid: Humanoid,
    #[serde(rename = "lookAt")]
    pub look_at: Option<LookAtProperties>,
    pub meta: Option<Meta>,
    #[serde(rename = "specVersion")]
    pub spec_version: String,
}

#[derive(Serialize, Deserialize)]
pub struct Expressions {
    #[serde(default)]
    pub preset: HashMap<String, VrmPreset>,
    /// Read leniently: a malformed custom expression is dropped on its own
    /// instead of failing the whole `VRMC_vrm` parse, which would cost the
    /// avatar every other extension it declares.
    #[serde(default, deserialize_with = "lenient_map")]
    pub custom: HashMap<String, VrmPreset>,
}

#[derive(Serialize, Deserialize)]
pub struct VrmPreset {
    /// If this value is `true`, `weight` value greater than 0.5 is 1.0, otherwise 0.0.
    #[serde(rename = "isBinary", default)]
    pub is_binary: bool,
    #[serde(
        rename = "materialColorBinds",
        default,
        deserialize_with = "lenient_vec"
    )]
    pub material_color_binds: Vec<MaterialColorBind>,
    #[serde(rename = "morphTargetBinds")]
    pub morph_target_binds: Option<Vec<MorphTargetBind>>,
    #[serde(rename = "overrideBlink", default = "default_override_type")]
    pub override_blink: String,
    #[serde(rename = "overrideLookAt", default = "default_override_type")]
    pub override_look_at: String,
    #[serde(rename = "overrideMouth", default = "default_override_type")]
    pub override_mouth: String,
    #[serde(
        rename = "textureTransformBinds",
        default,
        deserialize_with = "lenient_vec"
    )]
    pub texture_transform_binds: Vec<TextureTransformBind>,
}

fn default_override_type() -> String {
    "none".to_string()
}

#[derive(Serialize, Deserialize)]
pub struct MorphTargetBind {
    pub index: usize,
    pub node: usize,
    pub weight: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MaterialColorBind {
    pub material: usize,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "targetValue")]
    pub target_value: [f32; 4],
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct TextureTransformBind {
    pub material: usize,
    #[serde(default)]
    pub offset: [f32; 2],
    #[serde(default = "default_uv_scale")]
    pub scale: [f32; 2],
}

fn default_uv_scale() -> [f32; 2] {
    [1.0, 1.0]
}

/// Reads a list, dropping the entries that do not deserialize. Anything that
/// is not a list at all (a `null`, say) reads as an empty one.
fn lenient_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let serde_json::Value::Array(values) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    Ok(values
        .into_iter()
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}

/// Reads a map, dropping the entries that do not deserialize. Anything that is
/// not a map at all reads as an empty one.
fn lenient_map<'de, D, T>(deserializer: D) -> Result<HashMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let serde_json::Value::Object(entries) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(HashMap::default());
    };
    Ok(entries
        .into_iter()
        .filter_map(|(name, value)| Some((name, serde_json::from_value(value).ok()?)))
        .collect())
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Humanoid {
    #[serde(rename = "humanBones")]
    pub human_bones: HashMap<String, VrmNode>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
#[serde(rename_all = "camelCase")]
pub enum FirstPersonFlag {
    #[default]
    Auto,
    Both,
    ThirdPersonOnly,
    FirstPersonOnly,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct MeshAnnotation {
    pub node: usize,
    #[serde(rename = "firstPersonFlag", alias = "type", default)]
    pub first_person_flag: FirstPersonFlag,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FirstPerson {
    #[serde(rename = "meshAnnotations", default)]
    pub mesh_annotations: Vec<MeshAnnotation>,
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
    #[serde(rename = "allowAntisocialOrHateUsage", default)]
    pub allow_antisocial_or_hate_usage: bool,
    #[serde(rename = "allowExcessivelySexualUsage", default)]
    pub allow_excessively_sexual_usage: bool,
    #[serde(rename = "allowExcessivelyViolentUsage", default)]
    pub allow_excessively_violent_usage: bool,
    #[serde(rename = "allowPoliticalOrReligiousUsage", default)]
    pub allow_political_or_religious_usage: bool,
    #[serde(rename = "allowRedistribution", default)]
    pub allow_redistribution: bool,
    #[serde(default)]
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

#[derive(Component, Debug, Clone, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Default)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct LookAtProperties {
    /// An offset from the head bone to the lookAt reference position (between both eyes).
    #[serde(rename = "offsetFromHeadBone", default)]
    pub offset_from_head_bone: [f32; 3],
    /// A range map for the horizontal inner eye movement.
    #[serde(rename = "rangeMapHorizontalInner", default)]
    pub range_map_horizontal_inner: RangeMap,
    /// A range map for the horizontal outer eye movement (used by Expression's `LookLeft` and `LookRight`).
    #[serde(rename = "rangeMapHorizontalOuter", default)]
    pub range_map_horizontal_outer: RangeMap,
    /// A range map for the vertical down eye movement.
    #[serde(rename = "rangeMapVerticalDown", default)]
    pub range_map_vertical_down: RangeMap,
    /// A range map for the vertical up eye movement.
    #[serde(rename = "rangeMapVerticalUp", default)]
    pub range_map_vertical_up: RangeMap,
    /// `bone` or `expression` to look at.
    #[serde(rename = "type", default)]
    pub r#type: LookAtType,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Reflect)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub struct RangeMap {
    #[serde(rename = "inputMaxValue", default = "default_input_max_value")]
    pub input_max_value: f32,
    #[serde(rename = "outputScale", default = "default_output_scale")]
    pub output_scale: f32,
}

impl Default for RangeMap {
    fn default() -> Self {
        Self {
            input_max_value: default_input_max_value(),
            output_scale: default_output_scale(),
        }
    }
}

fn default_input_max_value() -> f32 {
    90.0
}

fn default_output_scale() -> f32 {
    10.0
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Reflect, Default)]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
#[serde(rename_all = "snake_case")]
pub enum LookAtType {
    #[default]
    Bone,
    Expression,
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::extensions::vrmc_vrm::{
        Expressions, FirstPersonFlag, LookAtProperties, LookAtType, MeshAnnotation, Meta, RangeMap,
        TextureTransformBind, VrmPreset,
    };

    #[test]
    fn vrm_preset_tolerates_omitted_optional_fields() -> TestResult {
        let preset: VrmPreset = serde_json::from_str(r#"{}"#)?;

        assert!(!preset.is_binary);
        assert_eq!(preset.override_blink, "none");
        assert_eq!(preset.override_look_at, "none");
        assert_eq!(preset.override_mouth, "none");
        success!()
    }

    #[test]
    fn look_at_properties_tolerate_omitted_optional_fields() -> TestResult {
        let look_at: LookAtProperties = serde_json::from_str(r#"{}"#)?;

        assert_eq!(look_at.offset_from_head_bone, [0.0; 3]);
        assert_eq!(look_at.r#type, LookAtType::Bone);
        assert_eq!(look_at.range_map_horizontal_inner.input_max_value, 90.0);
        assert_eq!(look_at.range_map_horizontal_inner.output_scale, 10.0);
        assert_eq!(look_at.range_map_vertical_up, RangeMap::default());
        success!()
    }

    #[test]
    fn meta_tolerates_omitted_optional_fields() -> TestResult {
        let meta: Meta = serde_json::from_str(r#"{}"#)?;

        assert!(!meta.allow_redistribution);
        assert!(meta.authors.is_empty());
        success!()
    }

    #[test]
    fn expressions_model_the_color_and_texture_transform_binds() -> TestResult {
        let expressions: Expressions = serde_json::from_str(
            r#"{
                "preset": {
                    "happy": {
                        "isBinary": true,
                        "materialColorBinds": [
                            {"material": 1, "type": "emissionColor", "targetValue": [1, 0, 0, 1]}
                        ],
                        "morphTargetBinds": [{"index": 0, "node": 2, "weight": 1}],
                        "textureTransformBinds": [{"material": 1, "offset": [0.5, 0.5]}]
                    }
                },
                "custom": {
                    "Wave": {
                        "materialColorBinds": [
                            {"material": 3, "type": "outlineColor", "targetValue": [0, 0, 1, 1]}
                        ]
                    }
                }
            }"#,
        )?;

        let happy = &expressions.preset["happy"];
        assert!(happy.is_binary);
        assert_eq!(happy.material_color_binds.len(), 1);
        assert_eq!(happy.material_color_binds[0].material, 1);
        assert_eq!(happy.material_color_binds[0].r#type, "emissionColor");
        assert_eq!(
            happy.material_color_binds[0].target_value,
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            happy.texture_transform_binds,
            [TextureTransformBind {
                material: 1,
                offset: [0.5, 0.5],
                scale: [1.0, 1.0],
            }]
        );

        let wave = &expressions.custom["Wave"];
        assert_eq!(wave.material_color_binds.len(), 1);
        assert_eq!(wave.material_color_binds[0].r#type, "outlineColor");

        success!()
    }

    /// `custom` and the two bind lists are parsed by this crate since v0.11.0; a
    /// file that got one of them wrong used to load, because they were ignored,
    /// and still has to.
    #[test]
    fn malformed_custom_expressions_and_null_bind_lists_are_tolerated() -> TestResult {
        let expressions: Expressions = serde_json::from_str(
            r#"{
                "preset": {
                    "happy": {
                        "materialColorBinds": null,
                        "textureTransformBinds": {},
                        "morphTargetBinds": [{"index": 0, "node": 2, "weight": 1}]
                    }
                },
                "custom": {
                    "Broken": {"isBinary": null},
                    "Wave": {"morphTargetBinds": [{"index": 1, "node": 2, "weight": 1}]}
                }
            }"#,
        )?;

        let happy = &expressions.preset["happy"];
        assert!(happy.material_color_binds.is_empty());
        assert!(happy.texture_transform_binds.is_empty());
        assert!(!expressions.custom.contains_key("Broken"));
        assert!(expressions.custom.contains_key("Wave"));
        success!()
    }

    #[test]
    fn malformed_bind_does_not_reject_the_whole_expression() -> TestResult {
        let expressions: Expressions = serde_json::from_str(
            r#"{
                "preset": {
                    "happy": {
                        "materialColorBinds": [{"material": 1, "type": "color"}],
                        "morphTargetBinds": [{"index": 0, "node": 2, "weight": 1}]
                    }
                }
            }"#,
        )?;

        let happy = &expressions.preset["happy"];
        assert!(happy.material_color_binds.is_empty());
        assert_eq!(happy.morph_target_binds.as_ref().map(Vec::len), Some(1));

        success!()
    }

    #[test]
    fn mesh_annotation_accepts_the_schema_and_the_document_property_name() -> TestResult {
        for json in [
            r#"{"node":1,"firstPersonFlag":"thirdPersonOnly"}"#,
            r#"{"node":1,"type":"thirdPersonOnly"}"#,
        ] {
            let annotation: MeshAnnotation = serde_json::from_str(json)?;
            assert_eq!(annotation.node, 1);
            assert_eq!(
                annotation.first_person_flag,
                FirstPersonFlag::ThirdPersonOnly
            );
        }

        success!()
    }
}
