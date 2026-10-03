use crate::prelude::ChildSearcher;
use crate::system_set::VrmSystemSets;
use crate::vrm::gltf::extensions::VrmExtensions;
use crate::vrm::gltf::extensions::vrmc_vrm::MorphTargetBind;
use crate::vrm::{Vrm, VrmExpression};
use crate::vrma::RetargetSource;
use bevy::animation::{AnimatedBy, AnimationTargetId};
use bevy::app::Plugin;
use bevy::asset::{Assets, Handle};
use bevy::gltf::GltfNode;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpressionCategory {
    Mouth,
    Blink,
    LookAt,
    Other,
}

impl ExpressionCategory {
    pub fn from_preset_name(name: &str) -> Self {
        match name {
            "aa" | "ih" | "ou" | "ee" | "oh" => Self::Mouth,
            "blink" | "blinkLeft" | "blinkRight" => Self::Blink,
            "lookUp" | "lookDown" | "lookLeft" | "lookRight" => Self::LookAt,
            _ => Self::Other,
        }
    }
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionOverrideType {
    None,
    Block,
    Blend,
}

impl ExpressionOverrideType {
    pub fn rate(
        &self,
        weight: f32,
    ) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Block => {
                if weight > 0.0 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Blend => weight,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "block" => Self::Block,
            "blend" => Self::Blend,
            _ => Self::None,
        }
    }
}

#[derive(Component, Reflect, Debug, Clone)]
#[reflect(Component)]
#[require(EffectiveExpressionWeight)]
pub struct ExpressionOverrideSettings {
    pub override_mouth: ExpressionOverrideType,
    pub override_blink: ExpressionOverrideType,
    pub override_look_at: ExpressionOverrideType,
}

#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Component)]
pub(crate) struct ExpressionCategoryTag(pub ExpressionCategory);

#[derive(Component, Reflect, Debug, Clone, Copy)]
#[reflect(Component)]
pub struct BinaryExpression;

#[derive(Reflect, Debug, Clone)]
pub(crate) struct ExpressionMetadata {
    pub nodes: Vec<ExpressionNode>,
    pub category: ExpressionCategory,
    pub override_settings: ExpressionOverrideSettings,
    pub is_binary: bool,
}

#[derive(Reflect, Debug, Clone)]
pub(crate) struct ExpressionNode {
    pub name: Name,
    pub morph_target_index: usize,
    pub weight: f32,
}

/// Final binary/override-adjusted weight consumed by morph bindings.
/// Read after `VrmSystemSets::Expressions` to apply other expression bindings
/// with exactly the same weight. Updated even for expressions without morphs.
#[derive(Component, Reflect, Debug, Default, Clone, Copy)]
#[reflect(Component)]
pub struct EffectiveExpressionWeight(pub f32);

/// Cached mapping from expression name to expression entity.
/// Built during VRM initialization. Use this to query available expressions.
#[derive(Component, Deref, Reflect)]
pub struct ExpressionEntityMap(pub HashMap<VrmExpression, Entity>);

/// Override weight for a single expression entity.
/// Inserted by [`SetExpressions`] or [`ModifyExpressions`], removed by [`ClearExpressions`].
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct ExpressionOverride(pub f32);

/// Sets expression weights on a VRM model, **replacing all previous overrides**.
///
/// Trigger this event to directly control facial expressions.
/// Expression weights are clamped to `0.0..=1.0`.
/// Expressions not included in this call will return to VRMA animation control.
///
/// For partial updates that preserve existing overrides, see [`ModifyExpressions`].
///
/// **Note**: Triggering both `SetExpressions` and [`ModifyExpressions`]
/// on the same entity in the same frame produces undefined results.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn set_happy(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
///     for vrm in vrms.iter() {
///         commands.trigger(SetExpressions::single(vrm, "happy", 1.0));
///     }
/// }
/// ```
#[derive(EntityEvent, Debug)]
pub struct SetExpressions {
    #[event_target]
    pub entity: Entity,
    pub weights: HashMap<VrmExpression, f32>,
}

impl SetExpressions {
    /// Creates a [`SetExpressions`] event for a single expression.
    pub fn single(
        entity: Entity,
        expression: impl Into<VrmExpression>,
        weight: f32,
    ) -> Self {
        Self {
            entity,
            weights: [(expression.into(), weight)].into_iter().collect(),
        }
    }

    /// Creates a [`SetExpressions`] event from an iterator of expression-weight pairs.
    pub fn from_iter(
        entity: Entity,
        iter: impl IntoIterator<Item = (impl Into<VrmExpression>, f32)>,
    ) -> Self {
        Self {
            entity,
            weights: iter.into_iter().map(|(e, w)| (e.into(), w)).collect(),
        }
    }
}

/// Modifies specific expression weights without affecting others (partial update).
///
/// Unlike [`SetExpressions`] which replaces all overrides,
/// this only inserts/updates the specified expressions.
/// Existing overrides not mentioned in this call remain unchanged.
///
/// This is the equivalent of `UniVRM`'s `SetWeight()` and three-vrm's `setValue()`.
/// Ideal for lip-sync where mouth expressions are updated every frame
/// while other expression overrides (e.g. emotions) remain active.
///
/// **Note**: Triggering both [`SetExpressions`] and `ModifyExpressions`
/// on the same entity in the same frame produces undefined results.
///
/// ```no_run
/// use bevy::prelude::*;
/// use bevy_vrm1::prelude::*;
///
/// fn add_blink(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
///     for vrm in vrms.iter() {
///         // Only modifies "blink", leaves other overrides (e.g. "happy") intact
///         commands.trigger(ModifyExpressions::single(vrm, "blink", 1.0));
///     }
/// }
/// ```
#[derive(EntityEvent, Debug)]
pub struct ModifyExpressions {
    #[event_target]
    pub entity: Entity,
    pub weights: HashMap<VrmExpression, f32>,
}

/// The five VRM preset mouth expressions used for lip-sync.
const MOUTH_EXPRESSIONS: [&str; 5] = ["aa", "ih", "ou", "ee", "oh"];

impl ModifyExpressions {
    /// Creates a [`ModifyExpressions`] event for a single expression.
    pub fn single(
        entity: Entity,
        expression: impl Into<VrmExpression>,
        weight: f32,
    ) -> Self {
        Self {
            entity,
            weights: [(expression.into(), weight)].into_iter().collect(),
        }
    }

    /// Creates a [`ModifyExpressions`] event from an iterator of expression-weight pairs.
    pub fn from_iter(
        entity: Entity,
        iter: impl IntoIterator<Item = (impl Into<VrmExpression>, f32)>,
    ) -> Self {
        Self {
            entity,
            weights: iter.into_iter().map(|(e, w)| (e.into(), w)).collect(),
        }
    }

    /// Sets a single mouth expression for lip-sync, resetting all other mouth
    /// expressions to 0.0.
    ///
    /// This is a convenience method that sets all five VRM preset mouth
    /// expressions (aa, ih, ou, ee, oh) with the specified one active and
    /// the rest at 0.0. Non-mouth expression overrides are preserved.
    ///
    /// Inserts `ExpressionOverride(0.0)` for inactive mouth expressions,
    /// which overrides any VRMA animation value. Use [`ClearExpressions`]
    /// to return all expressions to VRMA control.
    ///
    /// ```no_run
    /// use bevy::prelude::*;
    /// use bevy_vrm1::prelude::*;
    ///
    /// fn lip_sync(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
    ///     for vrm in vrms.iter() {
    ///         commands.trigger(ModifyExpressions::mouth(vrm, "aa", 0.8));
    ///     }
    /// }
    /// ```
    pub fn mouth(
        entity: Entity,
        expression: impl Into<VrmExpression>,
        weight: f32,
    ) -> Self {
        let active = expression.into();
        let mut weights: HashMap<VrmExpression, f32> = MOUTH_EXPRESSIONS
            .iter()
            .map(|&name| (VrmExpression::from(name), 0.0))
            .collect();
        weights.insert(active, weight);
        Self { entity, weights }
    }

    /// Sets multiple mouth expressions for blended lip-sync, resetting
    /// unspecified mouth expressions to 0.0.
    ///
    /// Useful for blend-based lip-sync where multiple vowels are active
    /// simultaneously (e.g. aa=0.3, ih=0.5). Non-mouth expression overrides
    /// are preserved.
    ///
    /// ```no_run
    /// use bevy::prelude::*;
    /// use bevy_vrm1::prelude::*;
    ///
    /// fn blended_lip_sync(mut commands: Commands, vrms: Query<Entity, With<Vrm>>) {
    ///     for vrm in vrms.iter() {
    ///         commands.trigger(ModifyExpressions::mouth_weights(
    ///             vrm,
    ///             [("aa", 0.3), ("ih", 0.5)],
    ///         ));
    ///     }
    /// }
    /// ```
    pub fn mouth_weights(
        entity: Entity,
        iter: impl IntoIterator<Item = (impl Into<VrmExpression>, f32)>,
    ) -> Self {
        let mut weights: HashMap<VrmExpression, f32> = MOUTH_EXPRESSIONS
            .iter()
            .map(|&name| (VrmExpression::from(name), 0.0))
            .collect();
        for (expr, weight) in iter {
            weights.insert(expr.into(), weight);
        }
        Self { entity, weights }
    }
}

/// Clears all expression overrides, returning control to VRMA animation.
///
/// After triggering this event, expressions previously set by [`SetExpressions`]
/// or [`ModifyExpressions`] will be controlled by VRMA animation again.
#[derive(EntityEvent, Debug)]
pub struct ClearExpressions {
    #[event_target]
    pub entity: Entity,
}

#[derive(EntityEvent)]
pub(crate) struct RequestInitializeExpressions(pub(crate) Entity);

#[derive(Reflect)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct BindExpressionNode {
    pub expression_entity: Entity,
    pub index: usize,
    pub weight: f32,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", reflect(Serialize, Deserialize))]
pub(crate) struct RetargetExpressionNodes(pub(crate) Vec<BindExpressionNode>);

#[derive(Component, Deref, Reflect)]
pub(crate) struct VrmExpressionRegistry(pub(crate) HashMap<VrmExpression, ExpressionMetadata>);

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
                .map(|(preset_name, preset)| {
                    let expression_nodes = preset
                        .morph_target_binds
                        .as_ref()
                        .map(|binds| {
                            binds
                                .iter()
                                .filter_map(|bind| convert_to_node(bind, node_assets, nodes))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let metadata = ExpressionMetadata {
                        nodes: expression_nodes,
                        category: ExpressionCategory::from_preset_name(preset_name),
                        override_settings: ExpressionOverrideSettings {
                            override_mouth: ExpressionOverrideType::parse(&preset.override_mouth),
                            override_blink: ExpressionOverrideType::parse(&preset.override_blink),
                            override_look_at: ExpressionOverrideType::parse(
                                &preset.override_look_at,
                            ),
                        },
                        is_binary: preset.is_binary,
                    };
                    (VrmExpression(preset_name.clone()), metadata)
                })
                .collect(),
        )
    }
}

pub(crate) struct VrmExpressionPlugin;

impl Plugin for VrmExpressionPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.register_type::<BindExpressionNode>()
            .register_type::<RetargetExpressionNodes>()
            .register_type::<VrmExpressionRegistry>()
            .register_type::<ExpressionEntityMap>()
            .register_type::<ExpressionOverride>()
            .register_type::<ExpressionOverrideSettings>()
            .register_type::<EffectiveExpressionWeight>()
            .register_type::<ExpressionCategoryTag>()
            .register_type::<BinaryExpression>()
            .add_observer(apply_initialize_expressions)
            .add_observer(apply_set_expressions)
            .add_observer(apply_modify_expressions)
            .add_observer(apply_clear_expressions)
            .add_systems(
                PostUpdate,
                bind_expressions
                    .in_set(VrmSystemSets::Expressions)
                    .after(VrmSystemSets::GazeControl),
            );
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
        weight: bind.weight,
    })
}

fn apply_initialize_expressions(
    trigger: On<RequestInitializeExpressions>,
    mut commands: Commands,
    expressions: Query<&VrmExpressionRegistry>,
    searcher: ChildSearcher,
) {
    let vrm_entity = trigger.event_target();
    let expressions_root = commands.spawn(Name::new(Vrm::EXPRESSIONS_ROOT)).id();
    commands.entity(vrm_entity).add_child(expressions_root);

    let Ok(registry) = expressions.get(vrm_entity) else {
        commands
            .entity(vrm_entity)
            .insert(ExpressionEntityMap(HashMap::default()));
        return;
    };
    let mut entity_map = HashMap::default();
    for (expression, metadata) in registry.iter() {
        let mut entity_commands = commands.spawn((
            Name::new(expression.to_string()),
            RetargetSource,
            Transform::default(),
            AnimationPlayer::default(),
            RetargetExpressionNodes(obtain_expression_nodes(
                vrm_entity,
                &searcher,
                &metadata.nodes,
            )),
            ExpressionCategoryTag(metadata.category),
            metadata.override_settings.clone(),
        ));
        if metadata.is_binary {
            entity_commands.insert(BinaryExpression);
        }
        let expression_entity = entity_commands.id();
        commands.entity(expression_entity).insert((
            AnimationTargetId::from_name(&Name::new(expression.to_string())),
            AnimatedBy(expression_entity),
        ));
        commands
            .entity(expressions_root)
            .add_child(expression_entity);
        entity_map.insert(expression.clone(), expression_entity);
    }
    commands
        .entity(vrm_entity)
        .insert(ExpressionEntityMap(entity_map));
}

fn bind_expressions(
    mut morph_query: Query<&mut MorphWeights>,
    mut rig_expressions: Query<(
        Entity,
        &Transform,
        &RetargetExpressionNodes,
        &ExpressionCategoryTag,
        &ExpressionOverrideSettings,
        Option<&ExpressionOverride>,
        Option<&BinaryExpression>,
        &mut EffectiveExpressionWeight,
    )>,
) {
    // Pass 1: Collect output weights and accumulate override rates.
    // Also collect all mesh entities that need resetting.
    let mut mouth_rate: f32 = 0.0;
    let mut blink_rate: f32 = 0.0;
    let mut look_at_rate: f32 = 0.0;

    struct ExpressionEntry {
        entity: Entity,
        output_weight: f32,
        category: ExpressionCategory,
        is_binary: bool,
        binds: Vec<(Entity, usize, f32)>,
    }

    let mut entries: Vec<ExpressionEntry> = Vec::new();
    let mut mesh_entities: Vec<Entity> = Vec::new();

    for (entity, tf, retarget, category_tag, override_settings, maybe_override, maybe_binary, _) in
        rig_expressions.iter()
    {
        let raw_weight = match maybe_override {
            Some(ExpressionOverride(w)) => *w,
            None => tf.translation.x,
        };
        let is_binary = maybe_binary.is_some();
        let output_weight = if is_binary {
            if raw_weight > 0.5 { 1.0 } else { 0.0 }
        } else {
            raw_weight.clamp(0.0, 1.0)
        };

        mouth_rate += override_settings.override_mouth.rate(output_weight);
        blink_rate += override_settings.override_blink.rate(output_weight);
        look_at_rate += override_settings.override_look_at.rate(output_weight);

        let binds: Vec<(Entity, usize, f32)> = retarget
            .0
            .iter()
            .map(|b| (b.expression_entity, b.index, b.weight))
            .collect();
        for &(entity, _, _) in &binds {
            mesh_entities.push(entity);
        }

        entries.push(ExpressionEntry {
            entity,
            output_weight,
            category: category_tag.0,
            is_binary,
            binds,
        });
    }

    // Pass 2: Compute per-category multipliers.
    let mouth_mul = 1.0 - mouth_rate.clamp(0.0, 1.0);
    let blink_mul = 1.0 - blink_rate.clamp(0.0, 1.0);
    let look_at_mul = 1.0 - look_at_rate.clamp(0.0, 1.0);

    // Pass 3: Reset morph weights, then accumulate.
    mesh_entities.sort_unstable();
    mesh_entities.dedup();
    for &entity in &mesh_entities {
        if let Ok(mut morph_weights) = morph_query.get_mut(entity) {
            for w in morph_weights.weights_mut().iter_mut() {
                *w = 0.0;
            }
        }
    }

    for entry in &entries {
        let multiplier = match entry.category {
            ExpressionCategory::Mouth => mouth_mul,
            ExpressionCategory::Blink => blink_mul,
            ExpressionCategory::LookAt => look_at_mul,
            ExpressionCategory::Other => 1.0,
        };
        let final_weight = if entry.is_binary && multiplier < 1.0 {
            0.0
        } else {
            entry.output_weight * multiplier
        };
        if let Ok((_, _, _, _, _, _, _, mut effective)) = rig_expressions.get_mut(entry.entity) {
            effective.0 = final_weight;
        }
        for &(entity, index, bind_weight) in &entry.binds {
            if let Ok(mut morph_weights) = morph_query.get_mut(entity) {
                morph_weights.weights_mut()[index] += final_weight * bind_weight;
            }
        }
    }
}

fn apply_set_expressions(
    trigger: On<SetExpressions>,
    cache: Query<&ExpressionEntityMap>,
    mut commands: Commands,
) {
    let vrm_entity = trigger.event_target();
    let Ok(map) = cache.get(vrm_entity) else {
        #[cfg(feature = "log")]
        warn!(
            "SetExpressions: ExpressionEntityMap not found for entity {:?}. VRM may not be initialized yet.",
            vrm_entity
        );
        return;
    };
    // Remove overrides not present in the new weights so that
    // each SetExpressions call fully replaces the previous state.
    for (&expr_entity, expression) in map.0.iter().map(|(e, id)| (id, e)) {
        if !trigger.weights.contains_key(expression) {
            commands.entity(expr_entity).remove::<ExpressionOverride>();
        }
    }
    for (expression, weight) in trigger.weights.iter() {
        let Some(&expr_entity) = map.0.get(expression) else {
            #[cfg(feature = "log")]
            warn!("SetExpressions: expression '{}' not found", expression);
            continue;
        };
        commands
            .entity(expr_entity)
            .insert(ExpressionOverride(weight.clamp(0.0, 1.0)));
    }
}

fn apply_modify_expressions(
    trigger: On<ModifyExpressions>,
    cache: Query<&ExpressionEntityMap>,
    mut commands: Commands,
) {
    let vrm_entity = trigger.event_target();
    let Ok(map) = cache.get(vrm_entity) else {
        #[cfg(feature = "log")]
        warn!(
            "ModifyExpressions: ExpressionEntityMap not found for entity {:?}. VRM may not be initialized yet.",
            vrm_entity
        );
        return;
    };
    for (expression, weight) in trigger.weights.iter() {
        let Some(&expr_entity) = map.0.get(expression) else {
            #[cfg(feature = "log")]
            warn!("ModifyExpressions: expression '{}' not found", expression);
            continue;
        };
        commands
            .entity(expr_entity)
            .insert(ExpressionOverride(weight.clamp(0.0, 1.0)));
    }
}

fn apply_clear_expressions(
    trigger: On<ClearExpressions>,
    cache: Query<&ExpressionEntityMap>,
    mut commands: Commands,
) {
    let vrm_entity = trigger.event_target();
    let Ok(map) = cache.get(vrm_entity) else {
        return;
    };
    for &expr_entity in map.0.values() {
        commands.entity(expr_entity).remove::<ExpressionOverride>();
    }
}

fn obtain_expression_nodes(
    vrm_entity: Entity,
    searcher: &ChildSearcher,
    nodes: &[ExpressionNode],
) -> Vec<BindExpressionNode> {
    nodes
        .iter()
        .flat_map(|node| {
            Some(BindExpressionNode {
                expression_entity: searcher.find_from_name(vrm_entity, &node.name)?,
                index: node.morph_target_index,
                weight: node.weight,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::expressions::{
        BinaryExpression, BindExpressionNode, ClearExpressions, ExpressionCategory,
        ExpressionCategoryTag, ExpressionEntityMap, ExpressionMetadata, ExpressionNode,
        ExpressionOverride, ExpressionOverrideSettings, ExpressionOverrideType, ModifyExpressions,
        RequestInitializeExpressions, RetargetExpressionNodes, SetExpressions, VrmExpressionPlugin,
        VrmExpressionRegistry,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;

    fn default_override_settings() -> ExpressionOverrideSettings {
        ExpressionOverrideSettings {
            override_mouth: ExpressionOverrideType::None,
            override_blink: ExpressionOverrideType::None,
            override_look_at: ExpressionOverrideType::None,
        }
    }

    fn simple_metadata(
        name: &str,
        index: usize,
    ) -> ExpressionMetadata {
        ExpressionMetadata {
            nodes: vec![ExpressionNode {
                name: Name::new(name.to_string()),
                morph_target_index: index,
                weight: 1.0,
            }],
            category: ExpressionCategory::Other,
            override_settings: default_override_settings(),
            is_binary: false,
        }
    }

    #[test]
    fn test_obtain_expression_nodes() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((VrmExpressionRegistry(
                [(VrmExpression::from("happy"), simple_metadata("Test", 0))]
                    .into_iter()
                    .collect(),
            ),))
            .with_children(|c| {
                c.spawn(Name::new("Test"));
            })
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestInitializeExpressions);
        app.update();

        app.world_mut()
            .run_system_once(move |s: ChildSearcher| s.find_expressions_root(vrm_entity))
            .expect("Failed to run system")
            .expect("Expression root not found");

        app.world_mut()
            .run_system_once(move |s: ChildSearcher| s.find_from_name(vrm_entity, "happy"))
            .expect("Failed to run system")
            .expect("Expression node not found");
        Ok(())
    }

    #[test]
    fn test_set_expressions() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((VrmExpressionRegistry(
                [(VrmExpression::from("happy"), simple_metadata("Test", 0))]
                    .into_iter()
                    .collect(),
            ),))
            .with_children(|c| {
                c.spawn(Name::new("Test"));
            })
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestInitializeExpressions);
        app.update();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 0.8));
        app.update();

        let map = app.world().get::<ExpressionEntityMap>(vrm_entity).unwrap();
        let expr_entity = *map.0.get(&VrmExpression::from("happy")).unwrap();

        let override_val = app
            .world()
            .get::<ExpressionOverride>(expr_entity)
            .expect("ExpressionOverride not found");
        assert!((override_val.0 - 0.8).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn test_expression_entity_map_built_on_init() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((VrmExpressionRegistry(
                [(VrmExpression::from("happy"), simple_metadata("Test", 0))]
                    .into_iter()
                    .collect(),
            ),))
            .with_children(|c| {
                c.spawn(Name::new("Test"));
            })
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestInitializeExpressions);
        app.update();

        let map = app
            .world()
            .get::<ExpressionEntityMap>(vrm_entity)
            .expect("ExpressionEntityMap not found");

        assert!(map.0.contains_key(&VrmExpression::from("happy")));
        Ok(())
    }

    #[test]
    fn test_clear_expressions() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((VrmExpressionRegistry(
                [(VrmExpression::from("happy"), simple_metadata("Test", 0))]
                    .into_iter()
                    .collect(),
            ),))
            .with_children(|c| {
                c.spawn(Name::new("Test"));
            })
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestInitializeExpressions);
        app.update();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 0.8));
        app.update();

        let map = app.world().get::<ExpressionEntityMap>(vrm_entity).unwrap();
        let expr_entity = *map.0.get(&VrmExpression::from("happy")).unwrap();
        assert!(app.world().get::<ExpressionOverride>(expr_entity).is_some());

        app.world_mut()
            .commands()
            .trigger(ClearExpressions { entity: vrm_entity });
        app.update();

        assert!(app.world().get::<ExpressionOverride>(expr_entity).is_none());
        Ok(())
    }

    #[test]
    fn test_set_expressions_replaces_previous() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((VrmExpressionRegistry(
                [
                    (VrmExpression::from("happy"), simple_metadata("MeshA", 0)),
                    (VrmExpression::from("angry"), simple_metadata("MeshB", 0)),
                ]
                .into_iter()
                .collect(),
            ),))
            .with_children(|c| {
                c.spawn(Name::new("MeshA"));
                c.spawn(Name::new("MeshB"));
            })
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestInitializeExpressions);
        app.update();

        let map = app.world().get::<ExpressionEntityMap>(vrm_entity).unwrap();
        let happy_entity = *map.0.get(&VrmExpression::from("happy")).unwrap();
        let angry_entity = *map.0.get(&VrmExpression::from("angry")).unwrap();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 1.0));
        app.update();

        assert!(
            app.world()
                .get::<ExpressionOverride>(happy_entity)
                .is_some()
        );
        assert!(
            app.world()
                .get::<ExpressionOverride>(angry_entity)
                .is_none()
        );

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "angry", 0.7));
        app.update();

        assert!(
            app.world()
                .get::<ExpressionOverride>(happy_entity)
                .is_none(),
            "Previous expression override should be removed"
        );
        let angry_override = app
            .world()
            .get::<ExpressionOverride>(angry_entity)
            .expect("New expression override not found");
        assert!((angry_override.0 - 0.7).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn test_bind_weight_applied() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0], None)?)
            .id();

        // bind.weight = 0.5, expression weight via transform = 0.8
        // expected: 0.8 * 0.5 = 0.4
        let expression_entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::new(0.8, 0.0, 0.0)),
                RetargetExpressionNodes(vec![BindExpressionNode {
                    expression_entity: mesh_entity,
                    index: 0,
                    weight: 0.5,
                }]),
                ExpressionCategoryTag(ExpressionCategory::Other),
                default_override_settings(),
            ))
            .id();
        let empty_entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::new(0.8, 0.0, 0.0)),
                RetargetExpressionNodes(Vec::new()),
                ExpressionCategoryTag(ExpressionCategory::Other),
                default_override_settings(),
            ))
            .id();
        app.update();

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert!(
            (morph.weights()[0] - 0.4).abs() < f32::EPSILON,
            "Expected 0.4, got {}",
            morph.weights()[0]
        );
        assert_eq!(
            app.world()
                .get::<super::EffectiveExpressionWeight>(expression_entity)
                .unwrap()
                .0,
            0.8
        );
        assert_eq!(
            app.world()
                .get::<super::EffectiveExpressionWeight>(empty_entity)
                .unwrap()
                .0,
            0.8
        );
        Ok(())
    }

    #[test]
    fn test_additive_accumulation() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0], None)?)
            .id();

        // Two expressions targeting the same morph index on the same mesh
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.3, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 0,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            default_override_settings(),
        ));
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.5, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 0,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            default_override_settings(),
        ));
        app.update();

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert!(
            (morph.weights()[0] - 0.8).abs() < f32::EPSILON,
            "Expected additive 0.3 + 0.5 = 0.8, got {}",
            morph.weights()[0]
        );
        Ok(())
    }

    #[test]
    fn test_override_block() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();

        // "happy" expression with overrideMouth=block, weight=1.0
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 0,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            ExpressionOverrideSettings {
                override_mouth: ExpressionOverrideType::Block,
                override_blink: ExpressionOverrideType::None,
                override_look_at: ExpressionOverrideType::None,
            },
        ));
        // "aa" mouth expression, weight=0.7
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.7, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 1,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Mouth),
            default_override_settings(),
        ));
        app.update();

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        // "happy" at index 0: 1.0 (Other, no suppression)
        assert!(
            (morph.weights()[0] - 1.0).abs() < f32::EPSILON,
            "Expected happy=1.0, got {}",
            morph.weights()[0]
        );
        // "aa" at index 1: 0.0 (Mouth suppressed by block, multiplier=0.0)
        assert!(
            (morph.weights()[1] - 0.0).abs() < f32::EPSILON,
            "Expected mouth suppressed to 0.0, got {}",
            morph.weights()[1]
        );
        Ok(())
    }

    #[test]
    fn test_override_blend() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();

        // Expression with overrideMouth=blend, weight=0.6
        // mouthRate += 0.6, mouthMul = 1.0 - 0.6 = 0.4
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.6, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 0,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            ExpressionOverrideSettings {
                override_mouth: ExpressionOverrideType::Blend,
                override_blink: ExpressionOverrideType::None,
                override_look_at: ExpressionOverrideType::None,
            },
        ));
        // Mouth expression, weight=1.0
        // finalWeight = 1.0 * 0.4 = 0.4
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 1,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Mouth),
            default_override_settings(),
        ));
        app.update();

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert!(
            (morph.weights()[0] - 0.6).abs() < f32::EPSILON,
            "Expected 0.6, got {}",
            morph.weights()[0]
        );
        assert!(
            (morph.weights()[1] - 0.4).abs() < f32::EPSILON,
            "Expected mouth attenuated to 0.4, got {}",
            morph.weights()[1]
        );
        Ok(())
    }

    #[test]
    fn test_is_binary() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();

        // Binary expression with raw weight 0.3 → output 0.0
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.3, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 0,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            default_override_settings(),
            BinaryExpression,
        ));
        // Binary expression with raw weight 0.7 → output 1.0
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.7, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 1,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            default_override_settings(),
            BinaryExpression,
        ));
        app.update();

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert!(
            (morph.weights()[0] - 0.0).abs() < f32::EPSILON,
            "Expected binary threshold: 0.3 → 0.0, got {}",
            morph.weights()[0]
        );
        assert!(
            (morph.weights()[1] - 1.0).abs() < f32::EPSILON,
            "Expected binary threshold: 0.7 → 1.0, got {}",
            morph.weights()[1]
        );
        Ok(())
    }

    #[test]
    fn test_modify_expressions_preserves_existing() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((VrmExpressionRegistry(
                [
                    (VrmExpression::from("happy"), simple_metadata("MeshA", 0)),
                    (VrmExpression::from("angry"), simple_metadata("MeshB", 0)),
                ]
                .into_iter()
                .collect(),
            ),))
            .with_children(|c| {
                c.spawn(Name::new("MeshA"));
                c.spawn(Name::new("MeshB"));
            })
            .id();

        app.world_mut()
            .commands()
            .entity(vrm_entity)
            .trigger(RequestInitializeExpressions);
        app.update();

        let map = app.world().get::<ExpressionEntityMap>(vrm_entity).unwrap();
        let happy_entity = *map.0.get(&VrmExpression::from("happy")).unwrap();
        let angry_entity = *map.0.get(&VrmExpression::from("angry")).unwrap();

        // Set happy via SetExpressions
        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 1.0));
        app.update();

        assert!(
            app.world()
                .get::<ExpressionOverride>(happy_entity)
                .is_some()
        );

        // Modify angry — happy override should be preserved
        app.world_mut()
            .commands()
            .trigger(ModifyExpressions::single(vrm_entity, "angry", 0.7));
        app.update();

        // happy override is still present
        let happy_override = app
            .world()
            .get::<ExpressionOverride>(happy_entity)
            .expect("Existing override should be preserved by ModifyExpressions");
        assert!((happy_override.0 - 1.0).abs() < f32::EPSILON);

        // angry override was added
        let angry_override = app
            .world()
            .get::<ExpressionOverride>(angry_entity)
            .expect("ModifyExpressions should add new override");
        assert!((angry_override.0 - 0.7).abs() < f32::EPSILON);
        Ok(())
    }

    #[test]
    fn test_is_binary_override_suppression() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0, 0.0], None)?)
            .id();

        // Expression with overrideBlink=blend, weight=0.3
        // blinkRate += 0.3, blinkMul = 0.7
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(0.3, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 0,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Other),
            ExpressionOverrideSettings {
                override_mouth: ExpressionOverrideType::None,
                override_blink: ExpressionOverrideType::Blend,
                override_look_at: ExpressionOverrideType::None,
            },
        ));
        // Binary blink expression, weight=1.0
        // multiplier=0.7 < 1.0, binary → finalWeight = 0.0
        app.world_mut().spawn((
            Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            RetargetExpressionNodes(vec![BindExpressionNode {
                expression_entity: mesh_entity,
                index: 1,
                weight: 1.0,
            }]),
            ExpressionCategoryTag(ExpressionCategory::Blink),
            default_override_settings(),
            BinaryExpression,
        ));
        app.update();

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert!(
            (morph.weights()[1] - 0.0).abs() < f32::EPSILON,
            "Expected binary blink fully suppressed to 0.0, got {}",
            morph.weights()[1]
        );
        Ok(())
    }
}
