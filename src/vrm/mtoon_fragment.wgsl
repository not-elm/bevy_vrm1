#import bevy_pbr::{
    forward_io::{
        VertexOutput,
        FragmentOutput,
    },
    pbr_fragment::pbr_input_from_vertex_output,
    pbr_types::PbrInput,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows::fetch_directional_shadow,
    ambient::ambient_light,
    mesh_view_bindings::{
        view,
        lights,
        globals,
    },
}
#import mtoon::types::{
    MToonInput,
    MToonMaterialUniform,
    material,
    base_color_texture,
    base_color_sampler,
    shading_shift_texture,
    shading_shift_texture_sampler,
    shade_multiply_texture,
    shade_multiply_texture_sampler,
    rim_multiply_texture,
    rim_multiply_sampler,
    uv_animation_mask_texture,
    uv_animation_mask_sampler,
    matcap_texture,
    matcap_sampler,
    emissive_texture,
    emissive_sampler,
    BASE_COLOR_TEXTURE,
    SHADING_SHIFT_TEXTURE,
    SHADE_MULTIPLY_TEXTURE,
    RIM_MAP_TEXTURE,
    UV_ANIMATION_MASK_TEXTURE,
    MATCAP_TEXTURE,
    EMISSIVE_TEXTURE,
    DOUBLE_SIDED,
    ALPHA_MODE_MASK,
    ALPHA_MODE_ALPHA_TO_COVERAGE,
    OUTLINE_WORLD_COORDINATES,
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
#ifdef OUTLINE_PASS
    // Currently, the outline only supports world coordinates.
    if((material.outline_flags & OUTLINE_WORLD_COORDINATES) == 0u) {
        discard;
    }
#endif

    var vertex_input = in;
    vertex_input.uv = calc_animated_uv((material.uv_transform * vec3(in.uv, 1.0)).xy);

    var out: FragmentOutput;
    var pbr_input = make_pbr_input(vertex_input, is_front);
    let mtoon_input = make_mtoon_input(vertex_input, pbr_input);
    out.color = apply_mtoon_lighting(mtoon_input);

#ifdef OUTLINE_PASS
    let outline_color = material.outline_color.rgb * mix(vec3(1.), out.color.rgb, material.outline_lighting_mix_factor);
    out.color = vec4(outline_color, material.base_color.a);
#endif

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
        let raw = base_color.a;
        let tmpAlpha = (raw - material.alpha_cutoff) / max(fwidth(raw), 0.00001) + 0.5;
        if(tmpAlpha < material.alpha_cutoff) {
            discard;
        }else{
            base_color.a = 1.0;
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
    let in_direct = apply_global_illumination(in);
    let emissive = apply_emissive_light(in);
    let rim = apply_rim_lighting(in.pbr, in.uv, direct, in_direct);
    return vec4<f32>(direct + in_direct + emissive + rim, in.lit_color.a);
}

fn apply_directional_lights(in: MToonInput) -> vec3<f32>{
    var direct: vec3<f32> = vec3(0.);
    var shade_color: vec3<f32> = calc_shade_color(in);
    var shading: f32 = 0.0;
    for (var i: u32 = 0u; i < lights.n_directional_lights; i = i + 1u) {
        if (lights.directional_lights[i].skip != 0u || (lights.directional_lights[i].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
            continue;
        }
        shading += calc_mtoon_lighting_shading(in, i);
    }
    return mix(shade_color, in.lit_color.rgb, shading);
}

fn calc_mtoon_lighting_shading(
    input: MToonInput,
    light_id: u32,
) -> f32 {
    let light = &lights.directional_lights[light_id];
    let light_color = (*light).color.rgb;
    let NdotL = saturate(dot(input.world_normal, (*light).direction_to_light));
    let shade_shift = calc_mtoon_lighting_reflectance_shading_shift(input);
#ifdef OUTLINE_PASS
    let shading = mtoon_linearstep(-1.0 + material.shading_toony_factor, 1.0 - material.shading_toony_factor, NdotL + shade_shift);
#else
    let shade_input = mix(-1., 1., mtoon_linearstep(-1., 1., NdotL));
    let view_z = dot(vec4<f32>(
        view.view_from_world[0].z,
        view.view_from_world[1].z,
        view.view_from_world[2].z,
        view.view_from_world[3].z
    ), input.world_position);
    var shadow = fetch_directional_shadow(
        light_id,
        input.world_position,
        input.world_normal,
        view_z,
    );
    let shading =  mtoon_linearstep(-1.0 + material.shading_toony_factor, 1.0 - material.shading_toony_factor, shade_input + shade_shift) * shadow;
#endif
   return shading;
}

fn calc_mtoon_lighting_reflectance_shading_shift(
    input: MToonInput,
) -> f32 {
    if((material.flags & SHADING_SHIFT_TEXTURE) != 0u) {
        return textureSampleBias(shading_shift_texture, shading_shift_texture_sampler, input.uv, view.mip_bias).r * material.shading_shift_texture_scale + material.shading_shift_factor;
    } else {
        return material.shading_shift_factor;
    }
}

//FIXME: This code is likely an incomplete implementation.
// https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#lighting
fn apply_global_illumination(
    in: MToonInput,
) -> vec3<f32> {
#ifdef OUTLINE_PASS
    return vec3(0.);
#else
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
    return view.exposure * in_direct_light;
#endif
}

fn calc_shade_color(in: MToonInput) -> vec3<f32>{
   let base_color = material.shade_color.rgb;
   if((material.flags & SHADE_MULTIPLY_TEXTURE) != 0u) {
       return base_color * textureSampleBias(shade_multiply_texture, shade_multiply_texture_sampler, in.uv, view.mip_bias).rgb;
   }else{
      return base_color;
   }
}

fn apply_emissive_light(in: MToonInput) -> vec3<f32> {
#ifdef OUTLINE_PASS
    return vec3(0.);
#else
    let emissive = in.pbr.material.emissive.rgb;
    if ((in.pbr.flags & EMISSIVE_TEXTURE) != 0u) {
        return emissive * textureSampleBias(emissive_texture, emissive_sampler, in.uv, view.mip_bias).rgb;
    } else {
        return emissive;
    }
#endif
}

fn apply_rim_lighting(in: PbrInput, uv: vec2<f32>, direct_light: vec3<f32>, in_direct: vec3<f32>) -> vec3<f32>{
    var rim = vec3(0.);
    let world_view_x = normalize(vec3<f32>(in.V.z, 0.0, -in.V.x));
    let world_view_y = cross(in.V, world_view_x);
    let matcap_uv = vec2<f32>(dot(world_view_x, in.N), dot(world_view_y, in.N)) * 0.495 + 0.5;
    let epsilon = 0.0001;
    if((material.flags & MATCAP_TEXTURE) != 0u) {
        rim = material.mat_cap_color.rgb * textureSampleBias(matcap_texture, matcap_sampler, matcap_uv, view.mip_bias).rgb;
    }

    let parametric_rim = saturate(1.0 - dot(in.N, in.V) + material.parametric_rim_lift_factor);
    rim += pow(parametric_rim, max(material.parametric_rim_fresnel_power, epsilon)) * material.parametric_rim_color.rgb;
    if((material.flags & RIM_MAP_TEXTURE) != 0u) {
        rim *= textureSampleBias(rim_multiply_texture, rim_multiply_sampler, uv, view.mip_bias).rgb;
    }
#ifdef OUTLINE_PASS
    rim *= mix(vec3(0.0), direct_light, material.rim_lighting_mix_factor);
#else
    rim *= mix(vec3(1.0), direct_light + in_direct, material.rim_lighting_mix_factor);
#endif
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
