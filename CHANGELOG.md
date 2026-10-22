## v0.11.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.11.0)

A `.vrm` is now initialised **while it loads**, through bevy 0.19's `GltfExtensionHandler` pipeline, instead of by spawning a plain glTF scene and patching it over several frames. `VrmGltfPlugin` claims the `vrm` file extension and writes every VRM component into the scene `WorldAsset`; the handler's computations stay at load time, and a thin scene-ready step binds that root data onto the entity the app spawned. `VrmPlugin` stays the runtime half (spring bones, gaze control, expressions, node constraints, `MToon`) and `VrmaPlugin` is unchanged.

The runtime path was removed rather than deprecated, so several public items are gone. See **Breaking Changes** below for the migration.

### Breaking Changes

- **`VrmHandle` wraps `Handle<Gltf>` and is retained after load.** Spawn `commands.spawn((VrmHandle(asset_server.load("vrm/Elmer.vrm")), ..))`; the entity you spawn **is** the VRM root. It receives `Vrm`, `Initialized` and every root component once the scene is bound, and it keeps its id — plus the components and children on it — across a hot reload or a changed handle. `VrmHandle` requires a `Transform` and `Visibility` and stays on the entity afterwards, where it marks the VRM as crate-managed. `VrmGltfPlugin` is required in the app — it is the only plugin that claims the `vrm` extension, so it is the only thing that can load a `.vrm`
- **Removed `VrmAsset`** and its loader. A raw `WorldAssetRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path)))` spawn still loads a `.vrm`, with the VRM root as its child
- An app needs both halves of the pipeline: `App::new().add_plugins((DefaultPlugins, VrmPlugin, VrmGltfPlugin))`. `VrmGltfPlugin` must come after `DefaultPlugins`, because it registers its handler in the resource `GltfPlugin` prepares and `finish` reuses
- **Removed the `VrmInitializePlugin` step and the `RequestInitialize*` flow** behind it (humanoid bones, spring bones, node constraints, expressions), together with `VrmLoaderPlugin`. The `HumanoidBoneRegistry` name lookup no longer runs for a `.vrm`: references between glTF nodes (`VRMC_node_constraint`, `VRMC_springBone`, `VRMC_vrm.firstPerson`, `morphTargetBinds`) are resolved inside the loader, where every referenced entity already exists. `HumanoidBoneRegistry` still exists, but only for the **VRMA source rig**, which carries node references rather than names
- **Removed `FirstPersonRegistry`**. The `VRMC_vrm.firstPerson` classification, including the `auto` head split, now happens at load time; `RequestEnableFirstPerson` / `RequestDisableFirstPerson` only switch the render layers recorded for each mesh
- **Removed the node-constraint destination registries** `RotationConstraintDestinations`, `RollConstraintDestinations` and `AimConstraintDestinations` from the prelude. `VRMC_node_constraint` now attaches a `VrmNodeConstraint` per destination inside the scene asset, and one ordered pass (`apply_node_constraints`, added by `VrmGltfRuntimePlugin`) evaluates them in the VRM update order. The `VrmNodeConstraintPlugin` / `NodeConstraintInitializePlugin` pair that used to bind them is gone with it
- **Removed the spring-bone runtime registries** that mapped bone names to joint properties and collider shapes (`SpringJointPropsRegistry`, `SpringColliderRegistry`, `SpringNodeRegistry`, `SpringBoneInitializePlugin`). `SpringRoot`, `SpringJoints`, `SpringJointProps`, `SpringJointState`, `SpringColliders`, `SpringCenterNode` and `ColliderShape` are attached by the loader instead, and their entity references now remap when the scene is instantiated
- **Removed the legacy expression shape**: `ExpressionEntityMap`, `ExpressionOverride`, `ExpressionOverrideSettings` and `BinaryExpression` from the prelude, plus the internal `ExpressionCategoryTag`, `RetargetExpressionNodes` and `VrmExpressionRegistry`, together with the per-expression entity tree they described. `SetExpressions` and `ModifyExpressions` now write `ExpressionOverrides` on the VRM root, which take precedence over the animated `VrmExpressionWeights` as `ExpressionOverride` did, and `ClearExpressions` removes them; a trigger aimed at an entity that carries no `VrmExpressionWeights` is a no-op. Thresholding `isBinary`, the `overrideMouth` / `overrideBlink` / `overrideLookAt` rules and the bind-weight multiplication all happen in the single `apply_expression_morph_binds` pass
- **Removed `VrmcMaterialRegistry`** from the prelude, and with it the runtime conversion that watched `Added<MeshMaterial3d<StandardMaterial>>`. `StandardMaterial` → `MToonMaterial` conversion now happens in the loader's `on_spawn_mesh_and_material` hook and no longer depends on the `StandardMaterial` asset being resolved first, so a material that arrives late no longer leaves a mesh unconverted
- `RequestDetachVrm` accepts the managed `VrmHandle` entity before or after readiness. For a raw `WorldAssetRoot` spawn it accepts either the scene owner or the child root carrying `Vrm`. The target entity stays alive; the rig, its instance registration and derived animation data are removed
- `.vrma` loading is unchanged: `VrmaPlugin`, `VrmaAsset`, `VrmaHandle`, `LoadedVrma`, `PlayVrma` and `StopVrma` all stay, `.vrma` keeps its own loader rather than being claimed by `VrmGltfPlugin`, and a `.vrma` is still spawned as a `VrmaHandle` child of an avatar that becomes `Initialized`. When a managed rig is replaced, the animation graph and playback are restored against the new rig from the pristine source clips; `LoadedVrma` stays a first-load notification

### Features

- Added the scene-ready binding that attaches a load-time VRM scene to the managed `VrmHandle` entity. A global `WorldInstanceReady` observer moves the root-level component inventory — expression data, look-at properties, constraint execution order, rest world transform, the `<Bone>BoneEntity` holders and the synthetic root animation target — onto the entity the app spawned, so `commands.spawn((VrmHandle(..), ..))` is the whole spawn API and the usual `Added<Initialized>` + entity-lookup dance is gone
- VRMA playback was bridged onto load-time scenes. A `.vrma` spawned as a child of an avatar retargets against that avatar's bones and expressions, per instance, and a `.vrma` expression track now drives `VrmExpressionWeights` on the avatar root instead of a per-expression entity tree. A track's X component is the expression weight, clamped into `[0, 1]` by the bind pass
- Added the root-level animatable property surface that retargeted expression tracks need: `VRM_ROOT_TARGET_NAME` / `vrm_root_animation_target`, `ExpressionWeightProperty`, `VrmExpressionIndex`, `VrmExpressionWeights`, `ExpressionOverrides` and `apply_expression_morph_binds`. The VRM root carries a synthetic animation target so a curve can address the avatar as a whole
- Node constraints are now evaluated from the execution order the loader computed while the file loaded, in `VrmSystemSets::Constraints`. The pass walks it in sequence, so a destination that is itself a source sees a fresh value
- Brought the load-time expression shape to parity with v0.10.0: `apply_expression_morph_binds` publishes `EffectiveExpressionWeights` — one entry per declared expression, the final weight the bound morphs receive, after the `isBinary` threshold and the `overrideMouth` / `overrideBlink` / `overrideLookAt` suppressors — so a material-color or texture-transform binding can consume exactly that weight, even when the expression binds no morph

### Bug Fixes

- Preserved the rig generation across invalidation, made duplicate/stale scene-ready events harmless, and scoped managed bone lookups to the avatar's own scene rather than its user-created children
- Restored complete VRMA playback state and transitions on rig replacement, retained node identities across graph updates and clip readmission, released retired clips, and ordered retarget/bake before the first animation evaluation of the new rig
- Fixed a panic on a malformed `VRMC_springBone` extension: a file whose spring-bone JSON does not deserialise now costs the avatar its hair physics instead of aborting the load
- Fixed a panic on an out-of-range `colliderGroups` index — a collider group the spring does not declare is skipped rather than indexed blindly
- A VRM that declares no `hips` humanoid bone no longer aborts the load. Such a file has no root bone at all, so the scene root itself carries the `AnimationPlayer` and the model still loads, renders and keeps whatever bones it does declare
- Fixed a panic when `VRMC_vrm.lookAt.type` is `expression`: gaze control is skipped with a one-time warning instead of hitting a `todo!()` every frame
- Fixed VRM extension parsing to tolerate omitted optional fields, so a file that leaves out something the schema marks optional no longer fails to load: `VRMC_vrm.expressions.preset`, `isBinary`, the three `override*` types, the `meta` flags and lists, `lookAt` and its `RangeMap` values, `VRMC_springBone.springs[*].name`, and `VRMC_materials_mtoon`'s optional textures and texture extensions. A malformed sub-object no longer costs the avatar more than itself: a spring-bone block that does not deserialise is reported and skipped, and a single malformed `materialColorBind` is dropped out of its list
- `VRMC_vrm.expressions.custom` and the `materialColorBinds` / `textureTransformBinds` of an expression are parsed now instead of being dropped, so a custom expression no longer disappears from the loaded model. They are modelled but not consumed yet
- Applied the VRM specification defaults where the file omits a value: spring-joint `dragForce`, `gravityDir`, `gravityPower`, `hitRadius` and `stiffness` (a joint missing one of these used to be discarded entirely, so a strand stopped simulating because the exporter left out `stiffness`), the `sphere` / `capsule` collider fields, the `VRMC_node_constraint` weights (`1.0`), the `RangeMap` values and the expression `override*` types (`none`)
- Fixed `VRMC_materials_hdr_emissiveMultiplier` being read from the wrong property. The multiplier is named `emissiveMultiplier`, not `emissiveStrength`; `emissiveStrength` belongs to `KHR_materials_emissive_strength`, which `bevy_gltf` already applies to the `StandardMaterial`. An `MToon` material's emissive factor is now multiplied by the VRM multiplier exactly once, so an avatar that uses the extension can emit light stronger than white
- Fixed first-person `auto` mesh splitting on meshes whose `JOINTS_0` attribute is not stored as `Uint16x4`. Any element format is accepted now, so a mesh that used to stay visible from a first-person camera (leaving the head in view) is split correctly
- Fixed the first-person `auto` split losing the primitive's topology and morph targets. Because the split replaces the whole mesh, it used to reset the topology to triangles and drop the morph deltas, which silently disabled a morph-target mesh's expressions; the split now carries both over (with the morph target names and pose-following skinned bounds), and a primitive whose morph deltas cannot be rebuilt exactly is conservatively left unsplit
- Fixed the first/third-person classification for models that do not annotate their meshes. The specification says a mesh without an annotation is `auto`, so every mesh that no annotation covers — on itself or on an ancestor node — is now classified `auto`, and `meshAnnotations[*]` accepts the property under either name (`firstPersonFlag`, as the document spells it, or `type`, as the schema spells it) with an absent value falling back to `auto`
- Fixed bone-holder and spring-bone entity references surviving scene instantiation. Components written into the scene asset that hold an `Entity` have to mark the field `#[entities]`, or they keep the loader's scratch-world id; the `<Bone>BoneEntity` holders and the spring-bone colliders now remap, so gaze control, body tracking and spring collision address the right bones of the instantiated avatar
- Fixed `RequestDetachVrm` not removing the components of the load-time component set — spring joints, colliders, node constraints, the first-person head copies, the expression data and the humanoid bone holders now go too, and a stale constraint entry no longer keeps the detached root in the constraint pass's query
- Fixed the first-person `auto` split hiding the neck. The specification says a vertex "contains the weight of Head bone" but names no threshold, and reading that as "any weight above zero" classified a throat vertex whose neck joint dominates as head-side, so the neck moved to the third-person-only layers and vanished from the first-person view on models whose neck skin blends toward the head (a `VRoid` 1.x export's head-weight gradient runs from 0.03 at the throat to 0.87 under the jaw). A vertex is now head-side only when the head subtree holds the majority of its total weight, which puts the split at the anatomical head on every model while a dedicated neck mesh weighted to the neck bone alone stays visible
- Fixed `VrmPlugin` + `VrmGltfPlugin` panicking at startup with "plugin was already added in application". Both add the unique `BonesPlugin` and `MtoonMaterialPlugin`, and adding either twice aborted `App::run`; both registrations now guard against the duplicate, so an app holding both plugins starts in either order
- Fixed the first-person `auto` split handing an empty sub-mesh to the asset server. `classify` decides per vertex, but the split sends a whole triangle to the head as soon as one of its vertices is a head vertex, so a primitive could produce an empty body or head; bevy then extracted that zero-length mesh, skipped its allocation and logged `bevy_render::slab_allocator` use-after-free errors (and its MToon pipeline failed to compile for lack of a UV). A primitive whose split left a half empty is now classified as a whole and left unsplit
- Fixed every `.vrma` playing nothing. The migration deleted the runtime step that stamped `Initialized` on a `.vrma` entity and captured its source rig's rest pose, so `trigger_loaded` never fired, the animation graph was never built and `LoadedVrma` never reached `PlayVrma`. `VrmaInitializePlugin` now waits for the source rig, captures `RestTransform`/`RestGlobalTransform` on its bones (and `RestWorldTransform` on the `.vrma`), and only then marks it initialized

- Fixed a `.vrma` whose sampler holds a huge timestamp aborting the app on allocation: resampling one curve is now capped at 65,536 samples, for bone and expression tracks alike

### Maintenance

- Depends on the `gltf` crate directly instead of reaching the VRM extension handler's types through `bevy_gltf`. `default-features = false` with the `extensions`, `extras`, `names` and `utils` features only, which adds no crate and leaves `Cargo.lock` untouched

## v0.10.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.10.0)

### Features

- Expose `EffectiveExpressionWeight` after the expression pass for material and UV bindings.

## v0.9.3

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.9.3)

### Bug Fixes

- Fixed VRMA retargeting baking the model entity's world placement into every clip: retarget transformations are now computed in the imported model's own frame, so a body that rotates or moves between the VRM and VRMA rest snapshots no longer ends up with a constant per-instance limb offset
- Fixed multi-instance VRMA corruption: the animation graph is now requested once per VRM after all its VRMA children load, every clip is retargeted and baked against its own rig, retarget tables are keyed by the VRMA entity (stable across graph rebuilds), and a rebuilt graph no longer retargets already-baked clips a second time
- Fixed humanoid rest transforms being snapshotted from the parent entity for every child; leaf bones now record their own rest

### Maintenance

- Updated the Windows cursor fallback dependency to `windows` 0.62
- Used fixed-size triangle chunks in first-person mesh classification to satisfy current Clippy

## v0.9.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.9.2)

### Bug Fixes

- Allowed `VRMC_springBone` to omit the optional `colliders`, `colliderGroups`, and `springs` arrays, defaulting them to empty collections as specified

## v0.9.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.9.1)

### Bug Fixes

- Fixed spring-bone chains disappearing when an inverse point transform rounded a very short tail direction to zero

## v0.9.0

### Features

- Added `VRMC_vrm.firstPerson` support: attach `FirstPersonCamera` or `ThirdPersonCamera` to a camera and trigger `RequestEnableFirstPerson` on the VRM entity to assign `RenderLayers` from the model's `meshAnnotations`. Meshes without an annotation are split by head bone weights, `RequestDisableFirstPerson` restores the default visibility, and the layers used for the separation are configurable through the `FirstPersonLayers` resource

## v0.8.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.8,0)

### Breaking Changes

- Migrates bevy_vrm1 from Bevy 0.18 to Bevy 0.19.

## v0.7.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.7.1)

### Bug Fixes

- Fixed MToon conversion skipping meshes whose glTF material name collided with another material: `VrmcMaterialRegistry` now resolves material handles by index instead of by name, so VRMs exported with duplicate material names (e.g. VRoid models with multiple `Body_mtoon` entries) no longer fall back to the default `StandardMaterial` for the collided meshes
- Fixed MToon characters not being lit by directional lights that have `shadows_enabled: false`: the MToon shader was using `shadows_enabled` to gate the entire light contribution, which is inconsistent with Bevy PBR where `shadows_enabled` only controls shadow-map sampling. `apply_directional_lights` now accumulates every directional light's contribution, and `calc_mtoon_lighting_shading` defaults `shadow` to `1.0` when the light has no shadow map
- Fixed full-viewport blackout on WebGPU when rendering MToon materials: `apply_emissive_light` was reading the `EMISSIVE_TEXTURE` bit from `PbrInput.flags` (the standard-material flags field, which `MToonMaterial` does not populate), causing an unbound-texture sample whose NaN/Inf output contaminated the HDR tonemap/bloom path. The shader now reads the bit from the MToon uniform `material.flags`, and `MtoonFlags::from(&MToonMaterial)` now actually sets the `EMISSIVE_TEXTURE` bit based on `emissive_texture.is_some()` so the branch works as intended

## v0.7.0

### Breaking Changes

- Redesigned VRMA retargeting: replaced custom `AnimationCurve` wrappers with pre-baked clips that apply retarget transformations at initialization time

### Bug Fixes

- Fixed multi-VRM animation stop bug: spawning 2+ VRMs from the same `.vrm` file no longer causes animations to stop
- Fixed animation transition interpolation for VRMs with different initial poses (Issue #32)

## v0.6.4

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.4)

### Features

- Added VRM detach request handling via `RequestDetachVrm`; detaching now removes VRM-related components and recursively despawns child entities while keeping the root entity alive

## v0.6.3

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.3)

### Others

- Removed unnecessary cursor position fallback log output

## v0.6.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.2)

### Bug Fixes

- Fixed outline rendering for double-sided meshes: skip outline enqueueing when neither CULL_FRONT nor CULL_BACK is set in MToonMaterialKey, preventing magenta-like artifacts on thin meshes such as skirts and sleeves

## v0.6.1

### Bug Fixes

- Fixed the issue on Windows where the cursor position couldn't be retrived correctly when hit_test is false, by switching to use WinApi.

## v0.6.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.6.0)

### Breaking Changes

- Simplified `LookAt::Cursor { camera: Option<Entity> }` to `LookAt::Cursor`
- `ModifyExpressions` doc comments updated to clarify its role as a partial update API (equivalent to UniVRM's `SetWeight` / three-vrm's `setValue`)
- `VrmExpressionRegistry` value type changed from `Vec<ExpressionNode>` to `ExpressionMetadata`

### Bug Fixes

- Fixed hips retargeting phantom X/Z shift by using local rest positions for delta computation instead of global positions; global positions are now only used for Y-based height scaling
- Fixed LookAt Cursor mode to use world-space ray casting instead of screen-space normalized coordinates, so gaze calculation now accounts for the avatar's world position
- Fixed `MorphTargetBind.weight` being parsed but ignored — now correctly applied as `expression_weight × bind.weight`
- Fixed expression weights using direct assignment instead of additive accumulation per VRM 1.0 spec
- Implemented `overrideBlink`/`overrideLookAt`/`overrideMouth` expression override system (was parsed but unused)
- Implemented `isBinary` threshold behavior (weight > 0.5 → 1.0, otherwise 0.0)

### Features

- Added direct expression control API (`SetExpressions`, `ClearExpressions`) for controlling VRM facial expression weights from user code without VRMA animation files
- Added `ExpressionEntityMap` component for O(1) expression entity lookups and introspection of available expressions
- Added `expressions` example demonstrating keyboard-driven expression control
- The `BodyTracking` component has been added. By inserting it with LookAt, you can control not only the eyes but also the upper body.

### Improvements

- Made Spring structs (`SpringRoot`, `SpringJoints`, `SpringJointProps`) public
- Moved `bind_expressions` system from `VrmaRetargetExpressionsPlugin` to `VrmExpressionPlugin` so expressions work with or without VRMA

## v0.5.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.5.1)

### Bug Fixes

- Fixed MToon outline rendering pipeline after Bevy 0.17+ migration
  - Outline pass now correctly uses MToonMaterial vertex/fragment shaders instead of default PBR
  - Added MToon material bind group layout at index 3 with MATERIAL_BIND_GROUP=3

## v0.5.0

- Migrated Bevy dependency from v0.17 to v0.18.

## v0.4.0

### Breaking Changes

- Migrated Bevy dependency from v0.16 to v0.17.
- Outline rendering now uses a strict depth compare to avoid full-surface outline fill on thin meshes.

### Bug Fixes

- Fixed system execution order for VRM constraints and expressions to comply with VRM specification
  - Added manual transform propagation after constraints and expressions
  - Ensures `GlobalTransform` updates propagate correctly between systems
  - Fixes rendering and physics issues caused by stale transform data

## v0.3.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.3.0)

### Features

- Added support for Node Constraints (VRMC_node_constraint-1.0)
  - Rotation Constraint: Transfers entire local rotation from source to destination nodes
  - Roll Constraint: Transfers rotation around a specific axis (X, Y, or Z)
  - Aim Constraint: Rotates a node to face a target node
  - All constraint types support weight-based interpolation using spherical linear interpolation (slerp)

### Breaking Changes

- `AnimationTransitions` are now used internally;This enables smooth animation transitions.
  - Changed fields of `PlayVrma`
- added `log` feature flag to enable logging.
  - Error logs are now not output by default.
- The update timing for SpringBone and LookAt has been changed to `PostUpdate`.
- Rust edition has been changed to 2024.
- Renamed some of the methods defined on SystemParams in this crate.
  - Doesn't affect most users

### Bug Fixes

- Fixed collision detection for the SpringBone sphere collider.
- Fixed logic to determine redraw
- Fixed look at bone rotation
- Fixed `ColliderGroup::name` types from `String` to `Option<String>` to match the spec.

## v0.2.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.2)

### Bug Fixes

- Fixed SpringBone colliders.
- Changed the spring bone calculation to use the center space if a center node is set.
- Fixed an issue that caused a crash during MToon shader processing.
  - This occurred in Bevy v0.16.1 and later versions.

## v0.2.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.1)
I was going to add this in v0.2.0 but forgot.

### Improvements

- Added `VrmSystemSets` to define the system order of `Retarget`, `LookAt`, and `SpringBone`.
- Export several VRM(A) components that were not being exported correctly via `prelude` module.

## v0.2.0

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.2.0)

### Breaking Changes

- `MToonOutline` is no longer a component; it has become part of the `MToonMaterial` fields.
- `OutlineWidthMode` has been added as part of the field of `MToonOutline`.
  - Currently only supports `OutlineWidthMode::WorldCoordinates` and `OutlineWidthMode::None`, and if
    `screenCoordinates` is passed, the outline will not be rendered.
- Fixed the rendering order of the outline to match the spec.
  - refer
    to [here](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md#rendering)
    for more details.
- Removed `reflect` feature flag, and `serde` has been added instead.
  - `Reflect` is now applied to most structs by default.

### Bug Fixes

- Fixed outline rendering

## v0.1.2

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.1.2)

### Bug Fixes

- Fixed so that retargeting bone works correctly between models with different initial poses.
- Fixed a bug that only one animation could be played.

## v0.1.1

[Release Notes](https://github.com/not-elm/bevy_vrm1/releases/tag/v0.1.1)

### Bug Fixes

- Fixed `VrmcMaterialsExtensitions::outline_width_factor` type from `f32` to `Option<f32>` to match the spec.
- Fixed shadow casting for directional lights.

### Features

- Supported multiple directional lights

## v0.1.0

First Release!
