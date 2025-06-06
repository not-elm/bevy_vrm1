use crate::vrm::mtoon::MToonMaterial;
use bevy::pbr::{DrawMesh, SetMaterialBindGroup, SetMeshBindGroup, SetMeshViewBindGroup};
use bevy::render::render_phase::SetItemPipeline;

pub(super) type DrawOutline = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshBindGroup<1>,
    SetMaterialBindGroup<MToonMaterial, 2>,
    DrawMesh,
);
