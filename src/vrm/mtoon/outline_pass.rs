mod phase_item;
mod pipeline;
mod render_command;
mod view_node;

use crate::vrm::mtoon::outline_pass::phase_item::{OutlinePhaseItem, OutlineTransparentPhaseItem};
use crate::vrm::mtoon::outline_pass::pipeline::MToonOutlinePipeline;
use crate::vrm::mtoon::outline_pass::render_command::{DrawOutline, DrawOutlineTransparent};
use crate::vrm::mtoon::outline_pass::view_node::{OutlineDrawNode, OutlineDrawPassLabel, OutlineTransparentDrawPassLabel, OutlineTransparentNode};
use crate::vrm::mtoon::MToonMaterial;
use bevy::core_pipeline::core_3d::Transparent3d;
use bevy::pbr::{
    alpha_mode_pipeline_key, queue_material_meshes, MaterialBindGroupAllocator,
    MaterialPipelineKey, PreparedMaterial, RenderLightmaps, RenderMeshInstanceFlags,
    RenderPhaseType, ViewKeyCache,
};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::sync_world::MainEntityHashMap;
use bevy::render::view::RenderVisibilityRanges;
use bevy::{
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    math::FloatOrd,
    pbr::{MeshPipeline, MeshPipelineKey, RenderMeshInstances},
    platform::collections::HashSet,
    prelude::*,
    render::{
        mesh::RenderMesh,
        render_asset::RenderAssets,
        render_graph::{RenderGraphApp, ViewNodeRunner},
        render_phase::{
            sort_phase_system, AddRenderCommand, DrawFunctions, PhaseItemExtraIndex,
            SortedRenderPhasePlugin, ViewSortedRenderPhases,
        },
        render_resource::{PipelineCache, SpecializedMeshPipelines},
        view::{ExtractedView, RenderVisibleEntities, RetainedViewEntity},
        Extract, Render, RenderApp, RenderDebugFlags, RenderSet,
    },
};
use bevy::ecs::schedule::graph::Direction::Outgoing;

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
            .init_resource::<SpecializedMeshPipelines<MToonOutlinePipeline>>()
            .init_resource::<DrawFunctions<OutlinePhaseItem>>()
            .init_resource::<DrawFunctions<OutlineTransparentPhaseItem>>()
            .add_render_command::<OutlinePhaseItem, DrawOutline>()
            .add_render_command::<OutlineTransparentPhaseItem, DrawOutlineTransparent>()
            .init_resource::<ViewSortedRenderPhases<OutlinePhaseItem>>()
            .init_resource::<ViewSortedRenderPhases<OutlineTransparentPhaseItem>>()
            .init_resource::<MToonMaterialInstances>()
            .add_systems(
                ExtractSchedule,
                (extract_camera_phases, extract_mtoon_materials),
            )
            .add_systems(
                Render,
                (
                    queue_outlines
                        .after(queue_material_meshes::<MToonMaterial>)
                        .in_set(RenderSet::QueueMeshes),
                    (
                        sort_phase_system::<OutlinePhaseItem>,
                        sort_phase_system::<OutlineTransparentPhaseItem>,
                    )
                        .in_set(RenderSet::PhaseSort),
                ),
            );

        render_app
            .add_render_graph_node::<ViewNodeRunner<OutlineDrawNode>>(Core3d, OutlineDrawPassLabel)
            .add_render_graph_node::<ViewNodeRunner<OutlineTransparentNode>>(Core3d, OutlineTransparentDrawPassLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::MainTransparentPass,
                    OutlineDrawPassLabel,
                    OutlineTransparentDrawPassLabel,
                    Node3d::EndMainPass,
                ),
            );
    }

    fn finish(
        &self,
        app: &mut App,
    ) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<MToonOutlinePipeline>();
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
struct MToonMaterialInstances(MainEntityHashMap<AssetId<MToonMaterial>>);

fn extract_camera_phases(
    mut outline_phases: ResMut<ViewSortedRenderPhases<OutlinePhaseItem>>,
    mut transparent_render_phases: ResMut<ViewSortedRenderPhases<OutlineTransparentPhaseItem>>,
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
        transparent_render_phases.insert_or_clear(retained_view_entity);
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
    mut transparent_render_phases: ResMut<ViewSortedRenderPhases<OutlineTransparentPhaseItem>>,
    mut views: Query<(&ExtractedView, &RenderVisibleEntities, &Msaa)>,
    material_bind_group_allocator: Res<MaterialBindGroupAllocator<MToonMaterial>>,
    view_key_cache: Res<ViewKeyCache>,
    mesh_allocator: Res<MeshAllocator>,
    render_lightmaps: Res<RenderLightmaps>,
    render_visibility_ranges: Res<RenderVisibilityRanges>,
    instances: Res<MToonMaterialInstances>,
    render_materials: Res<RenderAssets<PreparedMaterial<MToonMaterial>>>,
    draw_functions: Res<DrawFunctions<OutlinePhaseItem>>,
    pipeline_cache: Res<PipelineCache>,
    outline_pipeline: Res<MToonOutlinePipeline>,
    render_meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
) {
    for (view, visible_entities, msaa) in &mut views {
        let Some(view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };
        let Some(outline_phase) = outline_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(transparent_phase) = transparent_render_phases.get_mut(&view.retained_view_entity)
        else {
            continue;
        };

        let draw_function_id = draw_functions.read().id::<DrawOutline>();
        let rangefinder = view.rangefinder3d();
        for (render_entity, visible_entity) in visible_entities.iter::<Mesh3d>() {
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*visible_entity)
            else {
                continue;
            };
            let Some(mesh) = render_meshes.get(mesh_instance.mesh_asset_id) else {
                continue;
            };
            let Some(asset_id) = instances.get(visible_entity) else {
                continue;
            };
            let Some(material) = render_materials.get(*asset_id) else {
                continue;
            };
            let mut mesh_pipeline_key_bits = material.properties.mesh_pipeline_key_bits;
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
                    .flags
                    .contains(RenderMeshInstanceFlags::HAS_PREVIOUS_SKIN)
                {
                    mesh_key |= MeshPipelineKey::HAS_PREVIOUS_SKIN;
                }
                if mesh_instance
                    .flags
                    .contains(RenderMeshInstanceFlags::HAS_PREVIOUS_MORPH)
                {
                    mesh_key |= MeshPipelineKey::HAS_PREVIOUS_MORPH;
                }
            }

            let material_key = MaterialPipelineKey {
                mesh_key,
                bind_group_data: *material_bind_group_allocator
                    .get(material.binding.group)
                    .unwrap()
                    .get_extra_data(material.binding.slot),
            };

            let pipeline_id = pipelines.specialize(
                &pipeline_cache,
                &outline_pipeline,
                material_key,
                &mesh.layout,
            );
            let pipeline_id = match pipeline_id {
                Ok(id) => id,
                Err(err) => {
                    error!("{}", err);
                    continue;
                }
            };
            let distance = material.properties.depth_bias;
            match material.properties.render_phase_type {
                // RenderPhaseType::Transparent => {
                //     transparent_phase.add(OutlineTransparentPhaseItem {
                //         sort_key: FloatOrd(distance),
                //         entity: (*render_entity, *visible_entity),
                //         pipeline: pipeline_id,
                //         draw_function: draw_function_id,
                //         batch_range: 0..0,
                //         extra_index: PhaseItemExtraIndex::None,
                //         indexed: mesh.indexed(),
                //     });
                // }
                _ => {
                    outline_phase.add(OutlinePhaseItem {
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
}
