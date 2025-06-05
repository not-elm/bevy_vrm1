use crate::vrm::mtoon::MToonMaterial;
use bevy::pbr::MaterialPipeline;
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{CompareFunction, DepthBiasState, DepthStencilState, Face, RenderPipelineDescriptor, SpecializedMeshPipeline, SpecializedMeshPipelineError, StencilState, TextureFormat};

#[derive(Resource)]
pub(super) struct MToonOutlinePipeline {
    base: MaterialPipeline<MToonMaterial>,
}

impl FromWorld for MToonOutlinePipeline {
    fn from_world(world: &mut World) -> Self {
        Self {
            base: MaterialPipeline::from_world(world),
        }
    }
}

impl SpecializedMeshPipeline for MToonOutlinePipeline {
    type Key = <MaterialPipeline<MToonMaterial> as SpecializedMeshPipeline>::Key;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.base.specialize(key.clone(), layout)?;
        descriptor.label.replace("mtoon_outline_pipeline".into());
        descriptor.vertex.shader_defs.push("OUTLINE_PASS".into());
        descriptor.fragment.as_mut().unwrap().shader_defs.push("OUTLINE_PASS".into());
        descriptor.depth_stencil.replace(DepthStencilState {
            depth_compare: CompareFunction::Greater,
            depth_write_enabled: true,
            format: TextureFormat::Depth32Float,
            stencil: StencilState::default(),
            bias: DepthBiasState::default(),
        });
        descriptor.primitive.cull_mode.replace(Face::Front);
        Ok(descriptor)
    }
}
