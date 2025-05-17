use crate::vrm::gltf::materials::VrmcMaterialsExtensitions;
use bevy::prelude::*;

/// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#rim-lighting)
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(not(feature = "reflect"), derive(TypePath))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RimLighting {
    /// The color of the parametric rim lighting.
    pub color: LinearRgba,
    /// The fresnel power of the parametric rim lighting.
    pub fresnel_power: f32,
    /// The lift factor of the parametric rim lighting.
    pub lift_factor: f32,
    /// The color of the matcap texture.
    pub mat_cap_color: LinearRgba,
    /// The mix factor of the rim lighting.
    pub mix_factor: f32,
}

impl From<&VrmcMaterialsExtensitions> for RimLighting {
    fn from(extension: &VrmcMaterialsExtensitions) -> Self {
        Self {
            color: extension.parametric_rim_color(),
            mat_cap_color: extension.matcap_color(),
            lift_factor: extension.parametric_rim_lift_factor,
            fresnel_power: extension.parametric_rim_fresnel_power,
            mix_factor: extension.rim_lighting_mix_factor,
        }
    }
}

impl Default for RimLighting {
    fn default() -> Self {
        Self{
            color: LinearRgba::BLACK,
            fresnel_power: 5.0,
            lift_factor: 0.0,
            mat_cap_color: LinearRgba::WHITE,
            mix_factor: 1.0,
        }
    }
}