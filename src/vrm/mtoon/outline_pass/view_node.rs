use crate::error::vrm_error;
use crate::vrm::mtoon::outline_pass::phase_item::OutlinePhaseItem;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_phase::ViewSortedRenderPhases;
use bevy::render::render_resource::{RenderPassDescriptor, StoreOp};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::view::{ExtractedView, ViewDepthStencilTexture, ViewTarget};

pub(super) fn outline_draw_pass(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthStencilTexture,
    )>,
    outline_phases: Res<ViewSortedRenderPhases<OutlinePhaseItem>>,
    mut render_context: RenderContext,
) {
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth_texture) = view.into_inner();

    let Some(outline_pass) = outline_phases.get(&extracted_view.retained_view_entity) else {
        return;
    };
    if outline_pass.items.is_empty() {
        return;
    }

    let mut render_pass = render_context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("outline pass"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth_texture.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = camera.viewport.as_ref() {
        render_pass.set_camera_viewport(viewport);
    }
    if let Err(err) = outline_pass.render(&mut render_pass, world, view_entity) {
        vrm_error!(
            "Error encountered while rendering the mtoon outline phase",
            err
        );
    }
}
