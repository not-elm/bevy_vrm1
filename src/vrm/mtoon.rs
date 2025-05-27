mod material;
mod outline;
mod setup;

use crate::vrm::gltf::materials::VrmcMaterialsExtensitions;
use crate::vrm::mtoon::outline::MToonOutlinePlugin;
use crate::vrm::mtoon::setup::MToonMaterialSetupPlugin;
use bevy::asset::{load_internal_asset, weak_handle, AssetId};
use bevy::prelude::*;
use std::collections::HashMap;

pub use material::*;

const MTOON_SHADER_HANDLE: Handle<Shader> = weak_handle!("9a96eff2-1676-1dc0-9abc-2fd5e7134441");

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
                .register_type::<crate::vrm::mtoon::outline::MToonOutline>()
                .register_type::<VrmcMaterialRegistry>();
        }

        load_internal_asset!(app, MTOON_SHADER_HANDLE, "mtoon.wgsl", Shader::from_wgsl);
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
                    Err(_e) => {
                        #[cfg(feature = "log")]
                        {
                            bevy::log::error!("Failed to parse VRMC_materials_mtoon: {_e}");
                        }
                        None
                    }
                }
            })
            .collect();
        Some(Self { materials, images })
    }
}
