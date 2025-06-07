//! This module provides the functionality to load VRMA files.

use bevy::app::{App, Plugin};
use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, LoadContext};
use bevy::gltf::{Gltf, GltfError, GltfLoader, GltfLoaderSettings};
use bevy::image::CompressedImageFormats;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::utils::default;

pub(super) struct VrmaLoaderPlugin;

impl Plugin for VrmaLoaderPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.init_asset::<VrmaAsset>()
            .preregister_asset_loader::<VrmaLoader>(&["vrma"]);
    }

    fn finish(
        &self,
        app: &mut App,
    ) {
        let supported_compressed_formats = match app.world().get_resource::<RenderDevice>() {
            Some(render_device) => CompressedImageFormats::from_features(render_device.features()),
            None => CompressedImageFormats::NONE,
        };
        app.register_asset_loader(VrmaLoader(GltfLoader {
            supported_compressed_formats,
            custom_vertex_attributes: Default::default(),
        }));
    }
}

/// Represents a VRMA asset.
/// You can load it using [`AssetServer`].
///
///```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
/// use bevy_vrm1::vrma::VrmaHandle;
///
/// fn spawn_vrma(
///    mut commands: Commands,
///    asset_server: Res<AssetServer>,
/// ){
///    commands.spawn(VrmaHandle(asset_server.load("<vrma>.vrma")));
/// }
/// ```
#[derive(Debug, Asset, TypePath)]
pub struct VrmaAsset {
    pub gltf: Gltf,
}

struct VrmaLoader(GltfLoader);

impl AssetLoader for VrmaLoader {
    type Asset = VrmaAsset;
    type Settings = ();
    type Error = GltfError;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let settings = GltfLoaderSettings {
            include_source: true,
            ..default()
        };
        let gltf = self.0.load(reader, &settings, load_context).await?;
        Ok(VrmaAsset { gltf })
    }

    fn extensions(&self) -> &[&str] {
        &["vrma"]
    }
}
