import bevy_pbr::render::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph::{morph_position, morph_normal, morph_tangent},
    forward_io::{Vertex, UncompressedVertex, VertexOutput, decompress_vertex},
    view_transformations::position_world_to_clip,
    mesh_view_bindings::globals,
};
import package::vrm::mtoon_types::{
    MToonMaterialUniform,
    material,
    outline_width_multiply_texture,
    outline_width_multiply_sampler,
    OUTLINE_WIDTH_MULTIPLY_TEXTURE,
    uv_animation_mask_texture,
    uv_animation_mask_sampler,
    UV_ANIMATION_MASK_TEXTURE,
};

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let uncompressed_vertex_no_morph = decompress_vertex(vertex_no_morph, vertex_no_morph.instance_index);

@if(MORPH_TARGETS)
    var vertex = morph_vertex(uncompressed_vertex_no_morph, vertex_no_morph.instance_index);
@else
    var vertex = uncompressed_vertex_no_morph;

    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);

@if(SKINNED)
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index
    );
@else
    var world_from_local = mesh_world_from_local;

@if(VERTEX_NORMALS && SKINNED)
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
@if(VERTEX_NORMALS && !SKINNED)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        vertex_no_morph.instance_index
    );

@if(VERTEX_POSITIONS) {
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    @if(OUTLINE_PASS) {
        let animated_uv = calc_animated_uv((material.uv_transform * vec3(mesh_uv(vertex), 1.0)).xy);
        let width = outline_width(animated_uv);
        let outline_normal = normalize(out.world_normal.xyz);
        out.world_position = vec4(out.world_position.xyz + outline_normal * width, 1.0);
    }
    out.position = position_world_to_clip(out.world_position.xyz);
}

@if(VERTEX_UVS_A)
    out.uv = vertex.uv;
@if(VERTEX_UVS_B)
    out.uv_b = vertex.uv_b;

@if(VERTEX_TANGENTS)
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex_no_morph.instance_index
    );

@if(VERTEX_COLORS)
    out.color = vertex.color;

@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_no_morph.instance_index;

@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index, mesh_world_from_local[3]);

    return out;
}

// The mesh's first UV set. `UncompressedVertex.uv` only exists when the mesh has
// one; a mesh without one samples the outline textures at the origin.
fn mesh_uv(vertex: UncompressedVertex) -> vec2<f32> {
@if(VERTEX_UVS_A)
    return vertex.uv;
@else
    return vec2<f32>(0.0);
}

fn outline_width(uv: vec2<f32>) -> f32{
    let w = material.outline_width_factor;
    if ((material.flags & OUTLINE_WIDTH_MULTIPLY_TEXTURE) != 0) {
        let texel = textureSampleLevel(outline_width_multiply_texture, outline_width_multiply_sampler, uv, 0.0);
        return w * texel.g;
    } else {
        return w;
    }
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
        let mask = textureSampleLevel(uv_animation_mask_texture, uv_animation_mask_sampler, uv, 0.0).b;
        return mask * globals.time;
    }else{
        return globals.time;
    }
}

@if(MORPH_TARGETS)
fn morph_vertex(vertex_in: UncompressedVertex, instance_index: u32) -> UncompressedVertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;
    let morph_descriptor_index = mesh[instance_index].morph_descriptor_index;

    let weight_count = bevy_pbr::render::morph::layer_count(morph_descriptor_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::render::morph::weight_at(i, morph_descriptor_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, morph_descriptor_index);
        @if(VERTEX_NORMALS)
        vertex.normal += weight * morph_normal(vertex_index, i, morph_descriptor_index);
        @if(VERTEX_TANGENTS)
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, morph_descriptor_index), 0.0);
    }
    return vertex;
}
