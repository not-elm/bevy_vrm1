mod pipeline;
mod render_command;

use crate::vrm::gltf::materials::VrmcMaterialsExtensitions;
use crate::vrm::mtoon::outline::pipeline::MToonOutlinePipeline;
use crate::vrm::mtoon::outline::render_command::{DrawOutline, OutlineBindGroups};
use bevy::asset::{load_internal_asset, weak_handle};
use bevy::render::render_resource::{BindGroupEntry, BufferUsages, ShaderType, StoreOp, UniformBuffer};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::view::ViewDepthTexture;
use bevy::{
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::query::QueryItem,
    math::FloatOrd,
    pbr::{
        MeshPipeline, MeshPipelineKey, RenderMeshInstances,
    },
    platform::collections::HashSet,
    prelude::*,
    render::{
        camera::ExtractedCamera,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        mesh::RenderMesh,
        render_asset::RenderAssets,
        render_graph::{
            NodeRunError, RenderGraphApp, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_phase::{
            sort_phase_system, AddRenderCommand, CachedRenderPipelinePhaseItem, DrawFunctionId,
            DrawFunctions, PhaseItem, PhaseItemExtraIndex, SortedPhaseItem,
            SortedRenderPhasePlugin, ViewSortedRenderPhases,
        },
        render_resource::{
            CachedRenderPipelineId, PipelineCache, RenderPassDescriptor,
            SpecializedMeshPipelines,
        },
        renderer::RenderContext,
        sync_world::MainEntity,
        view::{ExtractedView, RenderVisibleEntities, RetainedViewEntity, ViewTarget},
        Extract, Render, RenderApp, RenderDebugFlags, RenderSet
    },
};
use std::ops::Range;

const OUTLINE_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("fd53b589-5a4c-6f4d-9318-18db0f44db85");

#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(not(feature = "reflect"), derive(TypePath))]
#[derive(Asset, PartialEq, Debug, Clone, Component, ExtractComponent)]
pub struct MToonOutline {
    /// The outline width
    ///
    /// the unit is in meters.
    pub width_factor: f32,

    /// The outline color
    pub color: LinearRgba,

    /// The ratio of the surface shading result to be multiplied by the outline color.
    pub lighting_mix_factor: f32,
}

impl From<&VrmcMaterialsExtensitions> for MToonOutline{
    fn from(value: &VrmcMaterialsExtensitions) -> Self {
        let color = value.outline_color_factor;
        Self{
            width_factor: value.outline_width_factor,
            lighting_mix_factor: value.outline_lighting_mix_factor,
            color: LinearRgba::rgb(color[0], color[1], color[2]),
        }
    }
}

#[derive(Clone, ShaderType, Component)]
#[repr(align(16))]
pub(crate) struct MToonOutlineUniform {
    pub width_factor: f32,
    pub color_factor: Vec4,
    pub lighting_mix_factor: f32,
}

impl From<&MToonOutline> for MToonOutlineUniform {
    fn from(outline: &MToonOutline) -> Self {
        Self {
            width_factor: outline.width_factor,
            color_factor: outline.color.to_vec4(),
            lighting_mix_factor: outline.lighting_mix_factor,
        }
    }
}

// TODO: Not supported yet
// #[derive(Reflect, Debug, Clone, Default)]
// pub enum OutlineWidthMode {
//     /// The outline will not be drawn.
//     #[default]
//     None,
//     /// The outline width is determined by the distance in world coordinates.
//     WorldCoordinates,
//     /// The outline width is determined by the screen coordinates
//     /// and is always a constant thickness regardless of distance.
//     ScreenCoordinates,
// }

pub struct MToonOutlinePlugin;

impl Plugin for MToonOutlinePlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, OUTLINE_SHADER_HANDLE, "outline.wgsl", Shader::from_wgsl);
        app.add_plugins((
            ExtractComponentPlugin::<MToonOutline>::default(),
            SortedRenderPhasePlugin::<OutlinePhaseItem, MeshPipeline>::new(RenderDebugFlags::default()),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedMeshPipelines<MToonOutlinePipeline>>()
            .init_resource::<DrawFunctions<OutlinePhaseItem>>()
            .init_resource::<OutlineBindGroups>()
            .add_render_command::<OutlinePhaseItem, DrawOutline>()
            .init_resource::<ViewSortedRenderPhases<OutlinePhaseItem>>()
            .add_systems(ExtractSchedule, extract_camera_phases)
            .add_systems(
                Render,
                (
                    queue_outlines.in_set(RenderSet::QueueMeshes),
                    sort_phase_system::<OutlinePhaseItem>.in_set(RenderSet::PhaseSort),
                    bevy::render::batching::gpu_preprocessing::batch_and_prepare_sorted_render_phase::<OutlinePhaseItem, MToonOutlinePipeline>
                        .in_set(RenderSet::PrepareResources),
                ),
            );

        render_app
            .add_render_graph_node::<ViewNodeRunner<OutlineDrawNode>>(Core3d, OutlineDrawPassLabel)
            .add_render_graph_edges(Core3d, (Node3d::MainOpaquePass, OutlineDrawPassLabel));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<MToonOutlinePipeline>();
    }
}

struct OutlinePhaseItem {
    pub sort_key: FloatOrd,
    pub entity: (Entity, MainEntity),
    pub pipeline: CachedRenderPipelineId,
    pub draw_function: DrawFunctionId,
    pub batch_range: Range<u32>,
    pub extra_index: PhaseItemExtraIndex,
    pub indexed: bool,
}

impl PhaseItem for OutlinePhaseItem {
    #[inline]
    fn entity(&self) -> Entity {
        self.entity.0
    }

    #[inline]
    fn main_entity(&self) -> MainEntity {
        self.entity.1
    }

    #[inline]
    fn draw_function(&self) -> DrawFunctionId {
        self.draw_function
    }

    #[inline]
    fn batch_range(&self) -> &Range<u32> {
        &self.batch_range
    }

    #[inline]
    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        &mut self.batch_range
    }

    #[inline]
    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.extra_index.clone()
    }

    #[inline]
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl SortedPhaseItem for OutlinePhaseItem {
    type SortKey = FloatOrd;

    #[inline]
    fn sort_key(&self) -> Self::SortKey {
        self.sort_key
    }

    #[inline]
    fn sort(items: &mut [Self]) {
        items.sort_by_key(SortedPhaseItem::sort_key);
    }

    #[inline]
    fn indexed(&self) -> bool {
        self.indexed
    }
}

impl CachedRenderPipelinePhaseItem for OutlinePhaseItem {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.pipeline
    }
}

fn extract_camera_phases(
    mut outline_phases: ResMut<ViewSortedRenderPhases<OutlinePhaseItem>>,
    mut live_entities: Local<HashSet<RetainedViewEntity>>,
    cameras: Extract<Query<(Entity, &Camera), With<Camera3d>>>,
) {
    live_entities.clear();
    for (main_entity, camera) in &cameras {
        if !camera.is_active {
            continue;
        }

        let retained_view_entity = RetainedViewEntity::new(main_entity.into(), None, 0);
        outline_phases.insert_or_clear(retained_view_entity);
        live_entities.insert(retained_view_entity);
    }

    outline_phases.retain(|camera_entity, _| live_entities.contains(camera_entity));
}

fn queue_outlines(
    mut pipelines: ResMut<SpecializedMeshPipelines<MToonOutlinePipeline>>,
    mut outline_phases: ResMut<ViewSortedRenderPhases<OutlinePhaseItem>>,
    mut views: Query<(&ExtractedView, &RenderVisibleEntities, &Msaa)>,
    mut bindgroups: ResMut<OutlineBindGroups>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    draw_functions: Res<DrawFunctions<OutlinePhaseItem>>,
    pipeline_cache: Res<PipelineCache>,
    outline_pipeline: Res<MToonOutlinePipeline>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    outlines: Query<&MToonOutline>,
) {
    for (view, visible_entities, msaa) in &mut views {
        let Some(outline_phase) = outline_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let draw_function_id = draw_functions.read().id::<DrawOutline>();
        let view_key = MeshPipelineKey::from_msaa_samples(msaa.samples()) |
            MeshPipelineKey::from_hdr(view.hdr);

        let rangefinder = view.rangefinder3d();
        for (render_entity, visible_entity) in visible_entities.iter::<Mesh3d>() {
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*visible_entity)
            else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                continue;
            };
            let Ok(outline) = outlines.get(*render_entity) else {
                continue;
            };
            let mut buffer = UniformBuffer::from(MToonOutlineUniform::from(outline));
            buffer.add_usages(BufferUsages::STORAGE);
            buffer.write_buffer(&render_device, &render_queue);
            let Some(binding) = buffer.binding() else{
                continue;
            };
            let mut mesh_key = view_key;
            mesh_key |= MeshPipelineKey::from_primitive_topology(mesh.primitive_topology());
            if mesh.morph_targets.is_some(){
                mesh_key |= MeshPipelineKey::MORPH_TARGETS;
            }
            let pipeline_id = pipelines.specialize(
                &pipeline_cache,
                &outline_pipeline,
                mesh_key,
                &mesh.layout,
            );
            let pipeline_id = match pipeline_id {
                Ok(id) => id,
                Err(err) => {
                    error!("{}", err);
                    continue;
                }
            };
            let distance = rangefinder.distance_translation(&mesh_instance.translation);
            outline_phase.add(OutlinePhaseItem {
                sort_key: FloatOrd(distance),
                entity: (*render_entity, *visible_entity),
                pipeline: pipeline_id,
                draw_function: draw_function_id,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: mesh.indexed(),
            });
            bindgroups.insert(*visible_entity, render_device.create_bind_group(
                "outline_bind_group",
                &outline_pipeline.outline_unifrom_layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: binding,
                    },
                ],
            ));
        }
    }
}

#[derive(RenderLabel, Debug, Clone, Hash, PartialEq, Eq)]
struct OutlineDrawPassLabel;

#[derive(Default)]
struct OutlineDrawNode;
impl ViewNode for OutlineDrawNode {
    type ViewQuery = (
        &'static ExtractedCamera,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static ViewDepthTexture,
    );

    fn run<'w>(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        (camera, view, target, depth_texture): QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let Some(outline_phases) = world.get_resource::<ViewSortedRenderPhases<OutlinePhaseItem>>() else {
            return Ok(());
        };
        let view_entity = graph.view_entity();
        let Some(outline_pass) = outline_phases.get(&view.retained_view_entity) else {
            return Ok(());
        };

        let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("outline pass"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: Some(depth_texture.get_attachment(StoreOp::Store)),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        if let Some(viewport) = camera.viewport.as_ref() {
            render_pass.set_camera_viewport(viewport);
        }

        if let Err(err) = outline_pass.render(&mut render_pass, world, view_entity) {
            error!("Error encountered while rendering the mtoon outline phase {err:?}");
        }

        Ok(())
    }
}
