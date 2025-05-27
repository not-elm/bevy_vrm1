#import bevy_pbr::pbr_types::PbrInput;
#import bevy_pbr::ambient::ambient_light;
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_vertex_output,
    pbr_types,
    pbr_types::{
        STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
        STANDARD_MATERIAL_FLAGS_ALPHA_MODE_RESERVED_BITS,
        STANDARD_MATERIAL_FLAGS_ALPHA_MODE_OPAQUE,
    },
    pbr_bindings,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    decal::clustered::apply_decal_base_color,
    mesh_view_bindings::{
        view,
        lights,
    },
    mesh_view_bindings as view_bindings,
    mesh_view_bindings::globals,
    mesh_view_types,
    lighting,
    lighting::{LAYER_BASE, LAYER_CLEARCOAT},
    transmission,
    clustered_forward as clustering,
    shadows,
    ambient,
    irradiance_volume,
    mesh_types::{MESH_FLAGS_SHADOW_RECEIVER_BIT, MESH_FLAGS_TRANSMITTED_SHADOW_RECEIVER_BIT},
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{forward_io::{VertexOutput, FragmentOutput}}
#endif

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
    mat_cap_color: vec4<f32>,
    parametric_rim_color: vec4<f32>,
    parametric_rim_lift_factor: f32,
    parametric_rim_fresnel_power: f32,
    rim_lighting_mix_factor: f32,
    alpha_cutoff: f32,
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

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let vertex_input = VertexOutput(
        in.position,
        in.world_position,
        in.world_normal,
#ifdef VERTEX_UVS_A
        calc_animated_uv(in.uv),
#endif
#ifdef VERTEX_UVS_B
        in.uv_b,
#endif
#ifdef VERTEX_TANGENTS
        in.world_tangent,
#endif
#ifdef VERTEX_COLORS
        in.color,
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
        in.instance_index,
#endif
#ifdef VISIBILITY_RANGE_DITHER
        in.visibility_range_dither,
#endif
    );

    var out: FragmentOutput;
    var pbr_input = make_pbr_input(vertex_input, is_front);
    let mtoon_input = make_mtoon_input(vertex_input, pbr_input);
    out.color = apply_mtoon_lighting(mtoon_input);

    return out;
}

fn make_pbr_input(
    vertex_input: VertexOutput,
    is_front: bool,
) -> PbrInput{
    let double_sided = (material.flags & DOUBLE_SIDED) != 0;
    var pbr_input = pbr_input_from_vertex_output(vertex_input, is_front, double_sided);
    pbr_input.material.base_color = lit_color(vertex_input.uv);
    pbr_input.material.metallic = 0.0;
    pbr_input.material.emissive = material.emissive_color;
    return pbr_input;
}

fn lit_color(uv: vec2<f32>) -> vec4<f32> {
    var base_color = material.base_color;
    if((material.flags & BASE_COLOR_TEXTURE) != 0u) {
        base_color *= textureSampleBias(base_color_texture, base_color_sampler, uv, view.mip_bias);
    }
    if((material.flags & ALPHA_MODE_MASK) != 0u || (material.flags & ALPHA_MODE_ALPHA_TO_COVERAGE) != 0u) {
        if(base_color.a >= material.alpha_cutoff) {
            base_color.a = 1.0;
        } else {
            discard;
        }
    }
    return base_color;
}

fn make_mtoon_input(in: VertexOutput, pbr_input: PbrInput) -> MToonInput{
    let uv = in.uv;
    return MToonInput(
        pbr_input,
        uv,
        pbr_input.V,
        in.world_position,
        pbr_input.N,
        pbr_input.material.base_color,
    );
}

fn calc_animated_uv(uv: vec2<f32>) -> vec2<f32>{
    let time = calc_uv_time(uv);
    let translate = time * vec2(material.uv_animation_scroll_speed_x, material.uv_animation_rotation_speed_y);
    let rotate_rad = fract(time * material.uv_animation_rotation_speed);
    let cos_rotate = cos(rotate_rad);
    let sin_rotate = sin(rotate_rad);
    let pivot = vec2<f32>(0.5, 0.5);
    return mat2x2(cos_rotate, -sin_rotate, sin_rotate, cos_rotate) * (uv - pivot) + pivot + translate;
}

fn calc_uv_time(uv: vec2<f32>) -> f32{
    if((material.flags & UV_ANIMATION_MASK_TEXTURE) != 0u) {
        // I referred to MToon's implementation, but I don't know why this works. ⊂二二二（　＾ω＾）二二⊃
        let mask = textureSample(uv_animation_mask_texture, uv_animation_mask_sampler, uv).b;
        return mask * globals.time;
    }else{
        return globals.time;
    }
}

fn apply_mtoon_lighting(in: MToonInput) -> vec4<f32> {
    let direct = apply_directional_lights(in);
    let in_direct = calc_global_illumination(in);
    let emissive = apply_emissive_light(in);
    let rim = apply_rim_lighting(in.pbr, in.uv, direct);
    return vec4<f32>(direct + emissive + rim, in.lit_color.a);
}

fn apply_directional_lights(in: MToonInput) -> vec3<f32>{
    var direct: vec3<f32> = vec3(0.);
    for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
        if (lights.directional_lights[i].skip != 0u) {
            continue;
        }
        direct += calc_directional_light_color(in, i);
    }
    return direct;
}

fn calc_directional_light_color(
    in: MToonInput,
    light_id: u32,
) -> vec3<f32> {
    if((lights.directional_lights[light_id].flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u){
        let shading = calc_mtoon_lighting_shading(in, light_id);
        let base_color_term = in.lit_color.rgb;
        let shade_color_term = calc_shade_color_term(in);
        return mix(shade_color_term, base_color_term, shading);
    }else{
        return in.lit_color.rgb;
    }
}

fn calc_mtoon_lighting_shading(
    input: MToonInput,
    light_id: u32,
) -> f32 {
    let light = &lights.directional_lights[light_id];
    let NdotL = saturate(dot(normalize(input.world_normal), normalize((*light).direction_to_light)));
    let shade_shift = calc_mtoon_lighting_reflectance_shading_shift(input);
    let shade_input = mix(-1., 1., mtoon_linearstep(-1., 1., NdotL));
    return mtoon_linearstep(-1.0 + material.shading_toony_factor, 1.0 - material.shading_toony_factor, shade_input + shade_shift);
}

fn calc_mtoon_lighting_reflectance_shading_shift(
    input: MToonInput,
) -> f32 {
    if((material.flags & SHADING_SHIFT_TEXTURE) != 0u) {
        return textureSample(shading_shift_texture, shading_shift_texture_sampler, input.uv).r * material.shading_shift_texture_scale + material.shading_shift_factor;
    } else {
        return material.shading_shift_factor;
    }
}

fn calc_global_illumination(
    in: MToonInput,
) -> vec3<f32> {
    let base_color = in.lit_color.rgb;
    let diffuse_color = calc_diffuse_color(
        base_color,
        in.pbr.material.diffuse_transmission,
    );
    let in_direct_light = ambient_light(
        in.world_position,
        in.world_normal,
        in.world_view_dir,
        dot(in.world_normal, in.world_view_dir),
        diffuse_color,
        // Is the reflection color unnecessary?
        vec3(0.),
        in.pbr.material.perceptual_roughness,
        in.pbr.diffuse_occlusion,
    );
//    let up = vec3<f32>(0.0, 1.0, 0.0);
//    let down = vec3<f32>(0.0, -1.0, 0.0);
//    //TODO: ちゃんと均一化する
//    let uniformed_gi = (up + down) * 0.5;
    return in_direct_light;
}

fn calc_shade_color_term(in: MToonInput) -> vec3<f32>{
   let base_color = material.shade_color.rgb;
   if((material.flags & SHADE_MULTIPLY_TEXTURE) != 0u) {
       return base_color * textureSample(shade_multiply_texture, shade_multiply_texture_sampler, in.uv).rgb;
   }else{
      return base_color;
   }
}

fn apply_emissive_light(in: MToonInput) -> vec3<f32> {
    let emissive = in.pbr.material.emissive.rgb;
    if ((in.pbr.flags & EMISSIVE_TEXTURE) != 0u) {
        return emissive * textureSampleBias(emissive_texture, emissive_sampler, in.uv, view.mip_bias).rgb;
    } else {
        return emissive;
    }
}

fn apply_rim_lighting(in: PbrInput, uv: vec2<f32>, direct_light: vec3<f32>) -> vec3<f32>{
    var rim = material.mat_cap_color.rgb;
    let world_view_x = normalize(vec3<f32>(in.V.z, 0.0, -in.V.x));
    let world_view_y = cross(in.V, world_view_x);
    let matcap_uv = vec2<f32>(dot(world_view_x, in.N), dot(world_view_y, in.N)) * 0.495 + 0.5;
    let epsilon = 0.0001;
    if((material.flags & MATCAP_TEXTURE) != 0u) {
        rim *= textureSampleBias(matcap_texture, matcap_sampler, matcap_uv, view.mip_bias).rgb;
    }

    let parametric_rim = saturate(1.0 - dot(in.N, in.V) + material.parametric_rim_lift_factor);
    rim += pow(parametric_rim, max(material.parametric_rim_fresnel_power, epsilon)) * material.parametric_rim_color.rgb;
    if((material.flags & RIM_MAP_TEXTURE) != 0u) {
        rim *= textureSampleBias(rim_multiply_texture, rim_multiply_sampler, uv, view.mip_bias).rgb;
    }
    rim *= mix(vec3(1.0), direct_light, material.rim_lighting_mix_factor);
    return rim;
}

fn mtoon_linearstep(a: f32, b: f32, t: f32) -> f32 {
    return saturate((t - a) / (b - a));
}

fn calc_diffuse_color(
    base_color: vec3<f32>,
    diffuse_transmission: f32
) -> vec3<f32> {
    return base_color * (1.0 - diffuse_transmission);
}
