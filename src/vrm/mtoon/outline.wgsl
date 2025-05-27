#import bevy_pbr::{
    skinning::skin_model,
    mesh_functions::{
        mesh_position_local_to_world,
        mesh_normal_local_to_world,
        get_world_from_local,
    },
    mesh_bindings::mesh,
    morph,
    view_transformations::position_world_to_clip
}

struct OutlineUniform{
    width_factor: f32,
    color: vec4<f32>,
    lighting_mix_factor: f32,
}

@group(2) @binding(0) var<uniform> outline: OutlineUniform;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    #ifdef SKINNED
        @location(5) joint_indices: vec4<u32>,
        @location(6) joint_weights: vec4<f32>,
    #endif
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
};

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
#ifdef MORPH_TARGETS
    var vertex = morph_vertex(vertex_no_morph);
#else
    var vertex = vertex_no_morph;
#endif

#ifdef SKINNED
    let world_from_local = skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex.instance_index
    );
#else // SKINNED
    let world_from_local = get_world_from_local(vertex.instance_index);
#endif // SKINNED
    var world_position = mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    let world_normal = normalize(mesh_normal_local_to_world(vertex.normal, vertex.instance_index));
    var out: VertexOutput;
    world_position = vec4(world_position.xyz + world_normal.xyz * outline.width_factor, 1.0);
    out.world_position = world_position;
    out.clip_position = position_world_to_clip(out.world_position.xyz);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    //TODO: Apply lighting mix factor
//        let color = outline.color.rgb * mix(vec3(1.), outline.base_color, outline.lighting_mix_factor);
//        return half4(outlineCol, outline.color.a);
    return outline.color;
}


#ifdef MORPH_TARGETS
fn morph_vertex(vertex_in: Vertex) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[vertex.instance_index].first_vertex_index;
    let vertex_index = vertex.instance_index - first_vertex;

    let weight_count = morph::layer_count();
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = morph::weight_at(i);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph::morph(vertex_index, morph::position_offset, i);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph::morph(vertex_index, morph::normal_offset, i);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph::morph(vertex_index, morph::tangent_offset, i), 0.0);
#endif
    }
    return vertex;
}
#endif