use crate::vrm::mtoon::outline_pass::phase_item::OutlinePhaseItem;
use bevy::ecs::query::QueryItem;
use bevy::log::error;
use bevy::prelude::World;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_graph::{NodeRunError, RenderGraphContext, RenderLabel, ViewNode};
use bevy::render::render_phase::ViewSortedRenderPhases;
use bevy::render::render_resource::{RenderPassDescriptor, StoreOp};
use bevy::render::renderer::RenderContext;
use bevy::render::view::{ExtractedView, ViewDepthTexture, ViewTarget};

#[derive(RenderLabel, Debug, Clone, Hash, PartialEq, Eq)]
pub(super) struct OutlineDrawPassLabel;

#[derive(Default)]
pub(super) struct OutlineDrawNode;

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
    ) -> bevy::prelude::Result<(), NodeRunError> {
        let Some(outline_phases) = world.get_resource::<ViewSortedRenderPhases<OutlinePhaseItem>>()
        else {
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
