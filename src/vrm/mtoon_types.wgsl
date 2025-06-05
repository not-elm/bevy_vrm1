#define_import_path mtoon::types

#import bevy_pbr::pbr_types::PbrInput;

struct MToonMaterialUniform {
    flags: u32,
    base_color: vec4<f32>,
    shade_color: vec4<f32>,
    emissive_color: vec4<f32>,
    shading_shift_factor: f32,
    shading_shift_texture_offset: f32,
    shading_shift_texture_scale: f32,
    shading_toony_factor: f32,
    gi_equalization_factor: f32,
    uv_animation_rotation_speed: f32,
    uv_animation_scroll_speed_x: f32,
    uv_animation_rotation_speed_y: f32,
    uv_transform: mat3x3<f32>,
    mat_cap_color: vec4<f32>,
    parametric_rim_color: vec4<f32>,
    parametric_rim_lift_factor: f32,
    parametric_rim_fresnel_power: f32,
    rim_lighting_mix_factor: f32,
    alpha_cutoff: f32,
    outline_flags: u32,
    outline_color: vec4<f32>,
    outline_width_factor: f32,
    outline_lighting_mix_factor: f32,
}

struct MToonInput{
    pbr: PbrInput,
    uv: vec2<f32>,
    world_view_dir: vec3<f32>,
    world_position: vec4<f32>,
    world_normal: vec3<f32>,
    lit_color: vec4<f32>,
};

@group(2) @binding(100) var<uniform> material: MToonMaterialUniform;
@group(2) @binding(101) var base_color_texture: texture_2d<f32>;
@group(2) @binding(102) var base_color_sampler: sampler;
@group(2) @binding(103) var shading_shift_texture: texture_2d<f32>;
@group(2) @binding(104) var shading_shift_texture_sampler: sampler;
@group(2) @binding(105) var shade_multiply_texture: texture_2d<f32>;
@group(2) @binding(106) var shade_multiply_texture_sampler: sampler;
@group(2) @binding(107) var rim_multiply_texture: texture_2d<f32>;
@group(2) @binding(108) var rim_multiply_sampler: sampler;
@group(2) @binding(109) var uv_animation_mask_texture: texture_2d<f32>;
@group(2) @binding(110) var uv_animation_mask_sampler: sampler;
@group(2) @binding(111) var matcap_texture: texture_2d<f32>;
@group(2) @binding(112) var matcap_sampler: sampler;
@group(2) @binding(113) var emissive_texture: texture_2d<f32>;
@group(2) @binding(114) var emissive_sampler: sampler;
@group(2) @binding(115) var outline_width_multiply_texture: texture_2d<f32>;
@group(2) @binding(116) var outline_width_multiply_sampler: sampler;

const BASE_COLOR_TEXTURE: u32 = 1u;
const SHADING_SHIFT_TEXTURE: u32 = 2u;
const SHADE_MULTIPLY_TEXTURE: u32 = 4u;
const RIM_MAP_TEXTURE: u32 = 8u;
const UV_ANIMATION_MASK_TEXTURE: u32 = 16u;
const MATCAP_TEXTURE: u32 = 32u;
const EMISSIVE_TEXTURE = 64u;
const DOUBLE_SIDED: u32 = 128u;
const ALPHA_MODE_MASK: u32 = 256u;
const ALPHA_MODE_ALPHA_TO_COVERAGE: u32 = 512u;
const OUTLINE_WIDTH_MULTIPLY_TEXTURE: u32 = 1024u;

// Outline flags
const OUTLINE_WORLD_COORDINATES: u32 = 1u;