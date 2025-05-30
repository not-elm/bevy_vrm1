mod rim_lighting;
mod shade;
mod uv_animation;

use crate::vrm::mtoon::MTOON_SHADER_HANDLE;
use bevy::math::Affine2;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey, OpaqueRendererMethod};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, AsBindGroupShaderType, Face, RenderPipelineDescriptor, ShaderRef, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::render::texture::GpuImage;
use bitflags::bitflags;
pub use rim_lighting::RimLighting;
pub use shade::Shade;
pub use uv_animation::UVAnimation;

/// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md)
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component, Default))]
#[cfg_attr(not(feature = "reflect"), derive(TypePath))]
#[derive(Asset, AsBindGroup, PartialEq, Debug, Clone, Component)]
#[data(100, MToonMaterialUniform)]
#[bind_group_data(MToonMaterialKey)]
pub struct MToonMaterial {
    /// The texture that defines the base color of the material.
    #[texture(101)]
    #[sampler(102)]
    #[dependency]
    pub base_color_texture: Option<Handle<Image>>,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#shadingshifttexture)
    #[texture(103)]
    #[sampler(104)]
    #[dependency]
    pub shading_shift_texture: Option<Handle<Image>>,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#shademultiplytexture)
    #[texture(105)]
    #[sampler(106)]
    #[dependency]
    pub shade_multiply_texture: Option<Handle<Image>>,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#rim-multiply-texture)
    #[texture(107)]
    #[sampler(108)]
    #[dependency]
    pub rim_multiply_texture: Option<Handle<Image>>,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#uv-animation-mask-texture)
    #[texture(109)]
    #[sampler(110)]
    #[dependency]
    pub uv_animation_mask_texture: Option<Handle<Image>>,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#matcaptexture)
    #[texture(111)]
    #[sampler(112)]
    #[dependency]
    pub matcap_texture: Option<Handle<Image>>,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#emission)
    #[texture(113)]
    #[sampler(114)]
    #[dependency]
    pub emissive_texture: Option<Handle<Image>>,
    pub uv_animation: UVAnimation,
    pub uv_transform: Affine2,
    pub rim_lighting: RimLighting,
    pub shade: Shade,
    pub base_color: Color,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#emission)
    pub emissive: LinearRgba,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#giequalizationfactor)
    pub gi_equalization_factor: f32,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
    /// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#renderqueueoffsetnumber)
    pub depth_bias: f32,
    pub opaque_renderer_method: OpaqueRendererMethod,
    #[cfg_attr(feature = "reflect", reflect(ignore, clone))]
    pub cull_mode: Option<Face>,
}

bitflags! {
    /// The pipeline key for `StandardMaterial`, packed into 64 bits.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub struct MToonMaterialKey: u64 {
        const CULL_FRONT = 1 << 0;
        const CULL_BACK = 1 << 1;
    }
}

impl Material for MToonMaterial {
    fn fragment_shader() -> ShaderRef {
        MTOON_SHADER_HANDLE.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn opaque_render_method(&self) -> OpaqueRendererMethod {
        self.opaque_renderer_method
    }

    fn depth_bias(&self) -> f32 {
        self.depth_bias
    }

    fn specialize(
        _pipeline: &MaterialPipeline<Self>,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode =
            if key.bind_group_data.contains(MToonMaterialKey::CULL_FRONT) {
                Some(Face::Front)
            } else if key.bind_group_data.contains(MToonMaterialKey::CULL_BACK) {
                Some(Face::Back)
            } else {
                None
            };
        Ok(())
    }
}

impl From<&MToonMaterial> for MToonMaterialKey {
    fn from(material: &MToonMaterial) -> Self {
        let mut key = MToonMaterialKey::empty();
        key.set(
            MToonMaterialKey::CULL_FRONT,
            material.cull_mode == Some(Face::Front),
        );
        key.set(
            MToonMaterialKey::CULL_BACK,
            material.cull_mode == Some(Face::Back),
        );
        key
    }
}

impl Default for MToonMaterial {
    fn default() -> Self {
        Self {
            base_color_texture: None,
            shading_shift_texture: None,
            shade_multiply_texture: None,
            rim_multiply_texture: None,
            uv_animation_mask_texture: None,
            matcap_texture: None,
            emissive_texture: None,
            uv_animation: UVAnimation::default(),
            uv_transform: Affine2::IDENTITY,
            rim_lighting: RimLighting::default(),
            shade: Shade::default(),
            base_color: Color::WHITE,
            emissive: LinearRgba::BLACK,
            gi_equalization_factor: 0.9,
            alpha_mode: AlphaMode::default(),
            double_sided: false,
            depth_bias: 0.0,
            opaque_renderer_method: OpaqueRendererMethod::default(),
            cull_mode: None,
        }
    }
}

bitflags::bitflags! {
    #[repr(transparent)]
    pub struct MtoonFlags: u32 {
        const BASE_COLOR_TEXTURE = 1 << 0;
        const SHADING_SHIFT_TEXTURE = 1 << 1;
        const SHADE_MULTIPLY_TEXTURE = 1 << 2;
        const RIM_MAP_TEXTURE = 1 << 3;
        const UV_ANIMATION_MASK_TEXTURE = 1 << 4;
        const MATCAP_TEXTURE = 1 << 5;
        const EMISSIVE_TEXTURE = 1 << 6;
        const DOUBLE_SIDED = 1 << 7;
        const ALPHA_MODE_MASK = 1 << 8;
        const ALPHA_MODE_ALPHA_TO_COVERAGE = 1 << 9;
    }
}

#[derive(Clone, Default, ShaderType)]
pub struct MToonMaterialUniform {
    pub flags: u32,
    pub base_color: Vec4,
    pub shade_color: Vec4,
    pub emissive_color: Vec4,
    pub shading_shift_factor: f32,
    pub shading_shift_texture_offset: f32,
    pub shading_shift_texture_scale: f32,
    pub shading_shift_toony_factor: f32,
    pub gi_equalization_factor: f32,
    pub uv_animation_rotation_speed: f32,
    pub uv_animation_scroll_speed_x: f32,
    pub uv_animation_scroll_speed_y: f32,
    pub uv_transform: Mat3,
    pub mat_cap_color: Vec4,
    pub parametric_rim_color: Vec4,
    pub parametric_rim_lift_factor: f32,
    pub parametric_rim_fresnel_power: f32,
    pub rim_lighting_mix_factor: f32,
    pub alpha_cutoff: f32,
}

impl AsBindGroupShaderType<MToonMaterialUniform> for MToonMaterial {
    fn as_bind_group_shader_type(
        &self,
        _images: &RenderAssets<GpuImage>,
    ) -> MToonMaterialUniform {
        let mut flags = MtoonFlags::empty();
        flags.set(
            MtoonFlags::BASE_COLOR_TEXTURE,
            self.base_color_texture.is_some(),
        );
        flags.set(MtoonFlags::DOUBLE_SIDED, self.double_sided);
        flags.set(
            MtoonFlags::SHADING_SHIFT_TEXTURE,
            self.shading_shift_texture.is_some(),
        );
        flags.set(
            MtoonFlags::SHADE_MULTIPLY_TEXTURE,
            self.shade_multiply_texture.is_some(),
        );
        flags.set(
            MtoonFlags::RIM_MAP_TEXTURE,
            self.rim_multiply_texture.is_some(),
        );
        flags.set(
            MtoonFlags::UV_ANIMATION_MASK_TEXTURE,
            self.uv_animation_mask_texture.is_some(),
        );
        flags.set(MtoonFlags::MATCAP_TEXTURE, self.matcap_texture.is_some());
        flags.set(
            MtoonFlags::ALPHA_MODE_MASK,
            matches!(self.alpha_mode, AlphaMode::Mask(_)),
        );
        flags.set(
            MtoonFlags::ALPHA_MODE_ALPHA_TO_COVERAGE,
            matches!(self.alpha_mode, AlphaMode::AlphaToCoverage),
        );
        MToonMaterialUniform {
            flags: flags.bits(),
            shade_color: self.shade.color.to_vec4(),
            shading_shift_factor: self.shade.shading_shift_factor,
            shading_shift_texture_offset: self.shade.texture_offset,
            shading_shift_texture_scale: self.shade.texture_scale,
            shading_shift_toony_factor: self.shade.toony_factor,
            gi_equalization_factor: self.gi_equalization_factor,
            uv_animation_rotation_speed: self.uv_animation.rotation_speed,
            uv_animation_scroll_speed_x: self.uv_animation.scroll_speed.x,
            uv_animation_scroll_speed_y: self.uv_animation.scroll_speed.y,
            uv_transform: self.uv_transform.into(),
            mat_cap_color: self.rim_lighting.mat_cap_color.to_vec4(),
            parametric_rim_color: self.rim_lighting.color.to_vec4(),
            parametric_rim_lift_factor: self.rim_lighting.lift_factor,
            parametric_rim_fresnel_power: self.rim_lighting.fresnel_power,
            rim_lighting_mix_factor: self.rim_lighting.mix_factor,
            base_color: self.base_color.to_linear().to_vec4(),
            emissive_color: self.emissive.to_vec4(),
            alpha_cutoff: match self.alpha_mode {
                AlphaMode::Mask(value) => value,
                _ => 0.5,
            },
        }
    }
}
