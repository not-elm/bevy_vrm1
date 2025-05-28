use crate::vrm::mtoon::{MToonMaterial, RimLighting, Shade, UVAnimation, VrmcMaterialRegistry};
use bevy::app::{App, Plugin};
use bevy::asset::Assets;
use bevy::prelude::*;

pub struct MToonMaterialSetupPlugin;

impl Plugin for MToonMaterialSetupPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(Update, turn_to_mtoon_material);
    }
}

fn turn_to_mtoon_material(
    mut commands: Commands,
    mut mtoon_materials: ResMut<Assets<MToonMaterial>>,
    standard_materials: Res<Assets<StandardMaterial>>,
    registries: Query<&VrmcMaterialRegistry>,
    parents: Query<&ChildOf>,
    added_materials: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    added_materials.iter().for_each(|(entity, handle)| {
        let root = parents.root_ancestor(entity);
        let Ok(registry) = registries.get(root) else {
            return;
        };
        let Some(extension) = registry.materials.get(&handle.id()) else {
            return;
        };
        let Some(base) = standard_materials.get(handle.id()).cloned() else {
            return;
        };
        let mut cmd = commands.entity(entity);
        cmd.remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(
                mtoon_materials.add(MToonMaterial {
                    base_color_texture: base.base_color_texture.clone(),
                    uv_animation_mask_texture: extension
                        .uv_animation_mask_texture
                        .and_then(|tex| registry.images.get(tex.index))
                        .cloned(),
                    shade_multiply_texture: extension
                        .shade_multiply_texture
                        .and_then(|tex| registry.images.get(tex.index))
                        .cloned(),
                    shading_shift_texture: extension
                        .shading_shift_texture
                        .and_then(|tex| registry.images.get(tex.index))
                        .cloned(),
                    matcap_texture: extension
                        .matcap_texture
                        .and_then(|tex| registry.images.get(tex.index))
                        .cloned(),
                    rim_multiply_texture: extension
                        .rim_multiply_texture
                        .and_then(|tex| registry.images.get(tex.index))
                        .cloned(),
                    shade: Shade::from(extension),
                    rim_lighting: RimLighting::from(extension),
                    uv_animation: UVAnimation::from(extension),
                    gi_equalization_factor: extension.gi_equalization_factor,
                    double_sided: base.double_sided,
                    alpha_mode: base.alpha_mode,
                    depth_bias: base.depth_bias + extension.render_queue_offset_number,
                    opaque_renderer_method: base.opaque_render_method,
                    base_color: base.base_color,
                    cull_mode: base.cull_mode,
                    emissive: base.emissive,
                    emissive_texture: base.emissive_texture.clone(),
                }),
            ));
        if extension.outline_width_mode != "none" {
            // cmd.insert(MToonOutline::from(extension));
        }
    });
}
