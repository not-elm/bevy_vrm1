mod material;
mod outline_pass;

use crate::prelude::*;
use crate::vrm::mtoon::outline_pass::MToonOutlinePlugin;
use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::prelude::*;

pub mod prelude {
    pub use crate::vrm::mtoon::{MtoonMaterialPlugin, material::prelude::*};
}

const MTOON_FRAGMENT_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("9a96eff2-1676-1dc0-9abc-2fd5e7134443");
const MTOON_VERTEX_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("f4041db8-c464-b84c-e3c9-e618527945a1");
const MTOON_TYPES_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("5d9302a3-6498-9d2a-fadb-842d01c87697");

pub struct MtoonMaterialPlugin;

impl Plugin for MtoonMaterialPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<MToonMaterial>()
            .register_type::<MToonOutline>()
            .register_type::<RimLighting>()
            .register_type::<UVAnimation>()
            .register_type::<Shade>()
            .add_plugins(MaterialPlugin::<MToonMaterial>::default())
            .add_plugins(MToonOutlinePlugin);
        load_internal_asset!(
            app,
            MTOON_FRAGMENT_SHADER_HANDLE,
            "mtoon_fragment.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            MTOON_TYPES_SHADER_HANDLE,
            "mtoon_types.wgsl",
            Shader::from_wgsl
        );
        load_internal_asset!(
            app,
            MTOON_VERTEX_SHADER_HANDLE,
            "mtoon_vertex.wgsl",
            Shader::from_wgsl
        );
    }
}
