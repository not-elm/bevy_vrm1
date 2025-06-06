use crate::vrm::mtoon::MToonMaterial;
use bevy::pbr::{MaterialPipeline, MeshPipelineKey};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{BlendState, ColorWrites, CompareFunction, DepthBiasState, DepthStencilState, Face, FrontFace, PolygonMode, PrimitiveState, RenderPipelineDescriptor, SpecializedMeshPipeline, SpecializedMeshPipelineError, StencilState, TextureFormat};

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
        const PASS_NAME: &str = "OUTLINE_PASS";
        let mut descriptor = self.base.specialize(key.clone(), layout)?;
        descriptor.label.replace("mtoon_outline_pipeline".into());
        descriptor.vertex.shader_defs.push(PASS_NAME.into());
        if let Some(stencil) = descriptor.depth_stencil.as_mut(){
            stencil.depth_compare = CompareFunction::GreaterEqual;
        }
        descriptor.primitive.cull_mode.replace(Face::Front);
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push(PASS_NAME.into());
        }
        Ok(descriptor)
    }
}

