//! Root-level animatable properties and the expression bind pass.
//!
//! The VRM root entity carries a *synthetic* animation target,
//! [`VRM_ROOT_TARGET_NAME`] ([`vrm_root_animation_target`]), so that baked
//! VRMA curves can address the avatar root itself instead of a bone. Expression
//! weights are animated through [`ExpressionWeightProperty`], which reads and
//! writes [`VrmExpressionWeights`] on the root, and set by the application in
//! [`ExpressionOverrides`], which take precedence; [`apply_expression_morph_binds`]
//! distributes those weights into the `MorphWeights` of the bound meshes and
//! publishes the final weight of every expression into
//! [`EffectiveExpressionWeights`], honouring `isBinary` and the `overrideMouth`
//! / `overrideBlink` / `overrideLookAt` rules of the VRM expression spec.
//!
//! # Ownership
//!
//! These components are data only: nothing here spawns entities or schedules
//! systems by itself. The loading pipeline that builds the animation graph owns
//! the lifecycle and **must** `App::register_type` every component below
//! before a scene that contains them is loaded
//! ([`VrmExpressionWeights`], [`EffectiveExpressionWeights`],
//! [`ExpressionMorphBinds`], [`MorphBind`], [`MorphBindTable`],
//! [`ExpressionSetting`], [`ExpressionSettings`]), attach
//! `AnimationTargetId::from_name(&Name::new(VRM_ROOT_TARGET_NAME))` to the VRM
//! root entity, and add [`apply_expression_morph_binds`] to its schedule.

use core::any::TypeId;
use std::sync::{Mutex, OnceLock};

use bevy::animation::AnimationEntityMut;
use bevy::animation::AnimationEvaluationError;
use bevy::animation::AnimationTargetId;
use bevy::animation::animation_curves::{AnimatableProperty, EvaluatorId};
use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::mesh::morph::MorphWeights;
use bevy::platform::collections::HashMap;
use bevy::platform::hash::Hashed;
use bevy::prelude::*;

use crate::vrm::expressions::{ExpressionCategory, ExpressionOverrideType};

/// Name of the synthetic animation target on the VRM root entity.
///
/// The loading pipeline attaches an `AnimationTargetId` built from this name to
/// the VRM root entity, so that curves which address the model as a whole (for
/// example expression weights) have a target to bind to.
pub const VRM_ROOT_TARGET_NAME: &str = "VrmGltfRoot";

/// The `AnimationTargetId` under which root-level properties are animated.
pub fn vrm_root_animation_target() -> AnimationTargetId {
    AnimationTargetId::from_name(&Name::new(VRM_ROOT_TARGET_NAME))
}

/// Animated weights of all expressions (preset + custom); lives on the VRM
/// root, distributed into morph targets by [`apply_expression_morph_binds`].
///
/// `.vrma` expression curves write this; with the `.vrma` plugin every weight
/// returns to `0.0` before the animation is evaluated, so an expression no
/// playing clip drives is neutral. An expression in [`ExpressionOverrides`]
/// is distributed with that weight instead.
#[derive(Component, Reflect, Debug, Clone, Default)]
#[reflect(Component)]
pub struct VrmExpressionWeights(pub HashMap<String, f32>);

/// Weights set by [`SetExpressions`](crate::prelude::SetExpressions) and
/// [`ModifyExpressions`](crate::prelude::ModifyExpressions); inserted on the
/// VRM root by the first of them and removed by
/// [`ClearExpressions`](crate::prelude::ClearExpressions). An expression
/// present here takes this weight instead of its animated
/// [`VrmExpressionWeights`] one.
#[derive(Component, Reflect, Debug, Clone, Default)]
#[reflect(Component)]
pub struct ExpressionOverrides(pub HashMap<String, f32>);

/// Final weights of all expressions, published by
/// [`apply_expression_morph_binds`]: every declared expression's override, else
/// its animated weight, after its `isBinary` threshold and the category
/// suppressors
/// (`overrideMouth` / `overrideBlink` / `overrideLookAt`) resolved to what the
/// pass actually moves the bound morphs by.
///
/// Bindings that are **not** morphs — `materialColorBinds` and
/// `textureTransformBinds` consumers, for example — read this instead of
/// [`VrmExpressionWeights`], so a binding applies exactly the weight the
/// morph targets applied. The map is rewritten on every run of the pass for
/// every declared expression, even when an expression binds no morph at all.
#[derive(Component, Reflect, Debug, Clone, Default)]
#[reflect(Component)]
pub struct EffectiveExpressionWeights(pub HashMap<String, f32>);

/// Key of one expression inside [`VrmExpressionWeights`].
#[derive(Reflect, Debug, Clone)]
pub struct VrmExpressionIndex {
    /// The expression name, as written in `VRMC_vrm.expressions`.
    pub name: String,
}

/// `AnimatableProperty` for a single expression weight.
///
/// The evaluator id must be unique per expression name, or curves of different
/// expressions would share one evaluator and overwrite each other. Names are
/// interned into stable ids; always construct through
/// [`ExpressionWeightProperty::new`].
#[derive(Reflect, Debug, Clone)]
pub struct ExpressionWeightProperty {
    /// The expression this curve drives.
    pub expression: VrmExpressionIndex,
    #[reflect(ignore)]
    evaluator_key: EvaluatorKey,
}

/// Pre-hashed `(TypeId, interned name id)` referenced by `evaluator_id`.
#[derive(Debug, Clone)]
pub struct EvaluatorKey(Hashed<(TypeId, usize)>);

impl Default for EvaluatorKey {
    /// Only used to fill the ignored reflection field; the resulting id is a
    /// placeholder that no curve is ever built from. Use
    /// [`ExpressionWeightProperty::new`].
    fn default() -> Self {
        Self(Hashed::new((
            TypeId::of::<ExpressionWeightProperty>(),
            usize::MAX,
        )))
    }
}

/// Interns expression names to small ids so that an
/// [`ExpressionWeightProperty`] is comparable and cheap to clone.
fn intern_expression_name(name: &str) -> usize {
    static INTERNER: OnceLock<Mutex<std::collections::HashMap<String, usize>>> = OnceLock::new();
    let mut names = INTERNER
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .expect("expression name interner poisoned");
    let next = names.len();
    *names.entry(name.to_owned()).or_insert(next)
}

impl ExpressionWeightProperty {
    /// Creates the animatable property for the expression `name`.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let interned = intern_expression_name(&name);
        Self {
            expression: VrmExpressionIndex { name },
            evaluator_key: EvaluatorKey(Hashed::new((TypeId::of::<Self>(), interned))),
        }
    }
}

impl AnimatableProperty for ExpressionWeightProperty {
    type Property = f32;

    fn evaluator_id(&self) -> EvaluatorId<'_> {
        EvaluatorId::ComponentField(&self.evaluator_key.0)
    }

    fn get_mut<'a>(
        &self,
        entity: &'a mut AnimationEntityMut,
    ) -> Result<&'a mut f32, AnimationEvaluationError> {
        let weights = entity
            .get_mut::<VrmExpressionWeights>()
            .ok_or(AnimationEvaluationError::ComponentNotPresent(TypeId::of::<
                VrmExpressionWeights,
            >(
            )))?
            .into_inner();
        Ok(weights.0.entry(self.expression.name.clone()).or_insert(0.0))
    }
}

/// A morph-target bind of an expression, resolved to the entity that carries
/// `MorphWeights`.
#[derive(Reflect, Debug, Clone)]
pub struct MorphBind {
    /// The mesh entity that owns the `MorphWeights` to be written.
    pub target: Entity,
    /// Index into `MorphWeights::weights`.
    pub index: usize,
    /// The bind's contribution multiplier (`VRMC_vrm.expressions` bind weight).
    pub weight: f32,
}

/// Expression name -> its morph binds; lives on the root. Stores entities,
/// hence the `MapEntities` impl.
#[derive(Component, Reflect, Debug, Clone, Default)]
#[reflect(Component)]
pub struct ExpressionMorphBinds(#[entities] pub MorphBindTable);

/// Newtype so entity remapping goes through `Component::map_entities`.
#[derive(Reflect, Debug, Clone, Default, Deref, DerefMut)]
pub struct MorphBindTable(pub HashMap<String, Vec<MorphBind>>);

impl MapEntities for MorphBindTable {
    fn map_entities<E: EntityMapper>(
        &mut self,
        entity_mapper: &mut E,
    ) {
        for binds in self.0.values_mut() {
            for bind in binds {
                bind.target = entity_mapper.get_mapped(bind.target);
            }
        }
    }
}

/// Per-expression settings from `VRMC_vrm.expressions`.
#[derive(Reflect, Debug, Clone)]
pub struct ExpressionSetting {
    /// `isBinary`: the weight is thresholded to `0.0` or `1.0`.
    pub is_binary: bool,
    /// The preset category the expression belongs to.
    pub category: ExpressionCategory,
    /// `overrideBlink`.
    pub override_blink: ExpressionOverrideType,
    /// `overrideLookAt`.
    pub override_look_at: ExpressionOverrideType,
    /// `overrideMouth`.
    pub override_mouth: ExpressionOverrideType,
}

/// Expression name -> settings; lives on the VRM root.
#[derive(Component, Reflect, Debug, Clone, Default)]
#[reflect(Component)]
pub struct ExpressionSettings(pub HashMap<String, ExpressionSetting>);

/// Per-category suppression multipliers of one model.
struct CategoryMultipliers {
    mouth: f32,
    blink: f32,
    look_at: f32,
}

impl CategoryMultipliers {
    /// The spec applies `1 - clamp(sum of rates)` per category, so all
    /// overriding expressions of one category share a single multiplier.
    fn of(
        settings: &HashMap<String, ExpressionSetting>,
        weight_of: impl Fn(&str) -> f32,
    ) -> Self {
        let mut mouth_rate = 0.0;
        let mut blink_rate = 0.0;
        let mut look_at_rate = 0.0;
        for (name, setting) in settings.iter() {
            let weight = weight_of(name.as_str());
            mouth_rate += setting.override_mouth.rate(weight);
            blink_rate += setting.override_blink.rate(weight);
            look_at_rate += setting.override_look_at.rate(weight);
        }
        Self {
            mouth: 1.0 - mouth_rate.clamp(0.0, 1.0),
            blink: 1.0 - blink_rate.clamp(0.0, 1.0),
            look_at: 1.0 - look_at_rate.clamp(0.0, 1.0),
        }
    }

    fn get(
        &self,
        category: ExpressionCategory,
    ) -> f32 {
        match category {
            ExpressionCategory::Mouth => self.mouth,
            ExpressionCategory::Blink => self.blink,
            ExpressionCategory::LookAt => self.look_at,
            ExpressionCategory::Other => 1.0,
        }
    }
}

/// The output weight of one expression: `isBinary` is thresholded at `0.5`,
/// every other weight is clamped to `0.0..=1.0`.
fn expression_output_weight(
    raw: f32,
    is_binary: bool,
) -> f32 {
    if is_binary {
        if raw > 0.5 { 1.0 } else { 0.0 }
    } else {
        raw.clamp(0.0, 1.0)
    }
}

/// Applies [`ExpressionOverrides`], else [`VrmExpressionWeights`], to the bound
/// `MorphWeights`.
///
/// The expression pipeline of a `.vrm`, in three passes:
///
/// 1. every expression's output weight (`isBinary` threshold, else clamp) is
///    computed, and **one rate per category** is accumulated over all
///    expressions, giving the multipliers `1 - clamp(sum)` of
///    [`CategoryMultipliers`];
/// 2. every mesh touched by a bind has its *whole* `MorphWeights` array reset,
///    so weights left over from a previous frame cannot survive an expression
///    whose weight dropped to zero (this also clears morph indices that no
///    expression binds, which is what the runtime path does);
/// 3. `weight * multiplier * bind.weight` is accumulated per bound index. A
///    *binary* expression whose multiplier is below `1.0` contributes exactly
///    `0.0`, because a suppressed blink must not end up half-closed.
///
/// Out-of-range bind indices are ignored instead of panicking: a VRMA/VRM file
/// may bind a morph index a mesh does not have.
///
/// The per-expression final weight — the number the bound morph actually
/// receives — is published into [`EffectiveExpressionWeights`], so a binding
/// that is not a morph applies exactly the same number. Updated for every
/// declared expression, even one that binds no morph.
pub fn apply_expression_morph_binds(
    mut roots: Query<(
        &VrmExpressionWeights,
        Option<&ExpressionOverrides>,
        &ExpressionMorphBinds,
        &ExpressionSettings,
        Option<&mut EffectiveExpressionWeights>,
    )>,
    mut morphs: Query<&mut MorphWeights>,
) {
    for (weights, overrides, binds, settings, effective) in roots.iter_mut() {
        let overrides = overrides.map(|overrides| &overrides.0);
        // A user-set weight wins over the animated one.
        let raw = |name: &str| {
            overrides
                .and_then(|overrides| overrides.get(name))
                .or_else(|| weights.0.get(name))
                .copied()
                .unwrap_or(0.0)
        };

        // Pass 1: output weights, and the accumulated per-category rates.
        let mut outputs: HashMap<&str, f32> = HashMap::default();
        for (name, setting) in settings.0.iter() {
            outputs.insert(
                name.as_str(),
                expression_output_weight(raw(name.as_str()), setting.is_binary),
            );
        }
        // A weight without a setting (should not happen for a well-formed
        // model) still animates, unsuppressed and non-binary.
        let overridden = overrides.into_iter().flat_map(|overrides| overrides.keys());
        for name in weights.0.keys().chain(overridden) {
            outputs
                .entry(name.as_str())
                .or_insert_with(|| expression_output_weight(raw(name.as_str()), false));
        }
        let multipliers = CategoryMultipliers::of(&settings.0, |name| {
            outputs.get(name).copied().unwrap_or(0.0)
        });

        // The final weight per expression: what the bound morphs receive. A
        // *binary* expression whose multiplier is below `1.0` is fully
        // suppressed (`0.0`), because a suppressed blink must not end up
        // half-closed.
        let finals: HashMap<_, f32> = outputs
            .iter()
            .map(|(name, output)| {
                let setting = settings.0.get(*name);
                let is_binary = setting.is_some_and(|setting| setting.is_binary);
                let category =
                    setting.map_or(ExpressionCategory::Other, |setting| setting.category);
                let multiplier = multipliers.get(category);
                let weight = if is_binary && multiplier < 1.0 {
                    0.0
                } else {
                    *output * multiplier
                };
                (*name, weight)
            })
            .collect();
        if let Some(mut effective) = effective {
            effective.0 = finals
                .iter()
                .map(|(name, weight)| ((*name).to_owned(), *weight))
                .collect();
        }

        // Pass 2: reset every mesh a bind points at.
        let mut touched: Vec<Entity> = binds.0.values().flatten().map(|bind| bind.target).collect();
        touched.sort_unstable();
        touched.dedup();
        for target in touched {
            if let Ok(mut morph) = morphs.get_mut(target) {
                for weight in morph.weights_mut().iter_mut() {
                    *weight = 0.0;
                }
            }
        }

        // Pass 3: accumulate the weighted contributions.
        for (name, bind_list) in binds.0.iter() {
            let Some(weight) = finals.get(name.as_str()).copied() else {
                continue;
            };
            if weight <= 0.0 {
                continue;
            }
            for bind in bind_list {
                if let Ok(mut morph) = morphs.get_mut(bind.target)
                    && let Some(slot) = morph.weights_mut().get_mut(bind.index)
                {
                    *slot += weight * bind.weight;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;

    use super::*;
    use crate::prelude::{
        EffectiveExpressionWeights as ExportedEffective, ExpressionMorphBinds as ExportedBinds,
        ExpressionSetting as ExportedSetting, ExpressionSettings as ExportedSettings,
        VrmExpressionWeights as ExportedWeights,
    };

    fn setting(
        category: ExpressionCategory,
        is_binary: bool,
    ) -> ExpressionSetting {
        ExpressionSetting {
            is_binary,
            category,
            override_blink: ExpressionOverrideType::None,
            override_look_at: ExpressionOverrideType::None,
            override_mouth: ExpressionOverrideType::None,
        }
    }

    /// `setting` with `override*` set to `Blend`, i.e. an expression that
    /// attenuates (rather than blocks) every category it names.
    fn blending(category: ExpressionCategory) -> ExpressionSetting {
        ExpressionSetting {
            override_blink: ExpressionOverrideType::Blend,
            override_look_at: ExpressionOverrideType::Blend,
            override_mouth: ExpressionOverrideType::Blend,
            ..setting(category, false)
        }
    }

    struct Model {
        app: App,
        mesh: Entity,
        root: Entity,
    }

    /// Spawns a root carrying `weights` / `binds` / `settings` (plus the
    /// `EffectiveExpressionWeights` the loader seeds), plus a mesh with
    /// `morphs.len()` morph weights, and runs the bind pass once.
    fn run(
        morph_count: usize,
        weights: &[(&str, f32)],
        binds: &[(&str, &[(usize, f32)])],
        settings: &[(&str, ExpressionSetting)],
    ) -> Model {
        let mut app = crate::tests::test_app();
        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![0.0; morph_count], None).unwrap())
            .id();
        let root = app
            .world_mut()
            .spawn((
                ExportedWeights(
                    weights
                        .iter()
                        .map(|(name, weight)| ((*name).to_owned(), *weight))
                        .collect(),
                ),
                ExportedBinds(MorphBindTable(HashMap::from_iter(binds.iter().map(
                    |(name, entries)| {
                        (
                            (*name).to_owned(),
                            entries
                                .iter()
                                .map(|(index, weight)| MorphBind {
                                    target: mesh,
                                    index: *index,
                                    weight: *weight,
                                })
                                .collect(),
                        )
                    },
                )))),
                ExportedSettings(HashMap::from_iter(
                    settings
                        .iter()
                        .map(|(name, setting)| ((*name).to_owned(), setting.clone())),
                )),
                ExportedEffective::default(),
            ))
            .id();
        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .unwrap();
        Model { app, mesh, root }
    }

    impl Model {
        fn weights(&self) -> &[f32] {
            self.app
                .world()
                .get::<MorphWeights>(self.mesh)
                .unwrap()
                .weights()
        }

        /// The final weight the pass published for `name`.
        fn final_weight(
            &self,
            name: &str,
        ) -> f32 {
            self.app
                .world()
                .get::<EffectiveExpressionWeights>(self.root)
                .expect("the root carries the published final weights")
                .0
                .get(name)
                .copied()
                .expect("the expression is published")
        }
    }

    fn assert_close(
        actual: f32,
        expected: f32,
    ) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn bind_weight_is_applied() {
        // 0.8 * 0.5
        let model = run(
            1,
            &[("happy", 0.8)],
            &[("happy", &[(0, 0.5)])],
            &[("happy", setting(ExpressionCategory::Other, false))],
        );
        assert_close(model.weights()[0], 0.4);
    }

    #[test]
    fn an_override_wins_over_the_animated_weight() {
        let mut model = run(
            2,
            &[("happy", 0.8), ("angry", 0.5)],
            &[("happy", &[(0, 1.0)]), ("angry", &[(1, 1.0)])],
            &[
                ("happy", setting(ExpressionCategory::Other, false)),
                ("angry", setting(ExpressionCategory::Other, false)),
            ],
        );
        model
            .app
            .world_mut()
            .entity_mut(model.root)
            .insert(ExpressionOverrides(HashMap::from([(
                "happy".to_owned(),
                0.0,
            )])));
        model
            .app
            .world_mut()
            .run_system_once(apply_expression_morph_binds)
            .unwrap();

        assert_close(model.weights()[0], 0.0);
        assert_close(model.final_weight("happy"), 0.0);
        // An expression without an override keeps its animated weight.
        assert_close(model.weights()[1], 0.5);
    }

    #[test]
    fn weights_accumulate_additively() {
        let model = run(
            1,
            &[("happy", 0.3), ("angry", 0.5)],
            &[("happy", &[(0, 1.0)]), ("angry", &[(0, 1.0)])],
            &[
                ("happy", setting(ExpressionCategory::Other, false)),
                ("angry", setting(ExpressionCategory::Other, false)),
            ],
        );
        assert_close(model.weights()[0], 0.8);
    }

    #[test]
    fn weights_are_clamped() {
        let model = run(
            2,
            &[("happy", 4.0), ("angry", -3.0)],
            &[("happy", &[(0, 1.0)]), ("angry", &[(1, 1.0)])],
            &[
                ("happy", setting(ExpressionCategory::Other, false)),
                ("angry", setting(ExpressionCategory::Other, false)),
            ],
        );
        assert_close(model.weights()[0], 1.0);
        assert_close(model.weights()[1], 0.0);
    }

    #[test]
    fn override_block_suppresses_the_category() {
        let mut blocking = setting(ExpressionCategory::Other, false);
        blocking.override_mouth = ExpressionOverrideType::Block;
        let model = run(
            2,
            &[("happy", 1.0), ("aa", 0.7)],
            &[("happy", &[(0, 1.0)]), ("aa", &[(1, 1.0)])],
            &[
                ("happy", blocking),
                ("aa", setting(ExpressionCategory::Mouth, false)),
            ],
        );
        // `happy` is `Other`, so it is unaffected.
        assert_close(model.weights()[0], 1.0);
        // `aa` is `Mouth`, and `block` drives the mouth rate to 1.
        assert_close(model.weights()[1], 0.0);
    }

    #[test]
    fn override_blend_attenuates_the_category() {
        let model = run(
            2,
            &[("happy", 0.6), ("aa", 1.0)],
            &[("happy", &[(0, 1.0)]), ("aa", &[(1, 1.0)])],
            &[
                ("happy", blending(ExpressionCategory::Other)),
                ("aa", setting(ExpressionCategory::Mouth, false)),
            ],
        );
        assert_close(model.weights()[0], 0.6);
        // mouth rate 0.6, so mouth multiplier is 1 - 0.6.
        assert_close(model.weights()[1], 0.4);
    }

    /// The rates of *all* overriding expressions of one category add up into a
    /// single multiplier; they are not multiplied per target.
    #[test]
    fn two_overrides_of_one_category_accumulate_their_rates() {
        let model = run(
            3,
            &[("a", 0.5), ("b", 0.5), ("aa", 1.0)],
            &[("a", &[(0, 1.0)]), ("b", &[(1, 1.0)]), ("aa", &[(2, 1.0)])],
            &[
                ("a", blending(ExpressionCategory::Other)),
                ("b", blending(ExpressionCategory::Other)),
                ("aa", setting(ExpressionCategory::Mouth, false)),
            ],
        );
        assert_close(model.weights()[0], 0.5);
        assert_close(model.weights()[1], 0.5);
        // 0.5 + 0.5 = 1.0 -> multiplier 0.0. Per-target factors would leave
        // (1 - 0.5) * (1 - 0.5) = 0.25 here.
        assert_close(model.weights()[2], 0.0);
    }

    #[test]
    fn accumulated_rates_are_clamped_to_one() {
        // `a`, `b` and `c` each blend-override blink; `blink` is the target.
        const BINDS: [(&str, &[(usize, f32)]); 4] = [
            ("a", &[(0, 1.0)]),
            ("b", &[(1, 1.0)]),
            ("c", &[(2, 1.0)]),
            ("blink", &[(3, 1.0)]),
        ];
        let overrides = [
            ("a", blending(ExpressionCategory::Other)),
            ("b", blending(ExpressionCategory::Other)),
            ("c", blending(ExpressionCategory::Other)),
            // `blink` overrides nothing itself.
            ("blink", setting(ExpressionCategory::Blink, false)),
        ];
        // Output weights are 0.0, 1.0 and 1.0 (`c`'s raw 2.0 is clamped), so the
        // blink rate sums to 2.0 and is clamped to 1.0 -> multiplier 0.0.
        let weights = [("a", 0.0), ("b", 1.0), ("c", 2.0), ("blink", 1.0)];
        let model = run(4, &weights, &BINDS, &overrides);
        assert_close(model.weights()[3], 0.0);
        // The overriding expressions are `Other`, so they are not suppressed.
        assert_close(model.weights()[0], 0.0);
        assert_close(model.weights()[1], 1.0);
        assert_close(model.weights()[2], 1.0);
    }

    #[test]
    fn binary_expressions_are_thresholded() {
        let model = run(
            2,
            &[("a", 0.3), ("b", 0.7)],
            &[("a", &[(0, 1.0)]), ("b", &[(1, 1.0)])],
            &[
                ("a", setting(ExpressionCategory::Other, true)),
                ("b", setting(ExpressionCategory::Other, true)),
            ],
        );
        assert_close(model.weights()[0], 0.0);
        assert_close(model.weights()[1], 1.0);
    }

    /// A suppressed binary expression must not end up half closed.
    #[test]
    fn suppressed_binary_expressions_are_fully_suppressed() {
        let model = run(
            2,
            &[("happy", 0.3), ("blink", 1.0)],
            &[("happy", &[(0, 1.0)]), ("blink", &[(1, 1.0)])],
            &[
                ("happy", blending(ExpressionCategory::Other)),
                ("blink", setting(ExpressionCategory::Blink, true)),
            ],
        );
        assert_close(model.weights()[0], 0.3);
        // blink multiplier is 0.7 < 1.0, and `blink` is binary.
        assert_close(model.weights()[1], 0.0);
    }

    /// An unsuppressed binary expression keeps its `1.0`.
    #[test]
    fn unsuppressed_binary_expressions_are_not_attenuated() {
        let model = run(
            1,
            &[("blink", 0.9)],
            &[("blink", &[(0, 1.0)])],
            &[("blink", setting(ExpressionCategory::Blink, true))],
        );
        assert_close(model.weights()[0], 1.0);
    }

    /// Every morph weight of a touched mesh is reset, so a stale value from a
    /// previous frame cannot survive, and unbound indices are cleared too.
    #[test]
    fn touched_meshes_are_reset_completely() {
        let mut app = crate::tests::test_app();
        let mesh = app
            .world_mut()
            .spawn(MorphWeights::new(vec![1.0, 1.0, 1.0], None).unwrap())
            .id();
        app.world_mut().spawn((
            VrmExpressionWeights(HashMap::from([("happy".to_owned(), 0.0)])),
            ExpressionMorphBinds(MorphBindTable(HashMap::from([(
                "happy".to_owned(),
                vec![MorphBind {
                    target: mesh,
                    index: 0,
                    weight: 1.0,
                }],
            )]))),
            ExpressionSettings(HashMap::from([(
                "happy".to_owned(),
                setting(ExpressionCategory::Other, false),
            )])),
        ));
        app.world_mut()
            .run_system_once(apply_expression_morph_binds)
            .unwrap();
        let weights = app.world().get::<MorphWeights>(mesh).unwrap().weights();
        for (index, weight) in weights.iter().enumerate() {
            assert_eq!(*weight, 0.0, "morph index {index} was not reset");
        }
    }

    #[test]
    fn out_of_range_binds_are_ignored() {
        let model = run(
            1,
            &[("happy", 1.0)],
            &[("happy", &[(7, 1.0)])],
            &[("happy", setting(ExpressionCategory::Other, false))],
        );
        assert_eq!(model.weights(), &[0.0]);
    }

    #[test]
    fn each_name_gets_its_own_evaluator_id() {
        // `EvaluatorId` has no `Debug`, so compare the pre-hashed key it wraps.
        fn key(property: &ExpressionWeightProperty) -> u64 {
            match property.evaluator_id() {
                EvaluatorId::ComponentField(key) => key.hash(),
                EvaluatorId::Type(_) => panic!("expected an EvaluatorId::ComponentField"),
            }
        }

        let happy = ExpressionWeightProperty::new("happy");
        let angry = ExpressionWeightProperty::new("angry");
        let happy_again = ExpressionWeightProperty::new("happy");
        assert_ne!(key(&happy), key(&angry));
        assert_eq!(key(&happy), key(&happy_again));
        assert_eq!(happy.expression.name, "happy");
    }

    #[test]
    fn animating_a_weight_inserts_a_zero_first() {
        let mut app = crate::tests::test_app();
        let root = app.world_mut().spawn(VrmExpressionWeights::default()).id();
        let mut query = app.world_mut().query::<AnimationEntityMut>();
        let mut entity = query.get_mut(app.world_mut(), root).unwrap();
        let property = ExpressionWeightProperty::new("happy");
        *property.get_mut(&mut entity).unwrap() = 0.75;
        assert_eq!(
            app.world().get::<VrmExpressionWeights>(root).unwrap().0["happy"],
            0.75
        );
    }

    #[test]
    fn root_animation_target_is_stable() {
        assert_eq!(
            vrm_root_animation_target(),
            AnimationTargetId::from_name(&Name::new(VRM_ROOT_TARGET_NAME))
        );
    }

    #[test]
    fn binds_are_remapped() {
        let source = Entity::from_raw_u32(1).unwrap();
        let target = Entity::from_raw_u32(2).unwrap();
        let mut table = MorphBindTable(HashMap::from([(
            "happy".to_owned(),
            vec![MorphBind {
                target: source,
                index: 0,
                weight: 1.0,
            }],
        )]));
        // `(Entity, Entity)` is a bevy `EntityMapper`.
        table.map_entities(&mut (source, target));
        assert_eq!(table.0["happy"][0].target, target);
    }

    /// [`ExportedSetting`] is the same type, so the prelude export is usable.
    #[test]
    fn settings_are_reachable_from_the_crate_prelude() {
        let _: ExportedSetting = setting(ExpressionCategory::Blink, true);
        let _: ExportedWeights = VrmExpressionWeights::default();
        let _: ExportedBinds = ExpressionMorphBinds::default();
        let _: ExportedSettings = ExpressionSettings::default();
        let _: ExportedEffective = EffectiveExpressionWeights::default();
    }

    /// The published final weight is the weight the bindings consume: the raw
    /// weight thresholded and de-suppressed, but *before* the per-bind scale
    /// (`bind.weight`), which is per bind. Here `happy` weighs `0.8`, and its
    /// only bind scales to `0.8 * 0.5 = 0.4`.
    #[test]
    fn the_published_final_weight_matches_the_morph() {
        let model = run(
            1,
            &[("happy", 0.8)],
            &[("happy", &[(0, 0.5)])],
            &[("happy", setting(ExpressionCategory::Other, false))],
        );
        assert_close(model.weights()[0], 0.4);
        assert_close(model.final_weight("happy"), 0.8);
    }

    /// An expression that binds no morph is still published, with its
    /// thresholded/clamped weight, so a material or UV bind can consume it.
    #[test]
    fn an_expression_without_binds_is_still_published() {
        let model = run(
            1,
            &[("happy", 0.8), ("solo", 1.6)],
            &[("happy", &[(0, 0.5)])],
            &[("happy", setting(ExpressionCategory::Other, false))],
        );
        assert_close(model.final_weight("solo"), 1.0);
    }

    /// A binary expression suppressed by an overriding expression publishes
    /// exactly `0.0`, matching the `0.0` the suppressed morph receives.
    #[test]
    fn a_suppressed_binary_expression_publishes_zero() {
        let mut blocking = setting(ExpressionCategory::Other, false);
        blocking.override_mouth = ExpressionOverrideType::Block;
        blocking.override_blink = ExpressionOverrideType::Block;
        let model = run(
            1,
            &[("happy", 1.0), ("aa", 0.7), ("blink", 1.0)],
            &[("aa", &[(0, 1.0)])],
            &[
                ("happy", blocking),
                ("aa", setting(ExpressionCategory::Mouth, false)),
                ("blink", setting(ExpressionCategory::Blink, true)),
            ],
        );
        // `aa` is throttled; `blink` is a suppressed binary expression.
        assert_close(model.weights()[0], 0.0);
        assert_close(model.final_weight("aa"), 0.0);
        assert_close(model.final_weight("blink"), 0.0);
        assert_close(model.final_weight("happy"), 1.0);
    }
}
