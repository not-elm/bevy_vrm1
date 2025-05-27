use crate::vrm::gltf::materials::VrmcMaterialsExtensitions;
use bevy::math::Vec2;
use bevy::prelude::*;

/// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#uv-animation)
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(not(feature = "reflect"), derive(TypePath))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UVAnimation {
    /// The unit is radians per second, so a value of 1.0 means the UV rotates once every 2π seconds.
    pub rotation_speed: f32,
    /// The speed of the UV scrolling in the X and Y directions.
    pub scroll_speed: Vec2,
}

impl From<&VrmcMaterialsExtensitions> for UVAnimation {
    fn from(extension: &VrmcMaterialsExtensitions) -> Self {
        Self {
            rotation_speed: extension.uv_animation_rotation_speed_factor,
            scroll_speed: Vec2::new(
                extension.uv_animation_scroll_x_speed_factor,
                extension.uv_animation_scroll_y_speed_factor,
            ),
        }
    }
}

impl Default for UVAnimation {
    fn default() -> Self {
        Self {
            rotation_speed: 0.0,
            scroll_speed: Vec2::ZERO,
        }
    }
}