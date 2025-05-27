use crate::vrm::gltf::materials::VrmcMaterialsExtensitions;
use bevy::prelude::*;

#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(not(feature = "reflect"), derive(TypePath))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shade {
    /// The shade color.
    pub color: LinearRgba,
    /// The value to shift the shading boundary.
    pub shading_shift_factor: f32,
    /// The offset of the shading shift texture.
    pub texture_offset: f32,
    /// The scale of the shading shift texture.
    pub texture_scale: f32,
    /// The value to specify the smoothness of the shading boundary.
    pub toony_factor: f32,
}

impl From<&VrmcMaterialsExtensitions> for Shade {
    fn from(extension: &VrmcMaterialsExtensitions) -> Self {
        Self {
            color: extension.shade_color(),
            shading_shift_factor: extension.shading_shift_factor,
            texture_offset: extension
                .shading_shift_texture
                .as_ref()
                .map(|t| t.tex_coord)
                .unwrap_or(1.),
            texture_scale: extension
                .shading_shift_texture
                .as_ref()
                .map(|t| t.scale)
                .unwrap_or_default(),
            toony_factor: extension.shading_toony_factor,
        }
    }
}

impl Default for Shade {
    fn default() -> Self {
        Self {
            color: LinearRgba::BLACK,
            shading_shift_factor: 0.0,
            texture_offset: 0.0,
            texture_scale: 1.0,
            toony_factor: 0.9,
        }
    }
}
