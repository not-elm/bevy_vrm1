use crate::vrm::mtoon::outline::{MToonOutlineUniform, OUTLINE_SHADER_HANDLE};
use bevy::asset::AssetId;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::log::error;
use bevy::math::UVec2;
use bevy::pbr::{setup_morph_and_skinning_defs, skins_use_uniform_buffers, MeshInputUniform, MeshPipeline, MeshPipelineKey, MeshPipelineViewLayoutKey, MeshUniform, RenderMeshInstances};
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{IndirectParametersCpuMetadata, UntypedPhaseIndirectParametersBuffers};
use bevy::render::batching::{GetBatchData, GetFullBatchData};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{MeshVertexBufferLayoutRef, RenderMesh};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::uniform_buffer_sized;
use bevy::render::render_resource::ShaderType;
use bevy::render::render_resource::{BindGroupLayout, BindGroupLayoutEntries, BlendState, ColorTargetState, ColorWrites, CompareFunction, DepthBiasState, DepthStencilState, Face, FragmentState, FrontFace, MultisampleState, PolygonMode, PrimitiveState, RenderPipelineDescriptor, ShaderStages, SpecializedMeshPipeline, SpecializedMeshPipelineError, StencilState, TextureFormat, VertexState};
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::MainEntity;
use nonmax::NonMaxU32;

#[derive(Resource)]
pub(super) struct MToonOutlinePipeline {
    mesh_pipeline: MeshPipeline,
    shader_handle: Handle<Shader>,
    pub(crate) outline_unifrom_layout: BindGroupLayout,
    skins_use_buffers: bool,
}

impl FromWorld for MToonOutlinePipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        Self {
            mesh_pipeline: world.resource::<MeshPipeline>().clone(),
            shader_handle: OUTLINE_SHADER_HANDLE,
            outline_unifrom_layout:  render_device.create_bind_group_layout(
                "outline_uniform_layout",
                &BindGroupLayoutEntries::single(
                    ShaderStages::VERTEX_FRAGMENT,
                    uniform_buffer_sized(false, Some(MToonOutlineUniform::min_size())),
                ),
            ),
            skins_use_buffers: skins_use_uniform_buffers(render_device),
        }
    }
}

impl SpecializedMeshPipeline for MToonOutlinePipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut vertex_defs = vec![];
        let mut buffer_attrs = vec![
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
        ];

        let mesh_bind_group = setup_morph_and_skinning_defs(
            &self.mesh_pipeline.mesh_layouts,
            layout,
            5,
            &key,
            &mut vertex_defs,
            &mut buffer_attrs,
            self.skins_use_buffers,
        );
        let buffers = vec![layout.0.get_layout(&buffer_attrs)?];
        Ok(RenderPipelineDescriptor {
            label: Some("OutlinePipeline".into()),
            layout: vec![
                self.mesh_pipeline
                    .get_view_layout(MeshPipelineViewLayoutKey::from(key))
                    .clone(),
                mesh_bind_group,
                self.outline_unifrom_layout.clone(),
            ],
            push_constant_ranges: vec![],
            vertex: VertexState {
                shader: self.shader_handle.clone(),
                shader_defs: vertex_defs,
                entry_point: "vertex".into(),
                buffers,
            },
            fragment: Some(FragmentState {
                shader: self.shader_handle.clone(),
                shader_defs: vec![],
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Front),
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
                topology: key.primitive_topology(),
                strip_index_format: None,
            },
            depth_stencil: Some(DepthStencilState{
                depth_compare: CompareFunction::Greater,
                depth_write_enabled: true,
                format: TextureFormat::Depth32Float,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState{
                count: key.msaa_samples(),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            zero_initialize_workgroup_memory: false,
        })
    }
}

impl GetBatchData for MToonOutlinePipeline {
    type Param = (
        SRes<RenderMeshInstances>,
        SRes<RenderAssets<RenderMesh>>,
        SRes<MeshAllocator>,
    );
    type CompareData = AssetId<Mesh>;
    type BufferData = MeshUniform;

    fn get_batch_data(
        (mesh_instances, _render_assets, mesh_allocator): &SystemParamItem<Self::Param>,
        (_entity, main_entity): (Entity, MainEntity),
    ) -> Option<(Self::BufferData, Option<Self::CompareData>)> {
        let RenderMeshInstances::CpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!(
                "`get_batch_data` should never be called in GPU mesh uniform \
                building mode"
            );
            return None;
        };
        let mesh_instance = mesh_instances.get(&main_entity)?;
        let first_vertex_index =
            match mesh_allocator.mesh_vertex_slice(&mesh_instance.mesh_asset_id) {
                Some(mesh_vertex_slice) => mesh_vertex_slice.range.start,
                None => 0,
            };
        let mesh_uniform = {
            let mesh_transforms = &mesh_instance.transforms;
            let (local_from_world_transpose_a, local_from_world_transpose_b) =
                mesh_transforms.world_from_local.inverse_transpose_3x3();
            MeshUniform {
                world_from_local: mesh_transforms.world_from_local.to_transpose(),
                previous_world_from_local: mesh_transforms.previous_world_from_local.to_transpose(),
                lightmap_uv_rect: UVec2::ZERO,
                local_from_world_transpose_a,
                local_from_world_transpose_b,
                flags: mesh_transforms.flags,
                first_vertex_index,
                current_skin_index: u32::MAX,
                material_and_lightmap_bind_group_slot: 0,
                tag: 0,
                pad: 0,
            }
        };
        Some((mesh_uniform, None))
    }
}

impl GetFullBatchData for MToonOutlinePipeline {
    type BufferInputData = MeshInputUniform;

    fn get_binned_batch_data(
        (mesh_instances, _render_assets, mesh_allocator): &SystemParamItem<Self::Param>,
        main_entity: MainEntity,
    ) -> Option<Self::BufferData> {
        let RenderMeshInstances::CpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!(
                "`get_binned_batch_data` should never be called in GPU mesh uniform building mode"
            );
            return None;
        };
        let mesh_instance = mesh_instances.get(&main_entity)?;
        let first_vertex_index = mesh_allocator
            .mesh_vertex_slice(&mesh_instance.mesh_asset_id)
            .map(|slice| slice.range.start)
            .unwrap_or_default();
        Some(MeshUniform::new(
            &mesh_instance.transforms,
            first_vertex_index,
            mesh_instance.material_bindings_index.slot,
            None,
            None,
            None,
        ))
    }

    fn get_index_and_compare_data(
        (mesh_instances, _, _): &SystemParamItem<Self::Param>,
        main_entity: MainEntity,
    ) -> Option<(NonMaxU32, Option<Self::CompareData>)> {
        let RenderMeshInstances::GpuBuilding(ref mesh_instances) = **mesh_instances else {
            error!(
                "`get_index_and_compare_data` should never be called in CPU mesh uniform building \
                mode"
            );
            return None;
        };

        let mesh_instance = mesh_instances.get(&main_entity)?;
        Some((
            mesh_instance.current_uniform_index,
            mesh_instance
                .should_batch()
                .then_some(mesh_instance.mesh_asset_id),
        ))
    }

    fn get_binned_index(
        _param: &SystemParamItem<Self::Param>,
        _query_item: MainEntity,
    ) -> Option<NonMaxU32> {
        None
    }

    fn write_batch_indirect_parameters_metadata(
        indexed: bool,
        base_output_index: u32,
        batch_set_index: Option<NonMaxU32>,
        indirect_parameters_buffers: &mut UntypedPhaseIndirectParametersBuffers,
        indirect_parameters_offset: u32,
    ) {
        let indirect_parameters = IndirectParametersCpuMetadata {
            base_output_index,
            batch_set_index: match batch_set_index {
                None => !0,
                Some(batch_set_index) => u32::from(batch_set_index),
            },
        };

        if indexed {
            indirect_parameters_buffers
                .indexed
                .set(indirect_parameters_offset, indirect_parameters);
        } else {
            indirect_parameters_buffers
                .non_indexed
                .set(indirect_parameters_offset, indirect_parameters);
        }
    }
}
