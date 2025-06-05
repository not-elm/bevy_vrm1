mod material;
mod outline_pass;
mod setup;

use crate::vrm::gltf::materials::VrmcMaterialsExtensitions;
use crate::vrm::mtoon::outline_pass::MToonOutlinePlugin;
use crate::vrm::mtoon::setup::MToonMaterialSetupPlugin;
use bevy::asset::{load_internal_asset, weak_handle, AssetId};
use bevy::prelude::*;
use std::collections::HashMap;

use crate::prelude::*;

pub mod prelude {
    pub use crate::vrm::mtoon::{material::prelude::*, MtoonMaterialPlugin, VrmcMaterialRegistry};
}

const MTOON_FRAGMENT_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("9a96eff2-1676-1dc0-9abc-2fd5e7134441");
const MTOON_VERTEX_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("f4041db8-c464-b84c-e3c9-e618527945a1");
const MTOON_TYPES_SHADER_HANDLE: Handle<Shader> =
    weak_handle!("5d9302a3-6498-9d2a-fadb-842d01c87697");

pub struct MtoonMaterialPlugin;

impl Plugin for MtoonMaterialPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_plugins(MaterialPlugin::<MToonMaterial>::default())
            .add_plugins((MToonMaterialSetupPlugin, MToonOutlinePlugin));

        #[cfg(feature = "reflect")]
        {
            app.register_type::<MToonMaterial>()
                .register_type::<MToonOutline>()
                .register_type::<VrmcMaterialRegistry>();
        }

        load_internal_asset!(
            app,
            MTOON_FRAGMENT_SHADER_HANDLE,
            "mtoon_fragment.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            MTOON_VERTEX_SHADER_HANDLE,
            "mtoon_vertex.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            MTOON_TYPES_SHADER_HANDLE,
            "mtoon_types.wgsl",
            Shader::from_wgsl
        );
    }
}

#[derive(Component, Default, Debug)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component))]
pub struct VrmcMaterialRegistry {
    pub images: Vec<Handle<Image>>,
    pub materials: HashMap<AssetId<StandardMaterial>, VrmcMaterialsExtensitions>,
}

impl VrmcMaterialRegistry {
    pub fn new(
        gltf: &Gltf,
        images: Vec<Handle<Image>>,
    ) -> Self {
        Self::try_new(gltf, images).unwrap_or_default()
    }

    fn try_new(
        gltf: &Gltf,
        images: Vec<Handle<Image>>,
    ) -> Option<Self> {
        let materials = gltf
            .source
            .as_ref()?
            .materials()
            .flat_map(|m| {
                let asset_id = gltf.named_materials.get(m.name()?)?.id();
                let extensions = m.extensions()?;
                match serde_json::from_value(extensions.get("VRMC_materials_mtoon")?.clone()) {
                    Ok(properties) => Some((asset_id, properties)),
                    Err(e) => {
                        error!("Failed to parse VRMC_materials_mtoon: {e}");
                        None
                    }
                }
            })
            .collect();
        Some(Self { materials, images })
    }
}
