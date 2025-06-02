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
    #[serde(rename = "matcapFactor")]
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
    #[serde(rename = "outlineColorFactor")]
    pub outline_color_factor: [f32; 3],
    #[serde(rename = "outlineLightingMixFactor")]
    pub outline_lighting_mix_factor: f32,
    #[serde(rename = "outlineWidthFactor")]
    pub outline_width_factor: Option<f32>,
    #[serde(rename = "outlineWidthMode")]
    pub outline_width_mode: String,
    #[serde(rename = "parametricRimColorFactor")]
    pub parametric_rim_color_factor: [f32; 3],
    #[serde(rename = "parametricRimLiftFactor")]
    pub parametric_rim_lift_factor: f32,
    #[serde(rename = "rimLightingMixFactor")]
    pub rim_lighting_mix_factor: f32,
    /// The shade color.
    /// The value is evaluated in linear color space.
    #[serde(rename = "shadeColorFactor")]
    pub shade_color_factor: [f32; 3],
    #[serde(rename = "shadeMultiplyTexture")]
    pub shade_multiply_texture: Option<VrmTexture>,
    #[serde(rename = "renderQueueOffsetNumber")]
    pub render_queue_offset_number: f32,
    #[serde(rename = "shadingShiftFactor")]
    pub shading_shift_factor: f32,
    #[serde(rename = "shadingShiftTexture")]
    pub shading_shift_texture: Option<ShadingShiftTexture>,
    #[serde(rename = "shadingToonyFactor")]
    pub shading_toony_factor: f32,
    #[serde(rename = "transparentWithZWrite")]
    pub transparent_with_z_write: bool,
    #[serde(rename = "uvAnimationMaskTexture")]
    pub uv_animation_mask_texture: Option<UVAnimationMaskTexture>,
    #[serde(rename = "uvAnimationRotationSpeedFactor")]
    pub uv_animation_rotation_speed_factor: f32,
    #[serde(rename = "uvAnimationScrollXSpeedFactor")]
    pub uv_animation_scroll_x_speed_factor: f32,
    #[serde(rename = "uvAnimationScrollYSpeedFactor")]
    pub uv_animation_scroll_y_speed_factor: f32,
    #[serde(rename = "giEqualizationFactor")]
    pub gi_equalization_factor: f32,
}

fn default_parametric_rim_fresnel_power() -> f32 {
    5.0
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
pub struct UVAnimationMaskTexture {
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct ShadingShiftTexture {
    pub index: usize,
    #[serde(rename = "texCoord")]
    pub tex_coord: f32,
    pub scale: f32,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct VrmTexture {
    pub extensions: VrmTextureExtensions,
    pub index: usize,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, Copy)]
pub struct VrmTextureExtensions {
    #[serde(rename = "KHR_texture_transform")]
    pub khr_texture_transform: KhrTextureTransform,
}

#[derive(Serialize, Deserialize, Reflect, Debug, Clone, PartialEq, Copy, ShaderType)]
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
