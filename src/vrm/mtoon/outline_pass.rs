mod phase_item;
mod pipeline;
mod render_command;
mod view_node;

use crate::error::vrm_error;
use crate::vrm::mtoon::outline_pass::phase_item::OutlinePhaseItem;
use crate::vrm::mtoon::outline_pass::pipeline::{MToonOutlinePipeline, OutlinePipelineKey};
use crate::vrm::mtoon::outline_pass::render_command::DrawOutline;
use crate::vrm::mtoon::{MToonMaterial, MToonMaterialKey};
use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::pbr::{
    MaterialPipeline, MaterialPipelineKey, PreparedMaterial, RenderMeshInstanceFlags, ViewKeyCache,
    alpha_mode_pipeline_key, init_material_pipeline, queue_material_meshes,
};
use bevy::render::sync_world::MainEntityHashMap;
use bevy::render::view::RenderVisibilityRanges;
use bevy::render::{GpuResourceAppExt, RenderStartup};
use bevy::{
    math::FloatOrd,
    pbr::{MeshPipeline, MeshPipelineKey, RenderMeshInstances},
    platform::collections::HashSet,
    prelude::*,
    render::{
        Extract, Render, RenderApp, RenderDebugFlags, RenderSystems,
        erased_render_asset::ErasedRenderAssets,
        mesh::RenderMesh,
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItemExtraIndex, SortedRenderPhasePlugin,
            ViewSortedRenderPhases, sort_phase_system,
        },
        render_resource::{PipelineCache, SpecializedMeshPipelines},
        view::{ExtractedView, RenderVisibleEntities, RetainedViewEntity},
    },
};
use std::any::TypeId;

pub struct MToonOutlinePlugin;

impl Plugin for MToonOutlinePlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_plugins(
            SortedRenderPhasePlugin::<OutlinePhaseItem, MeshPipeline>::new(
                RenderDebugFlags::default(),
            ),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_gpu_resource::<SpecializedMeshPipelines<MToonOutlinePipeline>>()
            .init_resource::<DrawFunctions<OutlinePhaseItem>>()
            .add_render_command::<OutlinePhaseItem, DrawOutline>()
            .init_resource::<ViewSortedRenderPhases<OutlinePhaseItem>>()
            .init_resource::<MToonMaterialInstances>()
            .add_systems(
                RenderStartup,
                init_mtoon_outline_pipeline.after(init_material_pipeline),
            )
            .add_systems(
                ExtractSchedule,
                (extract_camera_phases, extract_mtoon_materials),
            )
            .add_systems(
                Render,
                (
                    queue_outlines.in_set(RenderSystems::QueueMeshes),
                    sort_phase_system::<OutlinePhaseItem>.in_set(RenderSystems::PhaseSort),
                ),
            )
            .add_systems(Render, queue_outlines.after(queue_material_meshes));

        render_app.add_systems(
            Core3d,
            view_node::outline_draw_pass
                .in_set(Core3dSystems::MainPass)
                .after(main_transparent_pass_3d),
        );
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
struct MToonMaterialInstances(MainEntityHashMap<AssetId<MToonMaterial>>);

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
        outline_phases.prepare_for_new_frame(retained_view_entity);
        live_entities.insert(retained_view_entity);
    }

    outline_phases.retain(|camera_entity, _| live_entities.contains(camera_entity));
}

fn extract_mtoon_materials(
    mut instances: ResMut<MToonMaterialInstances>,
    materials: Extract<Query<(Entity, &MeshMaterial3d<MToonMaterial>)>>,
) {
    materials.iter().for_each(|(entity, material)| {
        instances.0.insert(entity.into(), material.id());
    });
}

fn queue_outlines(
    mut pipelines: ResMut<SpecializedMeshPipelines<MToonOutlinePipeline>>,
    mut outline_phases: ResMut<ViewSortedRenderPhases<OutlinePhaseItem>>,
    mut views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    view_key_cache: Res<ViewKeyCache>,
    render_visibility_ranges: Res<RenderVisibilityRanges>,
    instances: Res<MToonMaterialInstances>,
    render_materials: Res<ErasedRenderAssets<PreparedMaterial>>,
    draw_functions: Res<DrawFunctions<OutlinePhaseItem>>,
    pipeline_cache: Res<PipelineCache>,
    outline_pipeline: Res<MToonOutlinePipeline>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
) {
    for (view, visible_entities) in &mut views {
        let Some(view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };
        let Some(outline_phase) = outline_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let draw_function_id = draw_functions.read().id::<DrawOutline>();
        let Some(visible_mesh_entities) = visible_entities.get::<Mesh3d>() else {
            continue;
        };
        for (render_entity, visible_entity) in &visible_mesh_entities.entities_cpu_culling {
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*visible_entity)
            else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id()) else {
                continue;
            };
            let Some(asset_id) = instances.get(visible_entity) else {
                continue;
            };
            let Some(material) = render_materials.get(*asset_id) else {
                continue;
            };

            let mut mesh_pipeline_key_bits: MeshPipelineKey =
                material.properties.mesh_pipeline_key_bits.downcast();
            mesh_pipeline_key_bits.insert(alpha_mode_pipeline_key(
                material.properties.alpha_mode,
                &Msaa::from_samples(view_key.msaa_samples()),
            ));
            let mut mesh_key = *view_key
                | MeshPipelineKey::from_bits_retain(mesh.key_bits.bits())
                | mesh_pipeline_key_bits;

            if render_visibility_ranges.entity_has_crossfading_visibility_ranges(*visible_entity) {
                mesh_key |= MeshPipelineKey::VISIBILITY_RANGE_DITHER;
            }

            if view_key.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS) {
                if mesh_instance
                    .flags()
                    .contains(RenderMeshInstanceFlags::HAS_PREVIOUS_SKIN)
                {
                    mesh_key |= MeshPipelineKey::HAS_PREVIOUS_SKIN;
                }
                if mesh_instance
                    .flags()
                    .contains(RenderMeshInstanceFlags::HAS_PREVIOUS_MORPH)
                {
                    mesh_key |= MeshPipelineKey::HAS_PREVIOUS_MORPH;
                }
            }

            // Get the material key from the prepared material properties
            let mtoon_key = material
                .properties
                .material_key
                .to_key::<MToonMaterialKey>();

            // Skip outline for double-sided meshes (cull_mode: None).
            // The inverted-hull outline technique assumes back-faces are invisible
            // in the main pass; on double-sided meshes the inflated back-faces
            // overwrite correctly-rendered pixels with the outline color.
            if !mtoon_key.intersects(MToonMaterialKey::CULL_FRONT | MToonMaterialKey::CULL_BACK) {
                continue;
            }

            let outline_key = OutlinePipelineKey {
                mesh_key,
                bind_group_data: mtoon_key,
            };

            let pipeline_id = match pipelines.specialize(
                &pipeline_cache,
                &outline_pipeline,
                outline_key,
                &mesh.layout,
            ) {
                Ok(id) => id,
                Err(err) => {
                    vrm_error!(err);
                    continue;
                }
            };
            let distance = material.properties.depth_bias;
            {
                outline_phase.add_transient(OutlinePhaseItem {
                    sort_key: FloatOrd(distance),
                    entity: (*render_entity, *visible_entity),
                    pipeline: pipeline_id,
                    draw_function: draw_function_id,
                    batch_range: 0..0,
                    extra_index: PhaseItemExtraIndex::None,
                    indexed: mesh.indexed(),
                });
            }
        }
    }
}

fn init_mtoon_outline_pipeline(
    mut commands: Commands,
    material_pipeline: Res<MaterialPipeline>,
    render_device: Res<bevy::render::renderer::RenderDevice>,
) {
    use bevy::render::render_resource::AsBindGroup;
    let material_layout = MToonMaterial::bind_group_layout_descriptor(&render_device);
    commands.insert_resource(MToonOutlinePipeline {
        base: material_pipeline.clone(),
        material_layout,
    });
}
