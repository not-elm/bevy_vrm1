use crate::prelude::{ExpressionSetting, ExpressionSettings, VrmExpressionWeights};
use crate::vrm::{Vrm, VrmExpression};
use bevy::app::Plugin;
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
    /// Inserts a `0.0` weight for inactive mouth expressions,
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

pub(crate) struct VrmExpressionPlugin;

impl Plugin for VrmExpressionPlugin {
    fn build(
        &self,
        app: &mut App,
    ) {
        app.add_observer(apply_set_expressions)
            .add_observer(apply_modify_expressions)
            .add_observer(apply_clear_expressions);
    }
}

/// Clamps the weights and drops the expressions the model does not declare.
fn raw_expression_weights(
    weights: &HashMap<VrmExpression, f32>,
    known: Option<&HashMap<String, ExpressionSetting>>,
    trigger: &str,
) -> HashMap<String, f32> {
    weights
        .iter()
        .filter_map(|(expression, weight)| {
            let name = expression.as_str();
            if let Some(known) = known
                && !known.contains_key(name)
            {
                #[cfg(feature = "log")]
                warn!("{trigger}: expression '{name}' not found");
                return None;
            }
            Some((name.to_owned(), weight.clamp(0.0, 1.0)))
        })
        .collect()
}

fn apply_set_expressions(
    trigger: On<SetExpressions>,
    mut load_time: Query<(&mut VrmExpressionWeights, Option<&ExpressionSettings>)>,
) {
    let vrm_entity = trigger.event_target();
    let Ok((mut weights, settings)) = load_time.get_mut(vrm_entity) else {
        #[cfg(feature = "log")]
        warn!(
            "SetExpressions: VrmExpressionWeights not found for entity {:?}. VRM may not be initialized yet.",
            vrm_entity
        );
        return;
    };
    weights.0 = raw_expression_weights(
        &trigger.weights,
        settings.map(|settings| &settings.0),
        "SetExpressions",
    );
}

fn apply_modify_expressions(
    trigger: On<ModifyExpressions>,
    mut load_time: Query<(&mut VrmExpressionWeights, Option<&ExpressionSettings>)>,
) {
    let vrm_entity = trigger.event_target();
    let Ok((mut weights, settings)) = load_time.get_mut(vrm_entity) else {
        #[cfg(feature = "log")]
        warn!(
            "ModifyExpressions: VrmExpressionWeights not found for entity {:?}. VRM may not be initialized yet.",
            vrm_entity
        );
        return;
    };
    weights.0.extend(raw_expression_weights(
        &trigger.weights,
        settings.map(|settings| &settings.0),
        "ModifyExpressions",
    ));
}

fn apply_clear_expressions(
    trigger: On<ClearExpressions>,
    mut load_time: Query<&mut VrmExpressionWeights>,
) {
    let vrm_entity = trigger.event_target();
    let Ok(mut weights) = load_time.get_mut(vrm_entity) else {
        return;
    };
    weights.0.clear();
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::success;
    use crate::tests::{TestResult, test_app};
    use crate::vrm::expressions::{
        ClearExpressions, ExpressionCategory, ExpressionOverrideType, ModifyExpressions,
        SetExpressions, VrmExpressionPlugin,
    };
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::morph::MorphWeights;
    use bevy::platform::collections::HashMap;
    use bevy::prelude::*;

    fn load_time_settings(names: &[&str]) -> ExpressionSettings {
        ExpressionSettings(
            names
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        ExpressionSetting {
                            is_binary: false,
                            category: ExpressionCategory::from_preset_name(name),
                            override_blink: ExpressionOverrideType::None,
                            override_look_at: ExpressionOverrideType::None,
                            override_mouth: ExpressionOverrideType::None,
                        },
                    )
                })
                .collect(),
        )
    }

    fn load_time_weights(names: &[&str]) -> VrmExpressionWeights {
        VrmExpressionWeights(names.iter().map(|name| ((*name).to_owned(), 0.0)).collect())
    }

    fn weights_of(
        app: &App,
        vrm_entity: Entity,
    ) -> HashMap<String, f32> {
        app.world()
            .get::<VrmExpressionWeights>(vrm_entity)
            .expect("the root carries the expression weights")
            .0
            .clone()
    }

    #[test]
    fn test_set_expressions() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry", "aa"]),
                load_time_settings(&["happy", "angry", "aa"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 0.8));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights.len(), 1, "SetExpressions replaces all weights");
        assert_eq!(weights["happy"], 0.8);
        success!()
    }

    #[test]
    fn test_set_expressions_replaces_previous() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry"]),
                load_time_settings(&["happy", "angry"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 1.0));
        app.update();
        assert_eq!(weights_of(&app, vrm_entity)["happy"], 1.0);

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "angry", 0.7));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert!(
            !weights.contains_key("happy"),
            "the previous weight is dropped, leaving the expression to animation"
        );
        assert_eq!(weights["angry"], 0.7);
        success!()
    }

    #[test]
    fn test_set_expressions_clamps_and_skips_unknown_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy"]),
                load_time_settings(&["happy"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::from_iter(
                vrm_entity,
                [("happy", 4.0), ("nope", 1.0)],
            ));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights["happy"], 1.0, "the weight is clamped to 1.0");
        assert!(
            !weights.contains_key("nope"),
            "an expression the model does not declare is not written"
        );
        success!()
    }

    #[test]
    fn test_modify_expressions_preserves_existing() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry"]),
                load_time_settings(&["happy", "angry"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 1.0));
        app.update();

        app.world_mut()
            .commands()
            .trigger(ModifyExpressions::single(vrm_entity, "angry", 0.7));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights["happy"], 1.0, "the existing weight is preserved");
        assert_eq!(weights["angry"], 0.7, "the named weight is merged in");
        success!()
    }

    #[test]
    fn test_modify_expressions_mouth_resets_the_other_vowels_on_load_time_weights() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        const NAMES: [&str; 6] = ["happy", "aa", "ih", "ou", "ee", "oh"];
        let vrm_entity = app
            .world_mut()
            .spawn((load_time_weights(&NAMES), load_time_settings(&NAMES)))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "happy", 0.6));
        app.update();

        app.world_mut()
            .commands()
            .trigger(ModifyExpressions::mouth(vrm_entity, "aa", 0.8));
        app.update();

        let weights = weights_of(&app, vrm_entity);
        assert_eq!(weights["aa"], 0.8, "the active vowel");
        for vowel in ["ih", "ou", "ee", "oh"] {
            assert_eq!(
                weights[vowel], 0.0,
                "`ModifyExpressions::mouth` resets {vowel}"
            );
        }
        assert_eq!(weights["happy"], 0.6, "a non-mouth weight is preserved");
        success!()
    }

    #[test]
    fn test_clear_expressions() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["happy", "angry"]),
                load_time_settings(&["happy", "angry"]),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::from_iter(
                vrm_entity,
                [("happy", 0.8), ("angry", 0.5)],
            ));
        app.update();
        assert_eq!(weights_of(&app, vrm_entity).len(), 2);

        app.world_mut()
            .commands()
            .trigger(ClearExpressions { entity: vrm_entity });
        app.update();

        assert!(
            weights_of(&app, vrm_entity).is_empty(),
            "every weight is cleared"
        );
        assert!(
            app.world()
                .get::<VrmExpressionWeights>(vrm_entity)
                .is_some(),
            "the component stays: the bind pass needs it"
        );
        success!()
    }

    #[test]
    fn test_triggered_load_time_weights_are_distributed_by_the_bind_pass() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let mesh_entity = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0], None)?)
            .id();
        let vrm_entity = app
            .world_mut()
            .spawn((
                load_time_weights(&["blink"]),
                ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                    "blink".to_owned(),
                    vec![MorphBind {
                        target: mesh_entity,
                        index: 0,
                        weight: 1.0,
                    }],
                )]))),
                ExpressionSettings(HashMap::from([(
                    "blink".to_owned(),
                    ExpressionSetting {
                        is_binary: true,
                        category: ExpressionCategory::Blink,
                        override_blink: ExpressionOverrideType::None,
                        override_look_at: ExpressionOverrideType::None,
                        override_mouth: ExpressionOverrideType::None,
                    },
                )])),
            ))
            .id();

        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(vrm_entity, "blink", 0.3));
        app.update();

        assert_eq!(
            weights_of(&app, vrm_entity)["blink"],
            0.3,
            "the trigger stores the weight verbatim"
        );

        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .expect("the bind pass runs");

        let morph = app.world().get::<MorphWeights>(mesh_entity).unwrap();
        assert_eq!(
            morph.weights()[0],
            0.0,
            "`isBinary` thresholds 0.3 to 0.0, in the bind pass"
        );
        success!()
    }

    #[test]
    fn a_trigger_on_an_entity_without_weights_is_ignored() -> TestResult {
        let mut app = test_app();
        app.add_plugins(VrmExpressionPlugin);

        let entity = app.world_mut().spawn(Name::new("not-a-vrm")).id();
        app.world_mut()
            .commands()
            .trigger(SetExpressions::single(entity, "happy", 1.0));
        app.update();

        assert!(app.world().get::<Name>(entity).is_some());
        success!()
    }
}
