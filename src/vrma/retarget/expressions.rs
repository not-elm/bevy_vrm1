//!  This module handles the retargeting of expressions from a VRM model to a mascot model.

use crate::system_param::child_searcher::ChildSearcher;
use crate::vrm::expressions::VrmExpressionRegistry;
use crate::vrm::VrmExpression;
use crate::vrma::gltf::extensions::VrmaExtensions;
use crate::vrma::retarget::{CurrentRetargeting, RetargetBindingSystemSet};
use crate::vrma::{RetargetSource, RetargetTo};
use bevy::app::{App, Update};
use bevy::prelude::*;

pub(super) struct VrmaRetargetExpressionsPlugin;

impl Plugin for VrmaRetargetExpressionsPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_systems(
            Update,
            (
                retarget_expressions_to_mascot,
                bind_expressions.in_set(RetargetBindingSystemSet),
            ),
        );

        app.register_type::<RetargetExpressionTo>()
            .register_type::<BindExpressionNode>();
    }
}

#[derive(Component, Deref, Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct VrmaExpressionNames(Vec<VrmExpression>);

impl VrmaExpressionNames {
    pub fn new(extensions: &VrmaExtensions) -> Self {
        let Some(expressions) = extensions.vrmc_vrm_animation.expressions.as_ref() else {
            return Self(Vec::default());
        };
        Self(
            expressions
                .preset
                .keys()
                .map(|expression| VrmExpression(expression.clone()))
                .collect(),
        )
    }
}

#[derive(Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
struct BindExpressionNode {
    pub expression_entity: Entity,
    pub index: usize,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
struct RetargetExpressionTo(Vec<BindExpressionNode>);

fn retarget_expressions_to_mascot(
    mut commands: Commands,
    vrma: Query<(Entity, &RetargetTo, &VrmaExpressionNames), Added<Children>>,
    mascots: Query<&VrmExpressionRegistry>,
    searcher: ChildSearcher,
) {
    for (vrma_entity, retarget, expressions) in vrma.iter() {
        let Ok(vrm_expressions) = mascots.get(retarget.0) else {
            continue;
        };
        for expression_name in expressions.iter() {
            let Some(vrma_expression_entity) =
                searcher.find_from_name(vrma_entity, expression_name)
            else {
                debug!("[Expressions] expression entity not found: {expression_name}");
                continue;
            };
            let Some(nodes) = vrm_expressions.get(expression_name) else {
                debug!("[Expressions] expression nodes not found: {expression_name}");
                continue;
            };
            let binds = nodes
                .iter()
                .filter_map(|node| {
                    Some(BindExpressionNode {
                        expression_entity: searcher.find_from_name(retarget.0, &node.name)?,
                        index: node.morph_target_index,
                    })
                })
                .collect::<Vec<_>>();
            commands
                .entity(vrma_expression_entity)
                .insert((RetargetSource, RetargetExpressionTo(binds)));
        }
    }
}

fn bind_expressions(
    mut expressions: Query<&mut MorphWeights>,
    vrma: Query<
        (&Transform, &RetargetExpressionTo),
        (Changed<Transform>, With<CurrentRetargeting>),
    >,
) {
    for (tf, RetargetExpressionTo(binds)) in vrma.iter() {
        // VRMA uses x coordinate to represent expression weight.
        let weight = tf.translation.x;
        for BindExpressionNode {
            expression_entity,
            index,
        } in binds.iter()
        {
            if let Ok(mut morph_weights) = expressions.get_mut(*expression_entity) {
                morph_weights.weights_mut()[*index] = weight;
            }
        }
    }
}
