# Rust Coding Rules

Rust code in this crate (`src/`) follows the rules below. Edition 2024. These rules complement the conventions documented in `CLAUDE.md`.

## Existing code

The rules bind code written or changed from now on. Code that predates a
rule keeps its current form until it is touched; a change to a function
brings that function in line with every rule it falls under. Do not
rewrite untouched code only to satisfy a rule.

## Module layout — no `mod.rs`

Required:

| Pattern                 | Example                 | Why                                                                            |
| ----------------------- | ----------------------- | ------------------------------------------------------------------------------ |
| Rust 2018+ module files | `foo.rs` + `foo/bar.rs` | One file declares the module; no ambiguity about which file is the module root |

Forbidden:

| Pattern                 | Example      | Why                                                                          |
| ----------------------- | ------------ | ---------------------------------------------------------------------------- |
| `mod.rs` as module root | `foo/mod.rs` | Hard to navigate (many files all named `mod.rs`); editor tabs look identical |

## Comments

Non-doc line comments are restricted. The only permitted forms:

| Form                | Use                                                                                  |
| ------------------- | ------------------------------------------------------------------------------------ |
| `// TODO: <text>`   | Work to address later                                                                |
| `// NOTE: <text>`   | A **critical caveat** — only when overlooking it causes real harm; see the bar below |
| `// SAFETY: <text>` | Required justification for any `unsafe { ... }` block (rustc / clippy idiom)         |

Forbidden:

| Pattern                                                  | Example                        | Why                                                                 |
| -------------------------------------------------------- | ------------------------------ | ------------------------------------------------------------------- |
| Plain narrative comments                                 | `// 3. Smooth yaw/pitch.`      | What the code does belongs in identifiers                           |
| Block comments                                           | `/* ... */`                    | Same                                                                |
| Commented-out code                                       | `// let x = old_impl();`       | History lives in git                                                |
| `// NOTE:` for merely non-obvious or "good to know" info | `// NOTE: this is the handler` | NOTE is for critical caveats only — rename the identifier or delete |

A `// NOTE:` is reserved for a **critical caveat**: something that, if a
reader overlooks it, leads to a bug, a crash, data loss, a security
issue, or a violated invariant. "Non-obvious" or "good to know" is not
enough — the test is concrete harm on the line that misses it.
Qualifying examples: a race condition, a workaround for a specific
upstream bug, an ordering requirement the surrounding code silently
relies on (a system that must read `GlobalTransform` after a manual
propagation), an invariant a later mutation must preserve. If
overlooking the comment causes no real failure, do not write it —
rename an identifier so the code carries the meaning, or delete it.

Note: `///` and `//!` are **doc comments**, not "line comments" for this rule — see "Doc comments" and "Doc comment content — only what the reader needs" below.

## Comment references — cite specifications, nothing under `docs/`

Comments and doc comments may cite the VRM specifications and other
normative documents by URL — `vrm-c/vrm-specification`, the VRM update
order on `vrm.dev`, the glTF 2.0 specification — including a section of
one (`lookAt.md, "rangeMap"`). **No document under `docs/` may be cited
from code**, doc comments included — not by path, and not by section
number or decision ID (`spec §7`, `R3 of the design`).

`docs/plans/` and `docs/discussions/` are git-ignored working notes, so a
pointer to one is dead for every other reader the moment it is written,
and nothing fails to compile when it is. The tracked files under `docs/`
serve the README and are not kept in step with the code.

| Pattern                                       | Example                                         | Instead                                                          |
| --------------------------------------------- | ----------------------------------------------- | ---------------------------------------------------------------- |
| Citing a working note                         | `// TODO: … (docs/plans/look-at-plan.md task 4)` | State the blocker in the comment itself and drop the path        |
| Citing a design doc by section or decision ID | `/// … (spec §7)` / `// … (H2)`                 | Drop the pointer; state the contract (doc) or the caveat (`// NOTE:`) itself |
| Citing a VRM specification                    | `/// lookAt.md — "rangeMap"` with its URL       | **Allowed** — the specifications are permanent                   |

The rule is one-directional: Markdown under `docs/`, PR descriptions and
commit messages may link wherever they like. It binds only what ships
inside a `.rs` file.

When a working note holds reasoning a future reader will want, copy the
**conclusion** into the comment rather than linking the note. In a doc
comment the conclusion is copied only as the contract it implies; the
rationale behind it is not copied (see "Doc comment content — only what
the reader needs").

## Doc comments

Required:

| Place                                                                                      | Style                                               |
| ------------------------------------------------------------------------------------------ | --------------------------------------------------- |
| Every externally-public item (`pub` only — not `pub(crate)`, `pub(super)`, `pub(in path)`) | `///` — one-line summary, blank line, optional body |
| Each file-level module — `lib.rs` and every `foo.rs` that declares a module                | `//!` — module-level purpose in 1–3 lines           |

Equivalent attribute forms (`#[doc = "..."]`, `#[doc = include_str!("README.md")]`) count as doc comments and satisfy this rule.

Not required (but recommended):

- `pub(crate)` / `pub(super)` / `pub(in path)` items
- Inline modules (`mod inner { ... }` inside another file)
- `#[cfg(test)] mod tests { ... }` blocks and their contents — EXCEPT
  `#[test]` functions themselves, which are covered by "Test doc
  comments" below

Style guide:

- The first line is a noun phrase or third-person singular verb phrase (e.g., `/// Returns the world-space position being looked at.`). Stay descriptive, not imperative.
- Code examples use triple backticks; hidden setup lines may be prefixed with `# `.
- Invariants live under a `# Invariants` section.

Forbidden:

| Pattern                                     | Why                                       |
| ------------------------------------------- | ----------------------------------------- |
| Externally `pub` item with no doc           | Public API owes the reader an explanation |
| Placeholder doc like `/// TODO: write this` | Don't ship empty docs                     |

## Doc comment content — only what the reader needs

A doc comment (`///`, `//!`, or the equivalent `#[doc = "…"]`) states the
contract a reader relies on to use the item, and nothing else. Content
outside the allowed list below is deleted — not moved to a `//` comment, a
`# Notes` section, or another item's doc — except as the `NOTE:` boundary
case below allows. This applies to every doc comment under `src/`,
whatever the item's visibility.

Allowed:

| Content | Example |
| ------- | ------- |
| The one-line summary | `/// Returns the world-space position being looked at.` |
| Observable behavior, and what the return value means — including `None`, `false`, and sentinel values | `` Returns the ray, or `None` when no window contains the cursor. `` |
| Meaning of an input that its type does not state: unit, coordinate space, base, valid range | `` `depth` is in metres, measured from the face toward the camera. `` |
| Side effects and state changes the caller can observe | `The previous gaze target is kept.` |
| Preconditions and caller obligations, including an ordering constraint whose violation breaks behavior | `Insert it on the entity that carries LookAt.` |
| Edge cases and boundary values | `A non-finite or non-positive depth falls back to 1.0.` |
| `# Panics`, `# Errors`, `# Safety` | |
| `# Invariants` — the guarantee itself | `The position is never read before the first frame resolves it.` |
| Limitations and unsupported cases the caller can observe, stated in the present tense | `Only VRM 1.0 files are supported.` |
| Spec mapping: `# References`, and citations of the VRM / glTF specifications | `lookAt.md — "the yaw is measured in the look-at space"` |
| For a `#[test]` fn: the asserted contract and the `Case:` scenario (see "Test doc comments") | |
| `# Examples` | |
| `TODO: <text>` | `TODO: support expression-type lookAt.` |

Forbidden (representative — the allowed list above is authoritative):

| Category | Example |
| -------- | ------- |
| Implementation steps — how the body does its work | `The point is found by intersecting the ray with a plane through the head.` |
| Design rationale and rejected alternatives, including why an item is named or placed as it is | `A component is used instead of a resource because …` |
| External history and comparisons with other implementations: other VRM runtimes' behavior, issues, commits, dependency source lines | `UniVRM clamps the same way` / `three-vrm issue #123` |
| Hypothetical breakage, and deferral to another item's doc | `A rewrite that reads GlobalTransform first would lag a frame.` / `` [`GazeTarget`] records why `` |
| Time-bound wording, and pointers to design documents the reader cannot follow | `VRM 0.x support arrives later.` / `for now` / `until the next release` / `(spec §7)` |
| Echo of the code: the name, signature, or item kind restated; ECS registration, schedule, or `run_if` recited; another item's doc copied | `Bevy Component wrapping …` / `Runs in PostUpdate before …` |
| Callers and tests, in a non-test item's doc: who calls the item, how testable it is, what the tests cover | `Used by BodyTracking.` / `Kept pure so unit tests can cover every branch.` |
| Absence inventory: what the item does not hold or know, when no caller would expect it to | `The gaze target knows nothing about cameras.` |
| A `NOTE:` tag inside a doc comment | `/// NOTE: …` |

Boundary cases:

- An intra-doc link that names a type or item the contract uses is allowed;
  a link that hands the explanation to another item is not.
- "Preconditions and caller obligations" covers what a caller must do;
  "Callers and tests" covers who the callers are. Only the first is allowed.
- An `# Invariants` entry states the guarantee; what would break without it
  is not written.
- A contrast that states what the item does against a behavior a caller
  could expect ("kept rather than reset", "ignored rather than clamped") is
  observable behavior and is allowed; an alternative the designers
  considered and rejected is not.
- Citing what a specification says ("lookAt.md defines rangeMap as …") is
  spec mapping; describing what another implementation does ("UniVRM
  clamps …") is a comparison.
- Schedule and ordering information is allowed on an item an outside system
  must order against (a `SystemSet` such as `VrmSystemSets`, for instance);
  reciting where the item itself is registered or scheduled is not.
- Each item states its own contract, even when it matches a delegate's;
  "another item's doc copied" means text that describes a different item.
- A sentence that mixes allowed and forbidden content is split: the allowed
  part stays, and a consequence of misuse is restated as the precondition or
  `# Panics` entry it implies.
- Future work is written only as a `TODO:`, whose text the time-bound ban
  does not cover. Untagged future-design prose becomes a `TODO:` when it
  names pending work and is deleted otherwise.
- A heading outside the allowed list (`# Notes`, `# Layout`, …) grants no
  allowance: each sentence under it is judged by the rows above.
- A module doc (`//!`) states the module's purpose in 1–3 lines; it does not
  list the module's items, files, or callers.
- A field or variant doc does not repeat its parent's doc or a sibling's.
- A `NOTE:` found in a doc comment is rewritten as allowed doc content when
  it states a contract, becomes a body `// NOTE:` when it meets the bar in
  "Comments", and is deleted otherwise.
- Test doc comments keep the two-part structure in "Test doc comments"; the
  forbidden list applies to them as well.

## Comment prose — write complete English sentences

English prose in comment and doc bodies — doc-comment paragraphs
(including `Case:` paragraphs) and the text of `// TODO:` / `// NOTE:` /
`// SAFETY:` comments — is written as grammatically complete, natural
sentences. Telegraphic fragments that drop subjects, verbs, or articles
read as shorthand notes, not documentation, and force the reader to
reconstruct the elided grammar.

Required:

| Instead of (fragment)                                         | Use (complete sentence)                                                                                       |
| ------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `Case: cursor crosses the face — eyes follow, body lags.`     | `Case: the user moves the cursor across the avatar's face, so the eyes follow it while the body turns slowly.` |
| `Case: the detach teardown.`                                  | `Case: the app detaches a VRM from its scene root and keeps the model.`                                        |

Forbidden:

| Pattern                                                    | Example                         | Why                                          |
| ---------------------------------------------------------- | ------------------------------- | -------------------------------------------- |
| Noun-phrase-only sentence in a body paragraph              | `Case: the raw-spawn path.`     | A sentence needs a verb; say what happens    |
| Clause chains with dropped subjects, objects, or articles | `cursor moves, eyes follow`     | Shorthand the reader must mentally re-expand |

Not fragments for this rule (still fine):

- The **first line** of a doc comment — the style guide above
  deliberately makes it a noun phrase or third-person singular verb
  phrase (`/// Returns the world-space position being looked at.`).
- Standard parallel ellipsis sharing an auxiliary or subject
  (`are clamped to the range, not wrapped`).
- Em-dashes, semicolons, and colons joining clauses that are each
  complete on their own.
- Table cells, section headings, and list labels inside doc bodies.

Complete does not mean long — keep comments concise:

- Say it once: one to three short sentences per paragraph; a doc body
  that can be one sentence stays one sentence.
- Cut filler and repetition: do not restate the first line in the
  body. What a doc comment may contain at all is set by "Doc comment
  content — only what the reader needs".
- When trimming, drop whole sentences rather than degrading the
  survivors into fragments — the complete-sentences rule above still
  applies to what remains.

## Test doc comments

Every `#[test]` function carries a `///` doc comment that states the
asserted contract AND the concrete case the test envisions:

| Place                   | Style                                                                                                                       |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Every `#[test]` function | `///` — first line: what the test asserts; blank line; a `Case:` paragraph naming the envisioned scenario — and nothing else |

- The `Case:` paragraph describes the real-world scenario the test
  envisions — what the app or the user does and the state the avatar is
  in when it happens. It is NOT a restatement of the assertions; a
  reader should learn which situation the contract serves, not what the
  `assert!` lines already say.
- A test doc holds exactly those two parts — no policy paragraph in
  between. When a test pins a decided policy (e.g. "an invalid depth
  falls back to 1.0"), fold the decision — including the behavior a
  reader might expect instead, when it fits — into the first line
  ("Asserts that an invalid depth falls back to 1.0 rather than being
  clamped."); the rationale behind the decision is not written in any
  doc comment.
- Keep the `Case:` paragraph to the scenario and stop — 1–3 sentences.
  The scenario is the paragraph's ONLY content; in particular:
  - Do not restate what the test itself pins — the first line already
    says it.
  - Do not state policies, design decisions, or their rationale in the
    `Case:` paragraph — fold the decision into the first line and
    leave the rationale out.
  - Do not speculate about how a hypothetical broken implementation
    would misbehave ("a resolver that drops the target would leave the
    eyes still") — that is the test's justification, not the case.
  - Do not add layering footnotes about contracts pinned by another
    layer's tests (sign conventions, clamping, etc.) — leave them out
    entirely rather than pointing at them.

```rust
/// Asserts that a non-finite or non-positive cursor depth falls back to
/// 1.0 rather than being clamped.
///
/// Case: an app sets `CursorGazeDepth(0.0)`, or loads a NaN from a broken
/// settings file.
#[test]
fn invalid_cursor_depths_fall_back_to_one() { ... }
```

Not required:

- Test helpers, fixtures, and mocks inside `#[cfg(test)]` — keep their
  names descriptive instead (a doc comment is still welcome when it
  states a non-obvious contract of the helper, e.g. which bones a test
  rig spawns and where).

## Imports

Required:

- Every `use` is declared at the top of the file (immediately after the `//!` if present).
- All `use` statements form a single contiguous block — no blank lines separating `std`, external crates, and crate-local imports.

Exception:

- Inside `#[cfg(test)] mod tests { ... }` blocks, locally-scoped `use` statements are allowed (e.g., `use super::*;`, test-only fixtures). Test code is the only place where locality outweighs the "all imports at the top" rule.

Forbidden:

| Pattern                                             | Example                                  | Why                              |
| --------------------------------------------------- | ---------------------------------------- | -------------------------------- |
| `use` inside non-test functions or blocks           | `fn f() { use std::io; ... }`            | Spreads scope across the file    |
| Blank lines between `std` / external / crate `use`s | `use std::...;\n\nuse bevy::...;`        | We do not group                  |
| Glob import of anything but a prelude               | `use crate::vrm::look_at::*;`            | Hides which symbols are in scope |

Preludes are the exception to the glob ban: `use bevy::prelude::*;`,
`use crate::prelude::*;`, and a module's own `prelude` (such as
`crate::system_param::prelude::*`) may be glob-imported. A module that
_defines_ a prelude may also use `pub use foo::*;` inside it.

Required — import, don't inline:

- Prefer `use` imports over inline fully-qualified paths. If a type is used in a function signature or system body, it belongs in the `use` block at the top of the file, not written out inline as `some_crate::some_module::Type` at the call site.

| Pattern                                   | Example                                         | Fix                                                                       |
| ----------------------------------------- | ----------------------------------------------- | ------------------------------------------------------------------------- |
| Inline path in signature / body           | `fn f(x: foo::bar::Baz)`                        | Add `use foo::bar::Baz;` and write `fn f(x: Baz)`                         |
| Inline path in `run_if` or type parameter | `.add_message::<bevy::window::WindowResized>()` | `use bevy::window::WindowResized;` then `.add_message::<WindowResized>()` |
| Inline path in `.after()` / `.before()` / `.in_set()` system-ordering | `.in_set(crate::system_set::VrmSystemSets::GazeControl)` | `use crate::system_set::VrmSystemSets;` then `.in_set(VrmSystemSets::GazeControl)` |

## Naming — Query parameters

Bevy `Query` system parameters must not use a `_q` suffix. Use a descriptive noun instead:

- Singular (`window`, `camera`) when the query is expected to return one result and the system calls `.single()` / `.single_mut()`.
- Plural (`windows`, `vrms`) when the query is iterated or used with `.get()` over an arbitrary entity.

Forbidden:

| Pattern                              | Example                       | Fix                         |
| ------------------------------------ | ----------------------------- | --------------------------- |
| `_q` suffix on any `Query` parameter | `window_q: Query<&Window, …>` | `window: Query<&Window, …>` |

## Constructors — type-building functions are associated functions

A function whose job is to build a value of a struct or enum defined in
this crate belongs on that type as an associated function
(`impl T { fn … -> Self }`), called as `T::build(…)`, not as a free
`fn build_t(…) -> T`. Co-locating the constructor with its type keeps
every way to produce a `T` reachable from `T` itself, and reads as
`GazeTarget::from_cursor(…)` at the call site rather than a bare
`cursor_gaze_target(…)` whose return type you have to infer from the
name.

This covers factory / parser / classifier / decoder helpers — any
function that assembles a fresh value of a local type from its inputs —
whether it returns `T`, `Option<T>`, or `Result<T, _>`.

Required:

| Instead of (free function)                                    | Use (associated function)                                       |
| ------------------------------------------------------------- | --------------------------------------------------------------- |
| `fn cursor_ray(windows, cameras) -> Option<CursorRay>`        | `impl CursorRay { fn find(windows, cameras) -> Option<Self> }`  |
| `fn cursor_gaze_target(cursor, origin, depth) -> GazeTarget`  | `impl GazeTarget { fn from_cursor(cursor, origin, depth) -> Self }` |

- Return `-> Self` (not the spelled-out type name) and build variants
  with `Self::Variant` inside the `impl`.
- Name the method for what it does, dropping the type name the `T::`
  prefix already carries: `cursor_gaze_target` → `GazeTarget::from_cursor`.

Required — a constructor that takes no argument is `Default`:

| Instead of (inherent no-arg constructor) | Use                                                           |
| ---------------------------------------- | ------------------------------------------------------------- |
| `impl BoneState { fn new() -> Self }`    | `impl Default for BoneState`, called as `BoneState::default()` |

- Write `#[derive(Default)]` when every field's default is the value the
  empty `T` needs.
- On a generic type, write the impl by hand (`impl<P> Default for
  Tokens<P>`): the derive adds a `P: Default` bound the type parameter
  does not need.

Forbidden:

| Pattern                                                | Why                                                        |
| ------------------------------------------------------ | ---------------------------------------------------------- |
| Free `fn` returning a locally-defined `T` it constructs | The constructor is discoverable only by grep, not from `T` |
| Inherent `fn new() -> Self` taking no argument          | `Default` is the standard spelling of an argument-free constructor |

Exceptions:

- **Foreign return type.** You cannot add an inherent method to a type
  defined in another crate (`GlobalTransform`, `Quat`, `Vec3`). Prefer a
  `From` / `TryFrom` / `FromStr` trait `impl` (those satisfy this rule —
  they are associated functions); fall back to a free `fn` only when no
  conversion trait fits.
- **Standard conversion traits.** `From` / `TryFrom` / `FromStr` /
  `Default` impls already are the idiomatic constructors — a free `fn`
  that should be one of these is converted to the trait, not to an
  inherent method.
- **Bevy systems / observers.** A system or observer is not a
  constructor even when it `commands.trigger`s an event: its return is
  `()` and the ECS requires a free `fn`. This rule targets functions
  whose returned value **is** the constructed `T`.
- **Lookups / getters.** A function that returns an already-existing
  value (e.g. an `Option<Entity>` from a query, an accessor returning a
  borrow) is not building a new `T` and is out of scope.

## Constants — declare each `const` in the narrowest scope that holds its uses

A `const` is declared in the narrowest scope that covers every place
that names it. Module scope is the fallback, not the default: a constant
at the top of a file reads as shared by the whole module, so one that
serves a single function, or that describes one type, misleads the
reader about how far it reaches.

Pick the scope narrowest first:

1. Exactly one function names it — a `const` local to that function.
2. It states a property of one type — an associated const on that type.
3. Otherwise — a module-level `const` in the narrowest module that holds
   its uses.

Required — a constant one function uses is local to it:

- A `const` that exactly one function names is declared inside that
  function's body. This includes a method: a constant only one method
  uses is local to it, not an associated const.
- Declare it at the top of the body, before the first statement, with
  its `///` doc.

```rust
/// Whether `name` is the name of one of the eye bones.
fn is_eye_bone(name: &str) -> bool {
    /// The `VRMC_vrm` humanoid names of the bones the bone-type look-at rotates.
    const EYE_BONES: [&str; 2] = ["leftEye", "rightEye"];

    EYE_BONES.contains(&name)
}
```

Required — a constant that belongs to a type is an associated const:

- A `const` that states a property of one type's values — a limit, a
  default, a capacity — and is named by more than one function is an
  associated const in that type's `impl`, referenced as
  `CursorGazeDepth::DEFAULT_DEPTH` (`Self::DEFAULT_DEPTH` inside the
  `impl`).
- Name it for what it is, dropping the type name the `T::` prefix
  already carries: `DEFAULT_CURSOR_DEPTH` →
  `CursorGazeDepth::DEFAULT_DEPTH`.
- An associated const used only inside the type's own module stays
  private; a wider one follows "Visibility — don't restate a type's own
  ceiling on its members".
- Associated consts are declared at the top of the `impl` block, above
  every method, whatever their visibility: a private const still comes
  before a `pub fn`. Among themselves they keep the descending
  visibility order of "Item ordering — private items last".

```rust
impl CursorGazeDepth {
    /// The depth used when none is set or the set one is invalid (metres).
    const DEFAULT_DEPTH: f32 = 1.0;
}
```

| Instead of                                                                   | Use                                                         |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------- |
| Module-level `const EYE_BONES: …` that only `is_eye_bone` names              | `const EYE_BONES: …` at the top of `is_eye_bone`'s body     |
| Module-level `const DEFAULT_CURSOR_DEPTH: f32 = 1.0;`                        | `impl CursorGazeDepth { const DEFAULT_DEPTH: f32 = 1.0; }`  |

Forbidden:

| Pattern                                                              | Why                                                               |
| -------------------------------------------------------------------- | ----------------------------------------------------------------- |
| A module-level `const` that exactly one function names               | Claims a module-wide reach the constant does not have             |
| A module-level `const` that states a property of one type            | The constant is discoverable only by grep, not from the type      |
| An associated const declared below a method in the same `impl`       | The values the methods read are not visible before the methods    |

Counting uses:

- Uses are counted across the crate, and tests and intra-doc links count
  as uses. A constant that a test asserts against cannot be local, so it
  moves to the associated or module level.
- A constant whose type or value names a generic parameter of the
  enclosing function or `impl` cannot be local (rustc E0401), so it is an
  associated const.

Not covered by this rule:

- Test-only code: `#[cfg(test)] mod tests { ... }` contents, and any
  module gated behind `#[cfg(test)]`.

## Visibility — don't restate a type's own ceiling on its members

In an `impl` block on a type that is **not** itself externally `pub`,
associated items are declared `pub`, never `pub(crate)` / `pub(super)` /
`pub(in path)`. The type's own visibility already caps how far its
methods can be reached, so repeating a narrower spelling on each one adds
a qualifier that changes nothing.

`pub(crate) struct CursorRay` cannot be named outside the crate, so
`pub fn find(…)` on it is reachable in exactly the places
`pub(crate) fn find` would be. `pub` is then the honest spelling — "as
reachable as this type allows" — and it keeps the diff quiet when the
type's own visibility later changes: one line moves, not twenty.

Required:

| Instead of                                                          | Use                                  |
| ------------------------------------------------------------------- | ------------------------------------ |
| `pub(crate) fn find(…) -> Option<Self>` on a `pub(crate)` type      | `pub fn find(…) -> Option<Self>`     |
| `pub(super) fn build(…) -> Self` on a `pub(super)` type             | `pub fn build(…) -> Self`            |
| `pub(crate) const LIMIT: usize` in such an `impl`                   | `pub const LIMIT: usize`             |

Private stays private. This rule raises `pub(crate)`-and-wider to `pub`;
it never widens an item that has no visibility modifier. A helper used
only inside its own module keeps none.

Forbidden:

| Pattern                                                    | Why                                                       |
| ---------------------------------------------------------- | --------------------------------------------------------- |
| `pub(crate) fn` in an `impl` on a `pub(crate)` type        | Redundant with the type's own ceiling                     |
| `pub(super) fn` in an `impl` on a `pub(crate)` type        | Narrows below the ceiling for no stated reason            |

Consequences to keep straight:

- **Item ordering.** "Item ordering — private items last" still applies;
  the middle group simply disappears — `pub` items first, private last.
- **Doc comments.** "Every externally-public item (`pub` only …)" keys on
  what a downstream crate can reach, not on the spelling. A `pub fn` on a
  `pub(crate)` type is not externally public, so a `///` on it stays
  recommended rather than required — the standing it had as `pub(crate)`.
- Neither `unreachable_pub` nor `missing_docs` objects: `unreachable_pub`
  is not enabled in this crate — it flags precisely the pattern this rule
  requires, so the two cannot both hold — and `missing_docs` fires only on
  items a downstream crate can reach.

Not covered by this rule:

- An `impl` on a type that **is** externally `pub` (`GazeTarget`,
  `LookAt`). There, each member's visibility is a real API decision and
  `pub(crate)` on one of them says something true.
- Struct fields, trait definitions and their items, and module-level
  declarations (`pub(crate) mod`, `pub(crate) use`). Those are governed by
  where the name needs to be reachable, not by an enclosing type's
  ceiling.
- Trait `impl` blocks, whose item visibility the trait dictates.
- Test-only code: `#[cfg(test)] mod tests { ... }` contents, and any
  module gated behind `#[cfg(test)]`. Such code is unreachable from a
  release build, so no spelling of its members' visibility is
  load-bearing.

## Item ordering — private items last

Within an `impl` block (and at module / file scope), declare items in
descending visibility order: `pub`, then `pub(crate)` / `pub(super)` /
`pub(in path)`, then private (no-modifier) items **last**. Private
helper functions live at the bottom of the block, below every item that
exposes API.

Rationale: a reader scanning an `impl` block or module sees the surface
it can call first; the implementation details that support that surface
come after.

Required:

- Private (no-visibility-modifier) `fn`s are declared after every `pub`
  and `pub(crate)`-or-narrower-but-still-exported item in the same
  `impl` block or module.
- Within the private group, keep related helpers together; order them
  for readability (roughly call order).
- Associated consts are the exception: every associated const of an
  `impl` block, private ones included, is declared above its methods
  (see "Constants — declare each `const` in the narrowest scope that
  holds its uses"). The visibility order then applies within the consts
  and within the methods separately.

Not constrained:

- Test-only code: `#[cfg(test)] mod tests { ... }` contents, and any
  module gated behind `#[cfg(test)]` — test code is exempt.
- Trait `impl` blocks whose method order is dictated by the trait.
- Struct field order — governed by layout / grouping concerns, not this
  rule.

## Parameter ordering — mutable parameters first

Within a function or method signature, declare **mutable** parameters
before **immutable** ones. A parameter is mutable when its binding is
`mut` or it carries mutable access — e.g. `mut s: String`,
`buf: &mut Vec<u8>`, `mut commands: Commands`,
`mut transforms: Query<&mut Transform>`, `mut cursor: ResMut<CursorRayThisFrame>`. A
parameter is immutable when it is a non-`mut` by-value or shared-access
binding — e.g. `name: &str`, `windows: Query<&Window>`,
`cursor: Res<CursorRayThisFrame>`, `time: Res<Time>`.

Rationale: grouping the parameters a function writes through ahead of
the ones it only reads makes the call's effect surface visible at the
signature — the same "surface first" reasoning behind item ordering.

Required:

| Pattern                                      | Example                                                                       |
| -------------------------------------------- | ----------------------------------------------------------------------------- |
| Mutable params grouped first, then immutable | `fn track(mut transforms: Query<&mut Transform>, time: Res<Time>)`            |

Forbidden:

| Pattern                                   | Example                                                               | Why                            |
| ----------------------------------------- | --------------------------------------------------------------------- | ------------------------------ |
| An immutable param ahead of a mutable one | `fn track(time: Res<Time>, mut transforms: Query<&mut Transform>)`    | Mutable params must come first |

Exceptions — these override the style rule:

- A **fixed structural leading position** is exempt and never reordered: a
  method's `self` receiver (`&self` / `&mut self`), and a Bevy observer
  system's `On<E>` trigger (the system _input_, which `bevy_ecs` requires to
  be first). The mutable-first ordering governs only the parameters that
  **follow** such a slot — e.g.
  `fn on_insert(trigger: On<Insert<T>>, mut commands: Commands, q: Query<&U>)` is
  compliant: `trigger` is fixed first, and the params after it are mutable-first.
- A **semantic ordering requirement** wins. When parameter order carries
  meaning — e.g. Bevy `SystemParam`s with separate deferred command
  queues that must apply in a specific order — order for correctness and
  record why in a `// NOTE:`.
- Trait-method `impl`s whose signature is dictated by the trait.
- Test-only code is exempt: `#[cfg(test)] mod tests { ... }` contents,
  and any module gated behind `#[cfg(test)]`.

## System optimization — gate with `run_if`, not in-body change checks

A Bevy system that begins by checking a resource's change state and
returning early — `if !res.is_changed() { return; }` (or
`if !res.is_added() { return; }`) — must instead be gated at
registration with a run condition. The early-return form still pays the
cost of scheduling the system, acquiring its `SystemParam` data access,
and running its body up to the guard every frame; a `run_if` condition
skips the system entirely when the condition is false, and lets the
scheduler reason about the dependency.

Required:

| Instead of (in-body)                                           | Use (at registration)                          |
| -------------------------------------------------------------- | ---------------------------------------------- |
| `fn sys(res: Res<T>) { if !res.is_changed() { return; } ... }` | `sys.run_if(resource_exists_and_changed::<T>)` |
| `fn sys(res: Res<T>) { if !res.is_added() { return; } ... }`   | `sys.run_if(resource_added::<T>)`              |

- Prefer `resource_exists_and_changed::<T>` over bare
  `resource_changed::<T>` unless the resource is guaranteed to exist for
  the system's whole lifetime; the `_exists_` variant will not panic if
  the resource is absent.
- After moving the guard into a `run_if`, delete the in-body early
  return — leaving both is redundant and misleads the reader about when
  the system runs.
- Keep any test that registers the same system in sync: add the matching
  `run_if` so the test exercises the real scheduling behavior.

Not covered by this rule (leave as-is):

- Per-entity / per-component change detection inside a query loop
  (`query.iter().any(|c| c.is_changed())`) — that is not a whole-system
  gate and has no `run_if` equivalent.
- Bodies that branch on change state to do _different_ work (not an
  all-or-nothing early return).

## Change detection — let mutation drive it, don't force it manually

Bevy emits a change notification automatically when you write through a
`ResMut` / `Mut` (any `DerefMut`); readers gate on that via `run_if`
(see the section above) or `Changed<T>` / `Added<T>` queries. A design
that follows ordinary ECS data flow therefore never needs to _manually_
announce that something changed. Manual notification breaks the contract
that "changed" means "the value actually changed", so every downstream
`run_if`/`Changed` consumer can no longer trust it.

Forbidden:

| Pattern                                                                        | Why                                                                                                        |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `res.set_changed()` / `query_item.set_changed()`                               | Forces a notification the mutation itself should have produced                                             |
| `*res.bypass_change_detection() = …; res.set_changed();`                       | Suppresses the real change then re-emits it by hand — the honest form is one ordinary write through `&mut` |
| `res.bypass_change_detection()` used to _hide_ a genuine mutation from readers | Silently desyncs consumers gated on `Changed` / `run_if`                                                   |

Root cause and fix: this dance almost always appears because the code
writes through the mutable reference **unconditionally every frame** (so
naive change detection would fire every frame) and then tries to undo
that with `bypass_change_detection()` + a conditional `set_changed()`.
The ECS-aligned fix is to mutate **conditionally** — compute the next
value from an immutable read, compare, and write through the normal
`&mut` only when it differs. Change detection then fires exactly on real
changes, for free:

```rust
// Avoid: unconditional deref_mut, then hand-managed notification.
let changed = step(state.bypass_change_detection(), &events);
if changed { state.set_changed(); }

// Prefer: write through ResMut only on a real change.
let mut next = state.clone();
if step(&mut next, &events) {
    *state = next; // the single DerefMut — change fires here, only when it differs
}
```

The same applies to components: don't assign an identical value every
frame; write with `set_if_neq` (as `resolve_gaze_target` does for
`GazeTarget`) or guard the write with an equality check.

Escape hatch (justify with a `// NOTE:`, and `#[expect]` where a lint
applies): genuine interior mutation that change detection cannot observe
— e.g. a component owning a handle/buffer whose _contents_ are mutated in
place (no `DerefMut` on the component) while a downstream system must
still be told — or a documented workaround for a specific upstream Bevy
bug. "It's simpler" or "I mutate it every frame anyway" is not a valid
reason; mutate conditionally instead.

## Bevy `Plugin::build` — method chaining

All `App` configuration calls inside a `Plugin::build` body must be written
as a single method-chain off the first call rather than as repeated `app.`
statements. This keeps the registration block visually unified and avoids
redundant `app.` noise.

Required:

```rust
// Correct: one chain, semicolon only at the end.
fn build(&self, app: &mut App) {
    app.init_resource::<Foo>()
        .add_systems(Update, my_system)
        .add_observer(my_observer)
        .add_plugins(SubPlugin);
}
```

Forbidden:

```rust
// Wrong: each call re-states `app.`.
fn build(&self, app: &mut App) {
    app.init_resource::<Foo>();
    app.add_systems(Update, my_system);
    app.add_observer(my_observer);
}
```

Exception: a call that must be preceded by local logic (e.g., a conditional
`if` that decides whether to register a system) may start a new `app.` chain
for the calls after the branch. Keep each such sub-chain as long as possible;
do not split further than the branch requires.

## Plugin registration — register in the defining file's plugin

Systems and observers are registered by a `Plugin` defined in the SAME file
that defines them, not by `add_systems` / `add_observer` in an upstream /
aggregator plugin in another file. An aggregator plugin composes the per-file
plugins with `add_plugins`.

Rationale: a file's ECS registration stays self-contained and discoverable next
to the systems it registers; parent plugins remain thin aggregators.

Required:

- A file that defines systems/observers also defines the `Plugin` that registers
  them; parent modules include it via `add_plugins`.

Forbidden:

| Pattern                                                                                                           | Why                                                 |
| ----------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| `add_systems` / `add_observer` in a parent/aggregator plugin for a system or observer defined in a different file | Hoists registration away from the code it registers |

Exceptions:

- Cross-file ordering coupling (system A in file X must run before system B in
  file Y) is expressed with `.after()` / `.before()` / a shared `SystemSet`
  (`VrmSystemSets`, `GazeResolveSet`) across the per-file plugins — NOT by
  hoisting both registrations into one upstream plugin.
- Systems defined outside this crate (Bevy's `sync_simple_transforms` and
  `propagate_parent_transforms` for the manual propagation points) are
  registered by the plugin that needs them.

## System composition — keep systems focused; split by responsibility

A Bevy system that, in one body, **gathers** input, **decides** what to do, and
**applies** the result is doing three jobs at once: it grows long, mixes
immutable reads with broad `&mut` access, and traps the decision logic behind
ECS params, making it untestable without an `App`. Split such systems along the
gather → decide → apply seam.

- **Split by responsibility — one system, one job.** A system either gathers,
  decides, or applies — not all three. When two responsibilities share a body,
  cut them into separate systems (or a system + observer), and hand off across
  the seam with an `EntityEvent`, a `Message`, or a component or resource that
  a later system in the same `SystemSet` chain reads (`GazeTarget`,
  `CursorRayThisFrame`) — never by sequencing the phases inline in one `fn`.
- **Hand off with an `EntityEvent` + observer** when the effect targets a
  specific entity and should apply at command flush. The gather system queries
  the target **immutably**, computes effects, and `commands.trigger(...)`s
  them; the observer holds the `&mut` access and writes the world.
- **Hand off with a `Message`** (`MessageWriter` → `MessageReader`, the consumer
  gated with `on_message::<T>`) when a producer system decouples work to one or
  more consumer systems in a later phase or frame — a buffered broadcast rather
  than an entity-targeted apply.
- **Extract bulky inline blocks** into named helper `fn`s so the body reads as
  gate → collect → apply. Gate preconditions with `run_if` (see "System
  optimization — gate with `run_if`").
- **Keep each system body within ~150 lines.** Exceeding the cap is almost
  always the signal that a system took on more than one responsibility; split it
  by the rules above until each system does one job. The cap counts the `fn`
  body only — signature and any `#[cfg(test)]` block are excluded — and is a
  review-time smell test, not a compile error.

Required:

| Instead of (one system does everything)                      | Use                                                                                        |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------ |
| Read input, decide, and write the world inline in one system | A pure decide helper returning effect values + an `EntityEvent`/observer that applies them |
| Decision logic interleaved with `&mut` world access          | Decision over borrowed data returning intents; mutation isolated to the apply step         |
| A 40-plus-line inline block in a system body                 | A named helper `fn` the system calls                                                       |
| A system body over ~150 lines                                | Split by responsibility until each system is one job under the cap                         |

Forbidden:

| Pattern                                                                                                             | Why                                                                                                     |
| ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `.pipe()` to chain a gather system into an apply system                                                             | Not this crate's idiom — it sequences with `.chain()` / system sets and `EntityEvent` + observer        |
| A long system whose only structure is sequential gather/decide/apply phases that could each be a helper or observer | Defeats single-responsibility; hides the apply surface                                                  |

Exceptions:

- A genuinely **single-purpose** system (only gathers, only decides, or only
  applies) is already focused — leave it.
- **Do not over-fragment.** Splitting into several systems that each re-query
  the same components (duplicate access / scheduling cost) is worse than the
  monolith when a private helper `fn` would do. Prefer a helper `fn` unless the
  apply step needs isolated `&mut` access (the observer case) or independent
  scheduling.
- The VRM update order fixes where pose work happens (`VrmSystemSets`); a split
  must keep every part inside the set the specification requires.

## Error handling — return `Result`, don't assert or unwrap

A precondition, an invariant, or a value that may be missing is reported
to the caller as an error value, not enforced with a panic. A
`debug_assert!` checks nothing in a release build, so the violated state
simply continues; an `unwrap` / `expect` turns one malformed `.vrm` or
`.vrma` into a crash of the host app. A `Result` leaves the decision to
the caller — skip the bone, keep the old state and log, drop the asset.

Required:

| Instead of                                                  | Use                                                                                    |
| ----------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `debug_assert!(index < joints.len())` guarding a function body | `let Some(joint) = joints.get(index) else { return Err(..) };`                      |
| `map.get(&name).unwrap()` / `.expect("known bone")`         | `let Some(bone) = map.get(&name) else { return Err(..) };` or `?`                     |
| `.expect(..)` on an I/O or parse result                     | `?` into the crate's `AppResult`                                                       |

- Fallible functions return the crate's `AppResult<T>` (`anyhow`). A
  fallible `pub` fn documents its failures under `# Errors`.
- A precondition the type system can state is stated there instead (a
  newtype that can only hold a valid value): the check then disappears
  rather than becoming an error.
- The place that cannot recover — a Bevy system, an observer, or the asset
  loader's entry point — handles the `Err` at that boundary: log it with
  `vrm_warn!` / `vrm_error!` and leave the state as it was.

Forbidden:

| Pattern                                                   | Why                                                       |
| --------------------------------------------------------- | --------------------------------------------------------- |
| `debug_assert!` / `debug_assert_eq!` on an input or state | Silent in release; the violated invariant keeps running   |
| `.unwrap()` / `.expect(..)` in non-test code              | A panic tears down the whole app for one bad value        |
| `panic!` / `unreachable!` / `assert!` to reject an input  | Same                                                      |

Exceptions:

- Test-only code: `#[cfg(test)] mod tests { ... }` contents, `tests/`
  files, and any module gated behind `#[cfg(test)]`: `expect("what a
  valid value looks like")` is the idiom there, and `assert!` is the
  point.
- A condition that is impossible at that point — a literal, or a value
  the same function has already checked — may use `expect` with a message
  naming the invariant, justified with a `// NOTE:` and
  `#[expect(clippy::expect_used, reason = "...")]` (or
  `clippy::unwrap_used`), as "Escape hatches" prescribes.

## Static assertions — don't pin a trait bound with a `const` guard

A trait bound a type must satisfy is not restated as a compile-time
assertion item. The pattern below — a `const fn` with the bound in its
signature, instantiated by an anonymous `const` — is forbidden wherever
it appears under `src/`:

```rust
const fn assert_send_static<T: Send + 'static>() {}
const _: () = assert_send_static::<MyEvent>();
```

The bound is already enforced where the type is used: a `Component`,
`Resource`, or `Event` derive requires `Send + Sync + 'static`, a value
stored in a `static` needs `Sync`, and a value behind `dyn Trait` names
its bounds in the trait object. Any of those stops compiling the moment
the property is lost, and the error points at the site that actually
depends on it. The guard restates that obligation somewhere the reader
cannot connect to a caller, then goes stale when the real use moves or
disappears.

Forbidden:

| Pattern | Example | Why |
| --- | --- | --- |
| `const _: () = assert_fn::<T>();` pinning a bound | `const _: () = assert_send_static::<MyEvent>();` | Duplicates an obligation the real use already enforces |
| A `const fn` whose only purpose is to carry a bound | `const fn assert_send_static<T: Send + 'static>() {}` | Same; it is an assertion wearing an item's clothes |
| `static_assertions::assert_impl_all!` and other trait-bound assertion macros | `assert_impl_all!(MyEvent: Send);` | Same, with a dependency attached |

Instead:

- **Say nothing.** This is the default. The bound is load-bearing at the
  use site; let that site fail.
- When the requirement is genuinely a contract of the type rather than a
  by-product of one caller, state it in the type's doc comment under
  `# Invariants` — prose the reader sees when they reach for the type.
- When a specific caller must be prevented from regressing, write a
  `#[test]` that exercises that caller's shape. A test carries the
  two-part doc the "Test doc comments" rule requires, so the reason the
  bound matters is written down.

Not covered by this rule (leave as-is):

- A `const` that computes a value the program uses. This rule is about
  assertions, not constants.
- A bound written in a signature, `where` clause, or trait definition —
  that is the normal way to require a bound, and the point of the rule.
- Size and layout assertions for GPU buffers or FFI (`assert_eq_size!`,
  `assert_eq_align!` and the like) — they pin a property no call site
  enforces, unlike a trait bound a caller already depends on.

## Escape hatches

When a rule is physically impossible to follow (e.g., generated code, FFI conventions, a signature Bevy dictates), justify the exception with a one-line `// NOTE:` and apply a local lint allowance:

```rust
// NOTE: Bevy requires the system to take every query by value.
#[expect(clippy::needless_pass_by_value, reason = "Bevy system signature")]
```

- Prefer `#[expect(..., reason = "...")]` over `#[allow(...)]`. `#[expect]` fails the build if the underlying lint stops firing, which prevents stale allowances from accumulating.
- Fall back to `#[allow(...)]` only when `#[expect]` is impractical (e.g., conditionally compiled code where the lint may or may not trigger).

"Hard to read" or "annoying" is not a valid reason.

## Enforcement

Tool-enforced (CI runs all of these):

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -Dwarnings` and the same with `--all-features`; the extra clippy lints are configured in `Cargo.toml`
- `cargo test` and `cargo test --doc --all-features`
- Enabling `missing_docs = "warn"` under `[lints.rust]` in `Cargo.toml` (present but commented out) would move the doc requirement to this list

Not tool-enforced — review-time check required. The following rules cannot currently be detected by `clippy` / `rustfmt` and must be checked manually during code review (and by Claude when proposing changes):

- `mod.rs` ban
- Comment taxonomy — only `// TODO:` / `// NOTE:` / `// SAFETY:`
- Comment references — code cites specifications only; no document under `docs/` appears in a comment or doc comment, by path, section number, or decision ID (see "Comment references")
- Doc comment content — doc comments carry only the allowed content; forbidden content is deleted, not relocated (see "Doc comment content — only what the reader needs")
- Comment prose — English prose in comment/doc bodies is written as complete, natural sentences, not telegraphic fragments, and kept concise (see "Comment prose — write complete English sentences")
- File-level module `//!` requirement
- Test doc comments — every `#[test]` fn documents its asserted contract plus a scenario-only `Case:` paragraph, and nothing else; pinned policies fold into the first line, never a paragraph of their own (see "Test doc comments")
- Imports — one contiguous block at the top, no glob imports except preludes, and no inline fully-qualified paths in signatures, bodies, or type parameters (see "Imports")
- `#[expect]` preference over `#[allow]`
- Visibility — in an `impl` on a type that is not itself externally `pub`, associated items are spelled `pub`, not `pub(crate)` / `pub(super)` / `pub(in path)`; private items stay private (see "Visibility — don't restate a type's own ceiling on its members")
- Item ordering — private (no-modifier) items declared after `pub` / exported ones, except that associated consts come above every method of their `impl` (see "Item ordering — private items last")
- Parameter ordering — mutable parameters declared before immutable ones in function signatures (see "Parameter ordering — mutable parameters first")
- System optimization — whole-system resource change/added guards expressed as in-body early returns must be `run_if` run conditions instead (see "System optimization — gate with `run_if`, not in-body change checks")
- Change detection — no manual `set_changed()` / `bypass_change_detection()`-then-`set_changed()` notification; mutate conditionally so normal `DerefMut` drives change detection (see "Change detection — let mutation drive it, don't force it manually")
- Naming — `Query` parameters must not use a `_q` suffix; use a descriptive singular or plural noun (see "Naming — Query parameters")
- Constructors — a function that builds a value of a local struct/enum must be an associated function on that type (`T::build`), not a free `fn build_t(…) -> T`, and a constructor taking no argument is a `Default` impl, not an inherent `fn new()` (see "Constructors — type-building functions are associated functions")
- Constants — a `const` that exactly one function names is local to that function, and a `const` that states a property of one type is an associated const on it (`CursorGazeDepth::DEFAULT_DEPTH`), declared above the methods of its `impl` whatever its visibility; module scope is the fallback (see "Constants — declare each `const` in the narrowest scope that holds its uses")
- Plugin registration — systems and observers are registered by the plugin in the file that defines them (see "Plugin registration — register in the defining file's plugin")
- System composition — long systems that interleave gather/decide/apply are split, bulky inline blocks are extracted to helpers, and each system body is kept within ~150 lines (see "System composition — keep systems focused; split by responsibility")
- Error handling — no `debug_assert!`, `unwrap`, `expect`, `panic!` or `assert!` in non-test code to enforce a precondition or reject an input; failures are returned as `AppResult` and handled at the boundary that cannot recover (see "Error handling — return `Result`, don't assert or unwrap"). Enabling `clippy::unwrap_used` / `clippy::expect_used` would move the unwrap / expect half to the tool-enforced list
- Static assertions — no `const _: () = assert_fn::<T>();` guard, no `const fn` that exists only to carry a bound, and no `static_assertions` macro (see "Static assertions — don't pin a trait bound with a `const` guard")

If you add a tool or script that detects any of these, move the corresponding entry into the tool-enforced list above.
