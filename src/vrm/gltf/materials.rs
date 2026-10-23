use bevy::color::LinearRgba;
use bevy::prelude::Reflect;
use bevy::render::render_resource::ShaderType;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Reflect, Debug, Clone)]
pub struct VrmcMaterialsExtensitions {
    /// Indicates the version number of `VRMC_materials_mtoon` extension.
    ///
    /// The value is fixed to "1.0".
    #[serde(rename = "specVersion")]
    pub spec_version: String,
    #[serde(rename = "matcapFactor", default = "default_white_rgb")]
    pub matcap_factor: [f32; 3],
    #[serde(rename = "matcapTexture")]
    pub matcap_texture: Option<MatcapTexture>,
    #[serde(
        rename = "parametricRimFresnelPowerFactor",
        default = "default_parametric_rim_fresnel_power"
    )]
    pub parametric_rim_fresnel_power: f32,
    #[serde(rename = "rimMultiplyTexture")]
    pub rim_multiply_texture: Option<RimMultiplyTexture>,
    #[serde(rename = "outlineColorFactor", default = "default_black_rgb")]
    pub outline_color_factor: [f32; 3],
    #[serde(rename = "outlineLightingMixFactor", default = "default_one")]
    pub outline_lighting_mix_factor: f32,
    #[serde(rename = "outlineWidthFactor")]
    pub outline_width_factor: Option<f32>,
    #[serde(rename = "outlineWidthMultiplyTexture")]
    pub outline_width_multiply_texture: Option<OutlineWidthMultiplyTexture>,
    #[serde(rename = "outlineWidthMode", default = "default_outline_width_mode")]
    pub outline_width_mode: String,
    #[serde(rename = "parametricRimColorFactor", default = "default_black_rgb")]
    pub parametric_rim_color_factor: [f32; 3],
    #[serde(rename = "parametricRimLiftFactor", default = "default_zero")]
    pub parametric_rim_lift_factor: f32,
    #[serde(rename = "rimLightingMixFactor", default = "default_one")]
    pub rim_lighting_mix_factor: f32,
    /// The shade color.
    /// The value is evaluated in linear color space.
    #[serde(rename = "shadeColorFactor", default = "default_white_rgb")]
    pub shade_color_factor: [f32; 3],
    #[serde(rename = "shadeMultiplyTexture")]
    pub shade_multiply_texture: Option<VrmTexture>,
    #[serde(rename = "renderQueueOffsetNumber", default = "default_zero")]
    pub render_queue_offset_number: f32,
    #[serde(rename = "shadingShiftFactor", default = "default_zero")]
    pub shading_shift_factor: f32,
    #[serde(rename = "shadingShiftTexture")]
    pub shading_shift_texture: Option<ShadingShiftTexture>,
    #[serde(rename = "shadingToonyFactor", default = "default_toony_factor")]
    pub shading_toony_factor: f32,
    #[serde(rename = "transparentWithZWrite", default)]
    pub transparent_with_z_write: bool,
    #[serde(rename = "uvAnimationMaskTexture")]
    pub uv_animation_mask_texture: Option<UVAnimationMaskTexture>,
    #[serde(rename = "uvAnimationRotationSpeedFactor", default = "default_zero")]
    pub uv_animation_rotation_speed_factor: f32,
    #[serde(rename = "uvAnimationScrollXSpeedFactor", default = "default_zero")]
    pub uv_animation_scroll_x_speed_factor: f32,
    #[serde(rename = "uvAnimationScrollYSpeedFactor", default = "default_zero")]
    pub uv_animation_scroll_y_speed_factor: f32,
    #[serde(
        rename = "giEqualizationFactor",
        default = "default_gi_equalization_factor"
    )]
    pub gi_equalization_factor: f32,
}

fn default_white_rgb() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

fn default_black_rgb() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

fn default_zero() -> f32 {
    0.0
}

fn default_one() -> f32 {
    1.0
}

fn default_parametric_rim_fresnel_power() -> f32 {
    5.0
}

fn default_toony_factor() -> f32 {
    0.9
}

fn default_gi_equalization_factor() -> f32 {
    0.9
}

fn default_outline_width_mode() -> String {
    String::from("none")
}

impl VrmcMaterialsExtensitions {
    pub fn shade_color(&self) -> LinearRgba {
        let c = self.shade_color_factor;
        LinearRgba::rgb(c[0], c[1], c[2])
    }

    pub fn parametric_rim_color(&self) -> LinearRgba {
        let c = self.parametric_rim_color_factor;
        LinearRgba::rgb(c[0], c[1], c[2])
    }

    pub fn matcap_color(&self) -> LinearRgba {
        let c = self.matcap_factor;
        LinearRgba::rgb(c[0], c[1], c[2])
    }
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct MatcapTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct RimMultiplyTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct OutlineWidthMultiplyTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct UVAnimationMaskTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct ShadingShiftTexture {
    pub index: usize,
    #[serde(rename = "texCoord", default = "default_zero")]
    pub tex_coord: f32,
    #[serde(default = "default_one")]
    pub scale: f32,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct VrmTexture {
    #[serde(default)]
    pub extensions: Option<VrmTextureExtensions>,
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct VrmTextureExtensions {
    #[serde(rename = "KHR_texture_transform", default)]
    pub khr_texture_transform: Option<KhrTextureTransform>,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Reflect, Debug, Clone, PartialEq, Copy, ShaderType)]
#[serde(default)]
pub struct KhrTextureTransform {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
}

impl Default for KhrTextureTransform {
    fn default() -> Self {
        Self {
            offset: [0.0, 0.0],
            scale: [1.0, 1.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::success;
    use crate::tests::TestResult;
    use crate::vrm::gltf::materials::{KhrTextureTransform, VrmTexture, VrmcMaterialsExtensitions};

    #[test]
    fn deserialize_minimal_vrmc_materials_mtoon() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions = serde_json::from_str(r#"{"specVersion":"1.0"}"#)?;

        assert_eq!(mtoon.spec_version, "1.0");
        assert_eq!(mtoon.shade_color_factor, [1.0, 1.0, 1.0]);
        assert_eq!(mtoon.shading_shift_factor, 0.0);
        assert_eq!(mtoon.shading_toony_factor, 0.9);
        assert_eq!(mtoon.parametric_rim_color_factor, [0.0, 0.0, 0.0]);
        assert_eq!(mtoon.parametric_rim_fresnel_power, 5.0);
        assert_eq!(mtoon.parametric_rim_lift_factor, 0.0);
        assert_eq!(mtoon.rim_lighting_mix_factor, 1.0);
        assert_eq!(mtoon.matcap_factor, [1.0, 1.0, 1.0]);
        assert_eq!(mtoon.outline_width_mode, "none");
        assert_eq!(mtoon.outline_color_factor, [0.0, 0.0, 0.0]);
        assert_eq!(mtoon.outline_lighting_mix_factor, 1.0);
        assert_eq!(mtoon.outline_width_factor, None);
        assert_eq!(mtoon.uv_animation_rotation_speed_factor, 0.0);
        assert_eq!(mtoon.uv_animation_scroll_x_speed_factor, 0.0);
        assert_eq!(mtoon.uv_animation_scroll_y_speed_factor, 0.0);
        assert_eq!(mtoon.render_queue_offset_number, 0.0);
        assert!(!mtoon.transparent_with_z_write);
        assert_eq!(mtoon.gi_equalization_factor, 0.9);

        success!()
    }

    #[test]
    fn deserialize_vrmc_materials_mtoon_without_optional_textures() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions = serde_json::from_str(
            r#"{
                "specVersion": "1.0",
                "shadeColorFactor": [0.0, 0.0, 1.0],
                "giEqualizationFactor": 0.0,
                "shadingToonyFactor": 0.1
            }"#,
        )?;

        assert_eq!(mtoon.shade_color_factor, [0.0, 0.0, 1.0]);
        assert_eq!(mtoon.gi_equalization_factor, 0.0);
        assert_eq!(mtoon.shading_toony_factor, 0.1);

        assert!(mtoon.matcap_texture.is_none());
        assert!(mtoon.rim_multiply_texture.is_none());
        assert!(mtoon.shade_multiply_texture.is_none());
        assert!(mtoon.shading_shift_texture.is_none());
        assert!(mtoon.outline_width_multiply_texture.is_none());
        assert!(mtoon.uv_animation_mask_texture.is_none());

        success!()
    }

    #[test]
    fn deserialize_texture_without_khr_texture_transform() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions =
            serde_json::from_str(r#"{"specVersion":"1.0","shadeMultiplyTexture":{"index":1}}"#)?;
        let texture = mtoon
            .shade_multiply_texture
            .expect("shadeMultiplyTexture should be parsed");

        assert_eq!(texture.index, 1);
        assert!(texture.extensions.is_none());

        let texture: VrmTexture = serde_json::from_str(r#"{"index":0,"extensions":{}}"#)?;
        assert!(
            texture
                .extensions
                .is_some_and(|extensions| extensions.khr_texture_transform.is_none())
        );

        success!()
    }

    #[test]
    fn deserialize_texture_with_partial_khr_texture_transform() -> TestResult {
        let texture: VrmTexture = serde_json::from_str(
            r#"{"index":0,"extensions":{"KHR_texture_transform":{"scale":[2.0,2.0]}}}"#,
        )?;

        assert_eq!(
            texture
                .extensions
                .and_then(|extensions| extensions.khr_texture_transform),
            Some(KhrTextureTransform {
                offset: [0.0, 0.0],
                scale: [2.0, 2.0]
            })
        );

        success!()
    }

    #[test]
    fn deserialize_shading_shift_texture_without_scale() -> TestResult {
        let mtoon: VrmcMaterialsExtensitions =
            serde_json::from_str(r#"{"specVersion":"1.0","shadingShiftTexture":{"index":3}}"#)?;
        let texture = mtoon
            .shading_shift_texture
            .expect("shadingShiftTexture should be parsed");

        assert_eq!(texture.index, 3);
        assert_eq!(texture.tex_coord, 0.0);
        assert_eq!(texture.scale, 1.0);

        success!()
    }
}
