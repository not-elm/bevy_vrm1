use crate::prelude::*;
use bevy::asset::Asset;
use bevy::color::LinearRgba;
use bevy::prelude::{Component, Reflect, TypePath};
use bevy::render::extract_component::ExtractComponent;

#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(not(feature = "reflect"), derive(TypePath))]
#[derive(Asset, PartialEq, Debug, Clone, Component, ExtractComponent, Default)]
pub struct MToonOutline {
    pub mode: OutlineWidthMode,
    /// The outline width
    ///
    /// the unit is in meters.
    pub width_factor: f32,

    pub color: LinearRgba,

    pub lighting_mix_factor: f32,
}

impl From<&VrmcMaterialsExtensitions> for MToonOutline {
    fn from(value: &VrmcMaterialsExtensitions) -> Self {
        let color = value.outline_color_factor;
        Self {
            mode: match value.outline_width_mode.as_str() {
                "worldCoordinates" => OutlineWidthMode::WorldCoordinates,
                _ => OutlineWidthMode::None,
            },
            width_factor: value.outline_width_factor.unwrap_or_default(),
            lighting_mix_factor: value.outline_lighting_mix_factor,
            color: LinearRgba::rgb(color[0], color[1], color[2]),
        }
    }
}

#[derive(Reflect, Debug, Clone, Default, Copy, PartialEq, Eq, Hash)]
pub enum OutlineWidthMode {
    /// The outline will not be drawn.
    #[default]
    None,
    /// The outline width is determined by the distance in world coordinates.
    WorldCoordinates,
    // TODO: Not supported yet
    // ScreenCoordinates,
}