use crate::vrm::extensions::vrmc_vrm::MorphTargetBind;
use crate::vrm::extensions::VrmExtensions;
use crate::vrm::VrmExpression;
use bevy::app::Plugin;
use bevy::asset::{Assets, Handle};
use bevy::gltf::GltfNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

#[derive(Reflect, Debug, Clone)]
pub struct ExpressionNode {
    pub name: Name,
    pub morph_target_index: usize,
}

#[derive(Component, Deref, Reflect)]
pub struct VrmExpressionRegistry(HashMap<VrmExpression, Vec<ExpressionNode>>);

impl VrmExpressionRegistry {
    pub fn new(
        extensions: &VrmExtensions,
        node_assets: &Assets<GltfNode>,
        nodes: &[Handle<GltfNode>],
    ) -> Self {
        let Some(expressions) = extensions.vrmc_vrm.expressions.as_ref() else {
            return Self(HashMap::default());
        };
        Self(
            expressions
                .preset
                .iter()
                .filter_map(|(preset_name, preset)| {
                    let binds = preset.morph_target_binds.as_ref()?;
                    let node = binds
                        .iter()
                        .filter_map(|bind| convert_to_node(bind, node_assets, nodes))
                        .collect::<Vec<_>>();
                    Some((VrmExpression(preset_name.clone()), node))
                })
                .collect(),
        )
    }
}

pub struct VrmExpressionPlugin;

impl Plugin for VrmExpressionPlugin {
    fn build(
        &self,
        app: &mut bevy::app::App,
    ) {
        app.register_type::<VrmExpressionRegistry>();
    }
}

fn convert_to_node(
    bind: &MorphTargetBind,
    node_assets: &Assets<GltfNode>,
    nodes: &[Handle<GltfNode>],
) -> Option<ExpressionNode> {
    let node_handle = nodes.get(bind.node)?;
    let node = node_assets.get(node_handle)?;
    Some(ExpressionNode {
        name: Name::new(node.name.clone()),
        morph_target_index: bind.index,
    })
}
