use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::pbr::{DrawMesh, SetMeshBindGroup, SetMeshViewBindGroup};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_phase::{
    PhaseItem, RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
};
use bevy::render::render_resource::BindGroup;
use bevy::render::sync_world::MainEntity;

pub(super) type DrawOutline = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshBindGroup<1>,
    SetOutlineBindGroup<2>,
    DrawMesh,
);

pub(crate) struct SetOutlineBindGroup<const I: usize>();

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetOutlineBindGroup<I> {
    type Param = SRes<OutlineBindGroups>;
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        item: &P,
        _view_data: ROQueryItem<'w, Self::ViewQuery>,
        _entity_data: Option<()>,
        bind_group: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(group) = &bind_group.into_inner().0.get(&item.main_entity()) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, group, &[]);
        RenderCommandResult::Success
    }
}

#[derive(Resource, Deref, DerefMut, Default)]
pub(crate) struct OutlineBindGroups(HashMap<MainEntity, BindGroup>);
