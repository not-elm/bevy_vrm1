//! `MToon` materials, built while the file loads.
//!
//! `on_material` turns the glTF material plus its `VRMC_materials_mtoon`
//! extension into an [`MToonMaterial`] and registers it as a labeled asset;
//! `on_spawn_mesh_and_material` then swaps out the `StandardMaterial` that
//! `GltfExtensionHandlerPbr` put on the mesh entity.
//!
//! # Why the conversion happens here, and not on `Added<MeshMaterial3d>`
//!
//! Watching for `Added<MeshMaterial3d<StandardMaterial>>` and looking the
//! `VRMC_materials_mtoon` payload up in a registry keyed by `StandardMaterial`
//! handle is a one-shot filter: if the `StandardMaterial` asset is not in
//! `Assets` yet when the component appears, `Assets::<StandardMaterial>::get`
//! returns `None`, the entity is skipped, and it is never revisited — so the
//! conversion is silently lost for any load where the material resolves late.
//! Doing the conversion here removes the race by construction: the material *is*
//! the glTF material at this point in the load.
//!
//! # Where the values come from
//!
//! Everything the `StandardMaterial` already resolved is copied from
//! [`GltfMaterial`], not re-derived from JSON. In particular:
//!
//! * `emissive` is `emissiveFactor * KHR_materials_emissive_strength`, applied by
//!   `bevy_gltf` at `crates/bevy_gltf/src/loader/mod.rs`. It is
//!   multiplied here by the one factor that is still missing,
//!   `VRMC_materials_hdr_emissiveMultiplier`, so each extension applies exactly
//!   once.
//! * `uv_transform` is the `KHR_texture_transform` of the base color texture,
//!   the only one `bevy_gltf` reads
//!   (`crates/bevy_gltf/src/loader/mod.rs`; see the extension table in
//!   `bevy_gltf/src/lib.rs`). `VRMC_materials_mtoon` declares a *shared*
//!   transform for every texture of the material, so the per-texture transforms
//!   of the `MToon` texture references are deliberately not read.
//! * `cull_mode`, `alpha_mode` and `double_sided` likewise.
//!
//! Because `GltfMaterial` declares neither a depth bias nor an opaque render
//! method, both are written as the `StandardMaterial` defaults — which is also
//! what `bevy_pbr`'s own `GltfMaterial` -> `StandardMaterial` conversion
//! produces (`crates/bevy_pbr/src/gltf.rs`). They are named explicitly rather
//! than left to a `Default`, so a future bevy default change is visible here.

use bevy::asset::{Handle, LoadContext};
use bevy::ecs::world::EntityWorldMut;
use bevy::gltf::GltfAssetLabel;
use bevy::gltf::GltfMaterial;
use bevy::image::Image;
use bevy::material::OpaqueRendererMethod;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::platform::collections::HashMap;

use super::VrmLoadState;
use crate::error::vrm_warn;
use crate::prelude::{MToonMaterial, MToonOutline, RimLighting, Shade, UVAnimation};
use crate::vrm::gltf::materials::{VrmcMaterialsExtensitions, VrmcMaterialsHdrEmissiveMultiplier};

/// [VRMC_materials_mtoon-1.0](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md)
pub const EXT_MTOON: &str = "VRMC_materials_mtoon";

/// Builds the [`MToonMaterial`] of one glTF material and remembers its handle
/// under `material_label`.
///
/// A material without `VRMC_materials_mtoon` keeps the `StandardMaterial` the
/// PBR handler built for it, which is what a VRM with plain PBR materials must
/// render as.
pub(crate) fn process_material(
    state: &mut VrmLoadState,
    load_context: &mut LoadContext<'_>,
    gltf_material: &gltf::Material,
    material_asset: &GltfMaterial,
    material_label: &str,
) {
    let Some(extensions) = gltf_material.extensions() else {
        return;
    };
    let Some(value) = extensions.get(EXT_MTOON) else {
        return;
    };
    let mtoon: VrmcMaterialsExtensitions = match serde_json::from_value(value.clone()) {
        Ok(parsed) => parsed,
        Err(error) => {
            vrm_warn!(format!(
                "VRM: malformed `{EXT_MTOON}` in `{material_label}`, keeping the PBR material: \
                 {error}"
            ));
            return;
        }
    };

    if mtoon.outline_width_mode == "screenCoordinates" {
        // The crate's shader has no screen-space outline
        // (`OutlineWidthMode::None` is the only other variant,
        // `src/vrm/mtoon/material/outline.rs`), so this is the same silent
        // downgrade the spec would otherwise ask for, and it is the
        // conservative direction — now said out loud.
        vrm_warn!(format!(
            "VRM MToon: `{material_label}` requests a screenCoordinates outline, which is not \
             supported; drawing no outline instead"
        ));
    }

    // A *sibling* material extension, read from the same `extensions` map and
    // independent of the shape of the MToon block next to it. A material
    // without it keeps a multiplier of `1.0`.
    let hdr_emissive_multiplier =
        VrmcMaterialsHdrEmissiveMultiplier::from_material_extensions(extensions)
            .map_or(1.0, |multiplier| multiplier.emissive_multiplier);

    let texture_count = state.texture_count;
    let textures = &state.textures;
    let material = mtoon_material(
        &mut |index| texture(load_context, textures, texture_count, index),
        material_asset,
        &mtoon,
        hdr_emissive_multiplier,
    );
    let handle = load_context.add_labeled_asset(format!("{material_label}/MToon"), material);
    state.mtoon.insert(material_label.to_owned(), handle);
}

/// Replaces the PBR material on one mesh entity with the `MToon` one.
///
/// `material_label` is the same string `bevy_gltf` passed to `on_material`: both
/// come from `GltfAssetLabel::Material { index, is_scale_inverted }`
/// (`crates/bevy_gltf/src/loader/gltf_ext/material.rs`), including on the
/// inverted-scale path in `loader/mod.rs`.
pub(crate) fn swap_material(
    state: &VrmLoadState,
    entity: &mut EntityWorldMut,
    material_label: &str,
) {
    let Some(handle) = state.mtoon.get(material_label) else {
        return;
    };
    entity.remove::<MeshMaterial3d<StandardMaterial>>();
    entity.insert(MeshMaterial3d(handle.clone()));

    // The outline is *not* a second entity: `MToonOutlinePlugin::queue_outlines`
    // (`src/vrm/mtoon/outline_pass.rs`) walks
    // `RenderVisibleEntities::get::<Mesh3d>()` of the view and draws a second
    // phase for the very same mesh entity, keyed by its
    // `MeshMaterial3d<MToonMaterial>`. The outline therefore inherits the
    // entity's `RenderLayers` through `VisibleEntities` with nothing to
    // propagate here.
}

/// The full `VRMC_materials_mtoon` -> [`MToonMaterial`] mapping.
///
/// `textures` resolves a glTF texture index to the labeled image handle
/// `bevy_gltf` created for it.
fn mtoon_material(
    textures: &mut impl FnMut(Option<usize>) -> Option<Handle<Image>>,
    base: &GltfMaterial,
    extension: &VrmcMaterialsExtensitions,
    hdr_emissive_multiplier: f32,
) -> MToonMaterial {
    MToonMaterial {
        base_color: base.base_color,
        base_color_texture: base.base_color_texture.clone(),
        // `bevy_gltf` reads `KHR_texture_transform` off the base color texture
        // only, and MToon applies one transform to every texture of the
        // material, so this single value is the right one.
        uv_transform: base.uv_transform,
        // See the module docs: `KHR_materials_emissive_strength` is already
        // folded into `base.emissive`.
        emissive: base.emissive * hdr_emissive_multiplier,
        emissive_texture: base.emissive_texture.clone(),
        shade: Shade::from(extension),
        shade_multiply_texture: textures(extension.shade_multiply_texture.map(|t| t.index)),
        shading_shift_texture: textures(extension.shading_shift_texture.map(|t| t.index)),
        rim_lighting: RimLighting::from(extension),
        rim_multiply_texture: textures(extension.rim_multiply_texture.map(|t| t.index)),
        matcap_texture: textures(extension.matcap_texture.map(|t| t.index)),
        outline: MToonOutline::from(extension),
        outline_width_multiply_texture: textures(
            extension.outline_width_multiply_texture.map(|t| t.index),
        ),
        uv_animation: UVAnimation::from(extension),
        uv_animation_mask_texture: textures(extension.uv_animation_mask_texture.map(|t| t.index)),
        gi_equalization_factor: extension.gi_equalization_factor,
        alpha_mode: base.alpha_mode,
        double_sided: base.double_sided,
        // glTF declares neither; see the module docs.
        depth_bias: 0.0,
        opaque_renderer_method: OpaqueRendererMethod::default(),
        render_queue_offset: extension.render_queue_offset_number,
        transparent_with_z_write: extension.transparent_with_z_write,
        cull_mode: base.cull_mode,
    }
}

/// Handle to the image `bevy_gltf` created for texture `index`.
///
/// The handle recorded by `on_texture` is preferred: it is the one bevy loads,
/// whether the image is embedded (a `Texture{index}` labeled asset) or a
/// separate file (loaded by path, with no such label). The label handle is only
/// the fallback for a texture the hook did not report.
fn texture(
    load_context: &mut LoadContext<'_>,
    textures: &HashMap<usize, Handle<Image>>,
    texture_count: usize,
    index: Option<usize>,
) -> Option<Handle<Image>> {
    existing_texture(index, texture_count).map(|index| {
        textures.get(&index).cloned().unwrap_or_else(|| {
            load_context.get_label_handle::<Image>(GltfAssetLabel::Texture(index).to_string())
        })
    })
}

/// `index`, unless the file has no such texture: a handle to it would never
/// load, and the material waiting on it would never render.
fn existing_texture(
    index: Option<usize>,
    texture_count: usize,
) -> Option<usize> {
    let index = index?;
    if index >= texture_count {
        vrm_warn!(format!(
            "VRM MToon: texture {index} does not exist ({texture_count} textures); ignoring it"
        ));
        return None;
    }
    Some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::OutlineWidthMode;
    use crate::success;
    use crate::tests::TestResult;
    use bevy::color::LinearRgba;
    use bevy::prelude::*;

    fn extension(json: &str) -> TestResult<VrmcMaterialsExtensitions> {
        Ok(serde_json::from_str(json)?)
    }

    /// `bevy_gltf` (`crates/bevy_gltf/src/loader/mod.rs`) computes
    ///     `GltfMaterial::emissive` = `emissiveFactor * KHR_materials_emissive_strength`
    /// and this loader multiplies the remaining factor once:
    ///     `MToonMaterial::emissive` = `GltfMaterial::emissive * VRM_multiplier`
    /// so neither extension is applied twice.
    #[test]
    fn emissive_composes_the_khr_strength_with_the_vrm_multiplier() -> TestResult {
        let extension = extension(r#"{"specVersion":"1.0"}"#)?;

        for (factor, khr_strength, multiplier) in
            [(1.0, 1.0, 1.0), (0.5, 4.0, 2.0), (1.0, 1.0, 10.0)]
        {
            let base = GltfMaterial {
                emissive: LinearRgba::rgb(factor, factor, factor) * khr_strength,
                ..Default::default()
            };
            let material = mtoon_material(&mut |_| None, &base, &extension, multiplier);

            assert_eq!(
                material.emissive.red,
                factor * khr_strength * multiplier,
                "the multiplier applies once, on top of the KHR strength"
            );
        }

        success!()
    }

    /// `VRMC_materials_hdr_emissiveMultiplier` is a *sibling* of
    /// `VRMC_materials_mtoon`, and a material without it keeps a multiplier of
    /// `1.0` — which is what `from_material_extensions`' `None` maps to.
    #[test]
    fn an_absent_hdr_multiplier_leaves_the_emissive_alone() -> TestResult {
        let extension = extension(r#"{"specVersion":"1.0"}"#)?;
        let base = GltfMaterial {
            emissive: LinearRgba::rgb(0.5, 0.25, 0.125),
            ..Default::default()
        };
        let material = mtoon_material(&mut |_| None, &base, &extension, 1.0);

        assert_eq!(material.emissive, base.emissive);
        success!()
    }

    /// A handle to a texture the file does not have never loads, and a material
    /// waiting on it never renders; such a reference reads as no texture.
    #[test]
    fn an_out_of_range_texture_reads_as_no_texture() -> TestResult {
        assert_eq!(existing_texture(Some(1), 2), Some(1));
        assert_eq!(existing_texture(Some(2), 2), None);
        assert_eq!(existing_texture(None, 2), None);
        success!()
    }

    #[test]
    fn the_outline_modes_the_shader_can_draw_are_kept() -> TestResult {
        let world = extension(
            r#"{"specVersion":"1.0","outlineWidthMode":"worldCoordinates","outlineWidthFactor":0.5}"#,
        )?;
        let outline = MToonOutline::from(&world);
        assert_eq!(outline.mode, OutlineWidthMode::WorldCoordinates);
        assert_eq!(outline.width_factor, 0.5);

        // The default is "no outline", and a width factor alone must not
        // conjure one.
        let none = extension(r#"{"specVersion":"1.0","outlineWidthFactor":0.5}"#)?;
        assert_eq!(MToonOutline::from(&none).mode, OutlineWidthMode::None);

        // Screen-space outlines are unsupported; the warning is emitted by
        // `process_material` and the mode degrades to "none" here.
        let screen = extension(
            r#"{"specVersion":"1.0","outlineWidthMode":"screenCoordinates","outlineWidthFactor":0.5}"#,
        )?;
        assert_eq!(MToonOutline::from(&screen).mode, OutlineWidthMode::None);
        success!()
    }

    #[test]
    fn the_pbr_values_are_copied_from_the_gltf_material() -> TestResult {
        let extension = extension(
            r#"{"specVersion":"1.0","renderQueueOffsetNumber":3.0,"transparentWithZWrite":true}"#,
        )?;
        let base = GltfMaterial {
            double_sided: true,
            alpha_mode: AlphaMode::Mask(0.25),
            ..Default::default()
        };

        let material = mtoon_material(&mut |_| None, &base, &extension, 1.0);

        assert!(material.double_sided);
        assert_eq!(material.alpha_mode, AlphaMode::Mask(0.25));
        assert_eq!(material.cull_mode, base.cull_mode);
        assert_eq!(material.render_queue_offset, 3.0);
        assert!(material.transparent_with_z_write);
        // glTF declares neither, and `bevy_pbr`'s own `GltfMaterial` ->
        // `StandardMaterial` conversion leaves both at their defaults.
        assert_eq!(material.depth_bias, 0.0);
        assert_eq!(
            material.opaque_renderer_method,
            OpaqueRendererMethod::default()
        );
        success!()
    }
}
