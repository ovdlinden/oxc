// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.

//! Infers mutable ranges for identifiers and populates Place effects.
//!
//! Ported from TypeScript `src/Inference/InferMutationAliasingRanges.ts`.
//!
//! This pass builds an abstract model of the heap and interprets the effects of
//! the given function in order to determine:
//! - The mutable ranges of all identifiers in the function
//! - The externally-visible effects of the function (mutations of params/context
//!   vars, aliasing between params/context-vars/return-value)
//! - The legacy `Effect` to store on each Place

use oxc_allocator::CloneIn;
use oxc_diagnostics::OxcDiagnostic;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::diagnostics;
use crate::react_compiler_utils::FxIndexMap;

use crate::react_compiler_hir::environment::Environment;
use crate::react_compiler_hir::type_config::{ValueKind, ValueReason};
use crate::react_compiler_hir::visitors::{
    each_instruction_value_lvalue, for_each_instruction_value_lvalue_mut,
    for_each_instruction_value_operand_mut, for_each_terminal_operand_mut,
};
use crate::react_compiler_hir::{
    AliasingEffect, ArrayElement, BlockId, DeclarationId, Effect, EvaluationOrder, FunctionId,
    HirFunction, IdentifierId, InstructionValue, MutationReason, ObjectPropertyKey,
    ObjectPropertyOrSpread, ParamPattern, Place, PlaceOrSpread, PropertyLiteral, Terminal, Type,
    is_jsx_type, is_primitive_type,
};
use crate::react_compiler_validation::validate_locals_not_reassigned_after_render::no_alias_callback_parameters;
use oxc_span::Span;

// =============================================================================
// MutationKind
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MutationKind {
    Conditional = 1,
    Definite = 2,
}

// =============================================================================
// Node and AliasingState
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeKind {
    Capture,
    Alias,
    MaybeAlias,
}

#[derive(Debug, Clone)]
struct Edge {
    index: usize,
    node: IdentifierId,
    kind: EdgeKind,
}

#[derive(Debug, Clone)]
struct MutationInfo {
    kind: MutationKind,
    span: Option<Span>,
}

#[derive(Debug, Clone)]
enum NodeValue {
    Object,
    Phi,
    Function { function_id: FunctionId },
}

#[derive(Debug, Clone)]
struct Node {
    id: IdentifierId,
    created_from: FxIndexMap<IdentifierId, usize>,
    captures: FxIndexMap<IdentifierId, usize>,
    aliases: FxIndexMap<IdentifierId, usize>,
    maybe_aliases: FxIndexMap<IdentifierId, usize>,
    edges: Vec<Edge>,
    transitive: Option<MutationInfo>,
    local: Option<MutationInfo>,
    last_mutated: usize,
    mutation_reason: Option<MutationReason>,
    value: NodeValue,
    /// Whether a mutation that may call this value has reached it. A captured
    /// callback is mutated along with the closure that captures it, but is only
    /// called when that closure calls it directly.
    called: bool,
}

impl Node {
    fn new(id: IdentifierId, value: NodeValue) -> Self {
        Node {
            id,
            created_from: FxIndexMap::default(),
            captures: FxIndexMap::default(),
            aliases: FxIndexMap::default(),
            maybe_aliases: FxIndexMap::default(),
            edges: Vec::new(),
            transitive: None,
            local: None,
            last_mutated: 0,
            mutation_reason: None,
            value,
            called: false,
        }
    }
}

struct AliasingState {
    nodes: FxHashMap<IdentifierId, Node>,
    called_declarations: FxHashMap<FunctionId, FxHashSet<DeclarationId>>,
    function_declarations: Option<FxHashMap<DeclarationId, FunctionId>>,
    function_returns: FxHashMap<FunctionId, FxHashSet<DeclarationId>>,
}

impl AliasingState {
    fn new() -> Self {
        AliasingState {
            nodes: FxHashMap::default(),
            called_declarations: FxHashMap::default(),
            function_declarations: None,
            function_returns: FxHashMap::default(),
        }
    }

    fn create(&mut self, place: &Place, value: NodeValue) {
        self.nodes.insert(place.identifier, Node::new(place.identifier, value));
    }

    fn create_from(&mut self, index: usize, from: &Place, into: &Place) {
        self.create(into, NodeValue::Object);
        let from_id = from.identifier;
        let into_id = into.identifier;
        // Add forward edge from -> into on the from node
        if let Some(from_node) = self.nodes.get_mut(&from_id) {
            from_node.edges.push(Edge { index, node: into_id, kind: EdgeKind::Alias });
        }
        // Add created_from on the into node
        if let Some(to_node) = self.nodes.get_mut(&into_id) {
            to_node.created_from.entry(from_id).or_insert(index);
        }
    }

    fn capture(&mut self, index: usize, from: &Place, into: &Place) {
        let from_id = from.identifier;
        let into_id = into.identifier;
        if !self.nodes.contains_key(&from_id) || !self.nodes.contains_key(&into_id) {
            return;
        }
        self.nodes.get_mut(&from_id).unwrap().edges.push(Edge {
            index,
            node: into_id,
            kind: EdgeKind::Capture,
        });
        self.nodes.get_mut(&into_id).unwrap().captures.entry(from_id).or_insert(index);
    }

    fn assign(&mut self, index: usize, from: &Place, into: &Place) {
        let from_id = from.identifier;
        let into_id = into.identifier;
        if !self.nodes.contains_key(&from_id) || !self.nodes.contains_key(&into_id) {
            return;
        }
        self.nodes.get_mut(&from_id).unwrap().edges.push(Edge {
            index,
            node: into_id,
            kind: EdgeKind::Alias,
        });
        self.nodes.get_mut(&into_id).unwrap().aliases.entry(from_id).or_insert(index);
    }

    fn maybe_alias(&mut self, index: usize, from: &Place, into: &Place) {
        let from_id = from.identifier;
        let into_id = into.identifier;
        if !self.nodes.contains_key(&from_id) || !self.nodes.contains_key(&into_id) {
            return;
        }
        self.nodes.get_mut(&from_id).unwrap().edges.push(Edge {
            index,
            node: into_id,
            kind: EdgeKind::MaybeAlias,
        });
        self.nodes.get_mut(&into_id).unwrap().maybe_aliases.entry(from_id).or_insert(index);
    }

    fn render(&self, index: usize, start: IdentifierId, env: &mut Environment) {
        let mut seen = FxHashSet::default();
        let mut queue: Vec<IdentifierId> = vec![start];
        while let Some(current) = queue.pop() {
            if !seen.insert(current) {
                continue;
            }
            let node = match self.nodes.get(&current) {
                Some(n) => n,
                None => continue,
            };
            if node.called {
                continue;
            }
            if let NodeValue::Function { function_id } = &node.value {
                append_function_errors(env, *function_id);
            }
            for (&alias, &when) in &node.created_from {
                if when >= index {
                    continue;
                }
                queue.push(alias);
            }
            for (&alias, &when) in &node.aliases {
                if when >= index {
                    continue;
                }
                queue.push(alias);
            }
            for (&capture, &when) in &node.captures {
                if when >= index {
                    continue;
                }
                queue.push(capture);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mutate(
        &mut self,
        index: usize,
        start: IdentifierId,
        end: Option<EvaluationOrder>, // None for simulated mutations
        transitive: bool,
        start_kind: MutationKind,
        span: Option<Span>,
        reason: Option<MutationReason>,
        env: &mut Environment,
        should_record_errors: bool,
    ) {
        #[derive(Clone)]
        struct QueueEntry {
            place: IdentifierId,
            transitive: bool,
            direction: Direction,
            kind: MutationKind,
            may_call: bool,
        }
        #[derive(Clone, Copy, PartialEq)]
        enum Direction {
            Backwards,
            Forwards,
        }

        let mut seen: FxHashMap<IdentifierId, (MutationKind, bool)> = FxHashMap::default();
        let mut queue: Vec<QueueEntry> = vec![QueueEntry {
            place: start,
            transitive,
            direction: Direction::Backwards,
            kind: start_kind,
            may_call: true,
        }];

        while let Some(entry) = queue.pop() {
            let current = entry.place;
            let previous = seen.get(&current).copied();
            if let Some((prev_kind, prev_may_call)) = previous
                && prev_kind >= entry.kind
                && (prev_may_call || !entry.may_call)
            {
                continue;
            }
            seen.insert(
                current,
                previous.map_or((entry.kind, entry.may_call), |(prev_kind, prev_may_call)| {
                    (prev_kind.max(entry.kind), prev_may_call || entry.may_call)
                }),
            );

            let node = match self.nodes.get_mut(&current) {
                Some(n) => n,
                None => continue,
            };

            if node.mutation_reason.is_none() {
                node.mutation_reason = reason;
            }
            node.last_mutated = node.last_mutated.max(index);

            if let Some(end_val) = end {
                let ident = &mut env.identifiers[node.id];
                ident.mutable_range.end = ident.mutable_range.end.max(end_val);
            }

            if let NodeValue::Function { function_id } = &node.value
                && entry.may_call
                && !node.called
                && should_record_errors
            {
                append_function_errors(env, *function_id);
            }
            node.called |= entry.may_call;

            if entry.transitive {
                match &node.transitive {
                    None => {
                        node.transitive = Some(MutationInfo { kind: entry.kind, span });
                    }
                    Some(existing) if existing.kind < entry.kind => {
                        node.transitive = Some(MutationInfo { kind: entry.kind, span });
                    }
                    _ => {}
                }
            } else {
                match &node.local {
                    None => {
                        node.local = Some(MutationInfo { kind: entry.kind, span });
                    }
                    Some(existing) if existing.kind < entry.kind => {
                        node.local = Some(MutationInfo { kind: entry.kind, span });
                    }
                    _ => {}
                }
            }

            // Forward edges: Capture a -> b, Alias a -> b: mutate(a) => mutate(b)
            // Collect edges to avoid borrow conflict
            let edges: Vec<Edge> = node.edges.clone();
            let node_value_kind = match &node.value {
                NodeValue::Phi => "Phi",
                _ => "Other",
            };
            let capturing_function = match &node.value {
                NodeValue::Function { function_id } => Some(*function_id),
                _ => None,
            };
            let node_aliases: Vec<(IdentifierId, usize)> =
                node.aliases.iter().map(|(&k, &v)| (k, v)).collect();
            let node_maybe_aliases: Vec<(IdentifierId, usize)> =
                node.maybe_aliases.iter().map(|(&k, &v)| (k, v)).collect();
            let node_captures: Vec<(IdentifierId, usize)> =
                node.captures.iter().map(|(&k, &v)| (k, v)).collect();
            let node_created_from: Vec<(IdentifierId, usize)> =
                node.created_from.iter().map(|(&k, &v)| (k, v)).collect();

            for edge in &edges {
                if edge.index >= index {
                    break;
                }
                queue.push(QueueEntry {
                    place: edge.node,
                    transitive: entry.transitive,
                    direction: Direction::Forwards,
                    // MaybeAlias edges downgrade to conditional mutation
                    kind: if edge.kind == EdgeKind::MaybeAlias {
                        MutationKind::Conditional
                    } else {
                        entry.kind
                    },
                    // Capturing a value into another value mutates the target,
                    // but does not call it. Calls are represented by `Apply`
                    // effects and reach the callback independently.
                    may_call: entry.may_call && edge.kind != EdgeKind::Capture,
                });
            }

            for (alias, when) in &node_created_from {
                if *when >= index {
                    continue;
                }
                queue.push(QueueEntry {
                    place: *alias,
                    transitive: true,
                    direction: Direction::Backwards,
                    kind: entry.kind,
                    may_call: entry.may_call,
                });
            }

            if entry.direction == Direction::Backwards || node_value_kind != "Phi" {
                // Backward alias edges
                for (alias, when) in &node_aliases {
                    if *when >= index {
                        continue;
                    }
                    queue.push(QueueEntry {
                        place: *alias,
                        transitive: entry.transitive,
                        direction: Direction::Backwards,
                        kind: entry.kind,
                        may_call: entry.may_call,
                    });
                }
                // MaybeAlias backward edges (downgrade to conditional)
                for (alias, when) in &node_maybe_aliases {
                    if *when >= index {
                        continue;
                    }
                    queue.push(QueueEntry {
                        place: *alias,
                        transitive: entry.transitive,
                        direction: Direction::Backwards,
                        kind: MutationKind::Conditional,
                        may_call: entry.may_call,
                    });
                }
            }

            // Only transitive mutations affect captures backward
            if entry.transitive {
                for (capture, when) in &node_captures {
                    if *when >= index {
                        continue;
                    }
                    // Capturing a callback does not call it: an invoked closure (for
                    // example a `map` callback) that only passes a captured `onClick`
                    // handler along still mutates it, but does not run it.
                    let may_call = entry.may_call
                        && (!matches!(
                            env.types[env.identifiers[*capture].type_],
                            Type::Function { .. }
                        ) || capturing_function.is_some_and(|function_id| {
                            let function_declarations = self
                                .function_declarations
                                .get_or_insert_with(|| collect_function_declarations(env));
                            self.called_declarations
                                .entry(function_id)
                                .or_insert_with(|| {
                                    synchronously_called_declarations(
                                        env,
                                        function_id,
                                        function_declarations,
                                        &mut self.function_returns,
                                    )
                                })
                                .contains(&env.identifiers[*capture].declaration_id)
                        }));
                    queue.push(QueueEntry {
                        place: *capture,
                        transitive: entry.transitive,
                        direction: Direction::Backwards,
                        kind: entry.kind,
                        may_call,
                    });
                }
            }
        }
    }
}

type ParameterBindings = FxHashMap<DeclarationId, FxHashSet<DeclarationId>>;
type PropertySources = FxHashMap<IdentifierId, FxHashMap<Option<String>, FxHashSet<IdentifierId>>>;

// Wider collections lose exact positions and flow through the unknown-property bucket instead of
// expanding shared spreads exponentially.
const MAX_TRACKED_ARGUMENTS: usize = 256;

/// Sparse argument slots and their inferred array-like length.
///
/// Object property names are untrusted numeric indices, so a dense vector here could allocate
/// memory proportional to an arbitrary property value rather than the source size.
#[derive(Default)]
struct CollectionSources {
    values: FxHashMap<usize, FxHashSet<IdentifierId>>,
    length: usize,
}

fn resolve_property_sources(
    object: IdentifierId,
    key: Option<&String>,
    sources: &FxHashMap<IdentifierId, FxHashSet<IdentifierId>>,
    properties: &PropertySources,
) -> FxHashSet<IdentifierId> {
    let mut result = FxHashSet::default();
    let mut pending = vec![object];
    let mut seen = FxHashSet::default();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        if let Some(values) = properties.get(&current) {
            if let Some(key) = key {
                result.extend(values.get(&Some(key.clone())).into_iter().flatten().copied());
                result.extend(values.get(&None).into_iter().flatten().copied());
            } else {
                result.extend(values.values().flatten().copied());
            }
        }
        pending.extend(sources.get(&current).into_iter().flatten().copied());
    }
    result
}

fn resolve_collection_sources(
    object: IdentifierId,
    sources: &FxHashMap<IdentifierId, FxHashSet<IdentifierId>>,
    properties: &PropertySources,
) -> Option<CollectionSources> {
    let mut result = CollectionSources::default();
    let mut pending = vec![object];
    let mut seen = FxHashSet::default();
    let mut found = false;
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        if let Some(values) = properties.get(&current) {
            found = true;
            for (key, values) in values {
                let index: usize = key.as_ref()?.parse().ok()?;
                if index >= MAX_TRACKED_ARGUMENTS {
                    return None;
                }
                result.length = result.length.max(index.checked_add(1)?);
                result.values.entry(index).or_default().extend(values);
            }
        }
        pending.extend(sources.get(&current).into_iter().flatten().copied());
    }
    if !found {
        return None;
    }
    Some(result)
}

fn resolve_call_arguments(
    arguments: &[PlaceOrSpread],
    sources: &FxHashMap<IdentifierId, FxHashSet<IdentifierId>>,
    properties: &PropertySources,
) -> Option<CollectionSources> {
    let mut result = CollectionSources::default();
    for argument in arguments {
        match argument {
            PlaceOrSpread::Place(place) => {
                if result.length >= MAX_TRACKED_ARGUMENTS {
                    return None;
                }
                result.values.insert(result.length, FxHashSet::from_iter([place.identifier]));
                result.length = result.length.checked_add(1)?;
            }
            PlaceOrSpread::Spread(spread) => {
                let spread =
                    resolve_collection_sources(spread.place.identifier, sources, properties)?;
                for (index, values) in spread.values {
                    let index = result.length.checked_add(index)?;
                    if index >= MAX_TRACKED_ARGUMENTS {
                        return None;
                    }
                    result.values.entry(index).or_default().extend(values);
                }
                result.length = result.length.checked_add(spread.length)?;
                if result.length > MAX_TRACKED_ARGUMENTS {
                    return None;
                }
            }
        }
    }
    Some(result)
}

fn append_collection_sources(
    mut prefix: CollectionSources,
    suffix: CollectionSources,
) -> Option<CollectionSources> {
    for (index, values) in suffix.values {
        let index = prefix.length.checked_add(index)?;
        if index >= MAX_TRACKED_ARGUMENTS {
            return None;
        }
        prefix.values.entry(index).or_default().extend(values);
    }
    prefix.length = prefix.length.checked_add(suffix.length)?;
    (prefix.length <= MAX_TRACKED_ARGUMENTS).then_some(prefix)
}

fn collect_function_declarations(env: &Environment) -> FxHashMap<DeclarationId, FunctionId> {
    let mut declarations = FxHashMap::default();
    for index in 0..env.functions.len() {
        let func = &env.functions[FunctionId::from_usize(index)];
        for instr in &func.instructions {
            if let InstructionValue::FunctionExpression { lowered_func, .. }
            | InstructionValue::ObjectMethod { lowered_func, .. } = &instr.value
            {
                declarations.insert(
                    env.identifiers[instr.lvalue.identifier].declaration_id,
                    lowered_func.func,
                );
            }
        }
    }
    declarations
}

fn collect_function_returns(
    env: &Environment,
    function_id: FunctionId,
) -> FxHashSet<DeclarationId> {
    let func = &env.functions[function_id];
    let mut sources: FxHashMap<IdentifierId, FxHashSet<IdentifierId>> = FxHashMap::default();
    for (_, block) in &func.body.blocks {
        for phi in &block.phis {
            sources
                .entry(phi.place.identifier)
                .or_default()
                .extend(phi.operands.values().map(|operand| operand.identifier));
        }
    }
    for instr in &func.instructions {
        let into = instr.lvalue.identifier;
        match &instr.value {
            InstructionValue::LoadLocal { place, .. }
            | InstructionValue::LoadContext { place, .. }
            | InstructionValue::TypeCastExpression { value: place, .. } => {
                sources.entry(into).or_default().insert(place.identifier);
            }
            InstructionValue::StoreLocal { lvalue, value, .. }
            | InstructionValue::StoreContext { lvalue, value, .. } => {
                sources.entry(lvalue.place.identifier).or_default().insert(value.identifier);
                sources.entry(into).or_default().insert(value.identifier);
            }
            _ => {}
        }
    }

    let mut returned = FxHashSet::default();
    for (_, block) in &func.body.blocks {
        if let Terminal::Return { value, .. } = &block.terminal {
            let mut pending = vec![value.identifier];
            let mut seen = FxHashSet::default();
            while let Some(identifier) = pending.pop() {
                if !seen.insert(identifier) {
                    continue;
                }
                returned.insert(env.identifiers[identifier].declaration_id);
                pending.extend(sources.get(&identifier).into_iter().flatten().copied());
            }
        }
    }
    returned
}

fn resolve_bound_arguments(
    identifier: IdentifierId,
    sources: &FxHashMap<IdentifierId, FxHashSet<IdentifierId>>,
    bound_arguments: &FxHashMap<IdentifierId, Vec<PlaceOrSpread>>,
) -> Option<Vec<PlaceOrSpread>> {
    let mut pending = vec![identifier];
    let mut seen = FxHashSet::default();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        if let Some(arguments) = bound_arguments.get(&current) {
            return Some(arguments.clone());
        }
        pending.extend(sources.get(&current).into_iter().flatten().copied());
    }
    None
}

fn resolve_declaration_sources(
    initial: &FxHashSet<DeclarationId>,
    bindings: &ParameterBindings,
) -> FxHashSet<DeclarationId> {
    let mut result = FxHashSet::default();
    let mut pending: Vec<_> = initial.iter().copied().collect();
    while let Some(declaration) = pending.pop() {
        if !result.insert(declaration) {
            continue;
        }
        pending.extend(bindings.get(&declaration).into_iter().flatten().copied());
    }
    result
}

fn resolve_called_declarations(
    env: &Environment,
    identifier: IdentifierId,
    sources: &FxHashMap<IdentifierId, FxHashSet<IdentifierId>>,
    bindings: &ParameterBindings,
) -> FxHashSet<DeclarationId> {
    let mut identifiers = vec![identifier];
    let mut seen_identifiers = FxHashSet::default();
    let mut pending_declarations = Vec::new();
    while let Some(current) = identifiers.pop() {
        if !seen_identifiers.insert(current) {
            continue;
        }
        pending_declarations.push(env.identifiers[current].declaration_id);
        identifiers.extend(sources.get(&current).into_iter().flatten().copied());
    }

    let mut declarations = FxHashSet::default();
    while let Some(declaration) = pending_declarations.pop() {
        if !declarations.insert(declaration) {
            continue;
        }
        pending_declarations.extend(bindings.get(&declaration).into_iter().flatten().copied());
    }
    declarations
}

/// Declarations that `function_id` calls synchronously, including through local wrappers.
///
/// Unlike the TS implementation, the Oxc-only render-purity refinement distinguishes
/// captured callbacks that are invoked from callbacks that merely escape. Resolve
/// local aliases, `Function.prototype.call`/`apply`, invoked local wrappers, and callback
/// arguments forwarded through wrapper parameters before making that distinction.
fn synchronously_called_declarations(
    env: &Environment,
    function_id: FunctionId,
    function_declarations: &FxHashMap<DeclarationId, FunctionId>,
    function_returns: &mut FxHashMap<FunctionId, FxHashSet<DeclarationId>>,
) -> FxHashSet<DeclarationId> {
    enum CalledArguments {
        Direct(Vec<PlaceOrSpread>),
        Apply(IdentifierId),
    }

    struct Called {
        callee: IdentifierId,
        arguments: Option<CalledArguments>,
        into: Option<IdentifierId>,
    }

    let argument_identifier = |argument: &PlaceOrSpread| match argument {
        PlaceOrSpread::Place(place) => place.identifier,
        PlaceOrSpread::Spread(spread) => spread.place.identifier,
    };

    let mut declarations = FxHashSet::default();
    let mut pending_functions = vec![function_id];
    let mut function_bindings: FxHashMap<FunctionId, ParameterBindings> = FxHashMap::default();
    let mut analyzed_bindings: FxHashMap<FunctionId, ParameterBindings> = FxHashMap::default();

    while let Some(function_id) = pending_functions.pop() {
        let mut bindings = function_bindings.get(&function_id).cloned().unwrap_or_default();
        if analyzed_bindings.get(&function_id) == Some(&bindings) {
            continue;
        }
        analyzed_bindings.insert(function_id, bindings.clone());

        let func = &env.functions[function_id];
        let mut sources: FxHashMap<IdentifierId, FxHashSet<IdentifierId>> = FxHashMap::default();
        let mut properties = PropertySources::default();
        let mut property_loads = Vec::new();
        let mut method_names = FxHashMap::default();
        let mut bound_arguments = FxHashMap::default();
        for (_, block) in &func.body.blocks {
            for phi in &block.phis {
                sources
                    .entry(phi.place.identifier)
                    .or_default()
                    .extend(phi.operands.values().map(|operand| operand.identifier));
            }
        }

        let mut called = Vec::new();
        for instr in &func.instructions {
            let into = instr.lvalue.identifier;
            if let InstructionValue::PropertyLoad {
                property: PropertyLiteral::String(name), ..
            } = &instr.value
            {
                method_names.insert(into, *name);
            }
            match &instr.value {
                InstructionValue::LoadLocal { place, .. }
                | InstructionValue::LoadContext { place, .. }
                | InstructionValue::TypeCastExpression { value: place, .. } => {
                    sources.entry(into).or_default().insert(place.identifier);
                }
                InstructionValue::StoreLocal { lvalue, value, .. }
                | InstructionValue::StoreContext { lvalue, value, .. } => {
                    sources.entry(lvalue.place.identifier).or_default().insert(value.identifier);
                    sources.entry(into).or_default().insert(value.identifier);
                }
                InstructionValue::ObjectExpression { properties: object_properties, .. } => {
                    let mut values: FxHashMap<_, FxHashSet<_>> = FxHashMap::default();
                    for property in object_properties {
                        match property {
                            ObjectPropertyOrSpread::Property(property) => {
                                let key = match property.key {
                                    ObjectPropertyKey::String { name, .. }
                                    | ObjectPropertyKey::Identifier { name, .. } => {
                                        Some(name.to_string())
                                    }
                                    ObjectPropertyKey::Computed { .. } => None,
                                };
                                values.entry(key).or_default().insert(property.place.identifier);
                            }
                            ObjectPropertyOrSpread::Spread(spread) => {
                                if let Some(spread_values) =
                                    properties.get(&spread.place.identifier)
                                {
                                    for (key, sources) in spread_values {
                                        values.entry(key.clone()).or_default().extend(sources);
                                    }
                                } else {
                                    values.entry(None).or_default().insert(spread.place.identifier);
                                }
                            }
                        }
                    }
                    properties.insert(into, values);
                }
                InstructionValue::ArrayExpression { elements, .. } => {
                    let mut values: FxHashMap<_, FxHashSet<_>> = FxHashMap::default();
                    let mut index: usize = 0;
                    let max_spread_index = usize::MAX - elements.len();
                    for element in elements {
                        match element {
                            ArrayElement::Place(place) => {
                                if index >= MAX_TRACKED_ARGUMENTS {
                                    values.entry(None).or_default().insert(place.identifier);
                                    continue;
                                }
                                values
                                    .entry(Some(index.to_string()))
                                    .or_default()
                                    .insert(place.identifier);
                                index += 1;
                            }
                            ArrayElement::Spread(spread) => {
                                if let Some(spread) = resolve_collection_sources(
                                    spread.place.identifier,
                                    &sources,
                                    &properties,
                                ) {
                                    for (spread_index, sources) in spread.values {
                                        let Some(spread_index) = index.checked_add(spread_index)
                                        else {
                                            values.entry(None).or_default().extend(sources);
                                            continue;
                                        };
                                        if spread_index >= MAX_TRACKED_ARGUMENTS {
                                            values.entry(None).or_default().extend(sources);
                                            continue;
                                        }
                                        values
                                            .entry(Some(spread_index.to_string()))
                                            .or_default()
                                            .extend(sources);
                                    }
                                    let Some(next_index) = index.checked_add(spread.length) else {
                                        values.entry(None).or_default();
                                        break;
                                    };
                                    if next_index > max_spread_index
                                        || next_index > MAX_TRACKED_ARGUMENTS
                                    {
                                        values.entry(None).or_default();
                                        break;
                                    }
                                    index = next_index;
                                } else {
                                    values.entry(None).or_default().extend(
                                        resolve_property_sources(
                                            spread.place.identifier,
                                            None,
                                            &sources,
                                            &properties,
                                        ),
                                    );
                                }
                            }
                            ArrayElement::Hole => index += 1,
                        }
                    }
                    properties.insert(into, values);
                }
                InstructionValue::PropertyLoad { object, property, .. } => {
                    property_loads.push((into, object.identifier, Some(property.to_string())));
                    if property.is_string("call") || property.is_string("apply") {
                        sources.entry(into).or_default().insert(object.identifier);
                    }
                }
                InstructionValue::ComputedLoad { object, .. } => {
                    property_loads.push((into, object.identifier, None));
                }
                InstructionValue::PropertyStore { object, property, value, .. } => {
                    properties
                        .entry(object.identifier)
                        .or_default()
                        .entry(Some(property.to_string()))
                        .or_default()
                        .insert(value.identifier);
                }
                InstructionValue::ComputedStore { object, value, .. } => {
                    properties
                        .entry(object.identifier)
                        .or_default()
                        .entry(None)
                        .or_default()
                        .insert(value.identifier);
                }
                _ => {}
            }

            let fallback_method = match &instr.value {
                InstructionValue::MethodCall { property, .. } => {
                    let mut pending = vec![property.identifier];
                    let mut seen = FxHashSet::default();
                    let mut name = None;
                    while let Some(current) = pending.pop() {
                        if !seen.insert(current) {
                            continue;
                        }
                        if let Some(&current) = method_names.get(&current) {
                            if name.is_some_and(|name| name != current) {
                                name = None;
                                break;
                            }
                            name = Some(current);
                        }
                        pending.extend(sources.get(&current).into_iter().flatten().copied());
                    }
                    name
                }
                _ => None,
            };

            let arguments = match &instr.value {
                InstructionValue::CallExpression { args, .. }
                | InstructionValue::NewExpression { args, .. } => {
                    Some(CalledArguments::Direct(args.iter().copied().collect()))
                }
                InstructionValue::MethodCall { args, .. }
                    if fallback_method.as_ref().is_some_and(|name| name == "call") =>
                {
                    Some(CalledArguments::Direct(args.iter().skip(1).copied().collect()))
                }
                InstructionValue::MethodCall { args, .. }
                    if fallback_method.as_ref().is_some_and(|name| name == "apply") =>
                {
                    args.get(1)
                        .map(|argument| CalledArguments::Apply(argument_identifier(argument)))
                }
                InstructionValue::MethodCall { args, .. } => {
                    Some(CalledArguments::Direct(args.iter().copied().collect()))
                }
                _ => None,
            };

            if fallback_method.as_ref().is_some_and(|name| name == "bind")
                && let InstructionValue::MethodCall { receiver, args, .. } = &instr.value
            {
                // `bind` returns a callable for its receiver without invoking it. Preserve that
                // source so a later synchronous call to the bound result reaches the receiver.
                sources.entry(into).or_default().insert(receiver.identifier);
                bound_arguments.insert(into, args.iter().skip(1).copied().collect());
            }

            match &instr.value {
                InstructionValue::CallExpression { callee, .. }
                | InstructionValue::NewExpression { callee, .. } => {
                    called.push(Called { callee: callee.identifier, arguments, into: Some(into) });
                }
                InstructionValue::MethodCall { property, .. } => {
                    called.push(Called {
                        callee: property.identifier,
                        arguments,
                        into: Some(into),
                    });
                }
                InstructionValue::TaggedTemplateExpression { tag, .. } => {
                    called.push(Called {
                        callee: tag.identifier,
                        arguments: None,
                        into: Some(into),
                    });
                }
                _ => {}
            }

            let args = match &instr.value {
                InstructionValue::CallExpression { args, .. }
                | InstructionValue::MethodCall { args, .. } => args,
                _ => continue,
            };
            if !args.iter().any(|argument| {
                let identifier = argument_identifier(argument);
                matches!(env.types[env.identifiers[identifier].type_], Type::Function { .. })
            }) {
                continue;
            }
            for index in no_alias_callback_parameters(
                &instr.value,
                env,
                fallback_method.as_ref().map(|name| name.as_str()),
                false,
                args.len(),
            ) {
                if let Some(argument) = args.get(index) {
                    called.push(Called {
                        callee: argument_identifier(argument),
                        arguments: None,
                        into: None,
                    });
                }
            }
        }

        for (into, object, key) in property_loads {
            let values = resolve_property_sources(object, key.as_ref(), &sources, &properties);
            sources.entry(into).or_default().extend(values);
        }

        for call in called {
            let callees = resolve_called_declarations(env, call.callee, &sources, &bindings);
            declarations.extend(&callees);
            for declaration in callees {
                let Some(&called_function) = function_declarations.get(&declaration) else {
                    continue;
                };
                let mut changed = false;
                let arguments = match &call.arguments {
                    Some(CalledArguments::Direct(arguments)) => {
                        let mut arguments = arguments.clone();
                        if let Some(mut bound) =
                            resolve_bound_arguments(call.callee, &sources, &bound_arguments)
                        {
                            bound.append(&mut arguments);
                            arguments = bound;
                        }
                        resolve_call_arguments(&arguments, &sources, &properties)
                    }
                    Some(CalledArguments::Apply(arguments)) => {
                        if let Some(bound) =
                            resolve_bound_arguments(call.callee, &sources, &bound_arguments)
                        {
                            resolve_call_arguments(&bound, &sources, &properties).and_then(
                                |bound| {
                                    resolve_collection_sources(*arguments, &sources, &properties)
                                        .and_then(|applied| {
                                            append_collection_sources(bound, applied)
                                        })
                                },
                            )
                        } else {
                            resolve_collection_sources(*arguments, &sources, &properties)
                        }
                    }
                    None => None,
                };
                if let Some(arguments) = arguments {
                    let argument_declarations: FxHashMap<_, _> = arguments
                        .values
                        .iter()
                        .map(|(&index, arguments)| {
                            let mut values = FxHashSet::default();
                            for &argument in arguments {
                                values.extend(resolve_called_declarations(
                                    env, argument, &sources, &bindings,
                                ));
                            }
                            (index, values)
                        })
                        .collect();
                    let target = function_bindings.entry(called_function).or_default();
                    for (index, parameter) in
                        env.functions[called_function].params.iter().enumerate()
                    {
                        let (parameter, values) = match parameter {
                            ParamPattern::Place(place) => (
                                place.identifier,
                                argument_declarations.get(&index).cloned().unwrap_or_default(),
                            ),
                            ParamPattern::Spread(spread) => (
                                spread.place.identifier,
                                argument_declarations
                                    .iter()
                                    .filter(|(argument_index, _)| **argument_index >= index)
                                    .flat_map(|(_, values)| values)
                                    .copied()
                                    .collect(),
                            ),
                        };
                        let into =
                            target.entry(env.identifiers[parameter].declaration_id).or_default();
                        let old_len = into.len();
                        into.extend(values);
                        changed |= into.len() != old_len;
                    }
                }
                if let Some(into) = call.into {
                    let returned = function_returns
                        .entry(called_function)
                        .or_insert_with(|| collect_function_returns(env, called_function));
                    let returned = function_bindings.get(&called_function).map_or_else(
                        || returned.clone(),
                        |bindings| resolve_declaration_sources(returned, bindings),
                    );
                    bindings
                        .entry(env.identifiers[into].declaration_id)
                        .or_default()
                        .extend(returned);
                }
                if changed || !analyzed_bindings.contains_key(&called_function) {
                    pending_functions.push(called_function);
                }
            }
        }
    }
    declarations
}

// =============================================================================
// Helper: append function errors
// =============================================================================

fn append_function_errors(env: &mut Environment, function_id: FunctionId) {
    let func = &env.functions[function_id];
    // Collect diagnostic ids first to avoid borrow conflict, then resolve + record.
    let ids: Vec<_> = if let Some(ref effects) = func.aliasing_effects {
        effects
            .iter()
            .filter_map(|effect| match effect {
                AliasingEffect::Impure { error, .. }
                | AliasingEffect::MutateFrozen { error, .. }
                | AliasingEffect::MutateGlobal { error, .. } => Some(*error),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    for id in ids {
        let diagnostic = env.aliasing_diagnostic(id);
        env.record_diagnostic(diagnostic);
    }
}

// =============================================================================
// Public entry point
// =============================================================================

/// Infers mutable ranges for identifiers and populates Place effects.
///
/// Returns the externally-visible effects of the function (mutations of
/// params/context-vars, aliasing between params/context-vars/return).
///
/// Corresponds to TS `inferMutationAliasingRanges(fn, {isFunctionExpression})`.
pub fn infer_mutation_aliasing_ranges<'a>(
    func: &mut HirFunction<'a>,
    env: &mut Environment<'a>,
    is_function_expression: bool,
) -> Result<Vec<AliasingEffect<'a>>, OxcDiagnostic> {
    let mut function_effects: Vec<AliasingEffect<'a>> = Vec::new();

    // =========================================================================
    // Part 1: Build data flow graph and infer mutable ranges
    // =========================================================================
    let mut state = AliasingState::new();

    struct PendingPhiOperand {
        from: Place,
        into: Place,
        index: usize,
    }
    let mut pending_phis: FxHashMap<BlockId, Vec<PendingPhiOperand>> = FxHashMap::default();

    struct PendingMutation {
        index: usize,
        id: EvaluationOrder,
        transitive: bool,
        kind: MutationKind,
        place: Place,
        reason: Option<MutationReason>,
    }
    let mut mutations: Vec<PendingMutation> = Vec::new();

    struct PendingRender {
        index: usize,
        place: Place,
    }
    let mut renders: Vec<PendingRender> = Vec::new();

    let mut index: usize = 0;

    let should_record_errors = !is_function_expression && env.enable_validations();

    // A named function expression's private name is an entry value just like
    // its parameters and context. Keep it in the local graph so effects that
    // capture or mutate the function through its recursive name are not lost.
    // It is intentionally excluded from `function_effects` below because the
    // private name is not an externally visible parameter or context value.
    if let Some(self_binding) = &func.self_binding {
        state.create(self_binding, NodeValue::Object);
    }

    // Create nodes for params, context vars, and return
    for param in &func.params {
        let place = match param {
            ParamPattern::Place(p) => p,
            ParamPattern::Spread(s) => &s.place,
        };
        state.create(place, NodeValue::Object);
    }
    for ctx in &func.context {
        state.create(ctx, NodeValue::Object);
    }
    state.create(&func.returns, NodeValue::Object);

    let mut seen_blocks: FxHashSet<BlockId> = FxHashSet::default();

    // Collect block iteration data to avoid borrow conflicts
    let block_order: Vec<BlockId> = func.body.blocks.keys().cloned().collect();

    for &block_id in &block_order {
        let block = &func.body.blocks[&block_id];

        // Process phis
        for phi in &block.phis {
            state.create(&phi.place, NodeValue::Phi);
            for (&pred, operand) in &phi.operands {
                if !seen_blocks.contains(&pred) {
                    pending_phis.entry(pred).or_default().push(PendingPhiOperand {
                        from: *operand,
                        into: phi.place,
                        index,
                    });
                    index += 1;
                } else {
                    state.assign(index, operand, &phi.place);
                    index += 1;
                }
            }
        }
        seen_blocks.insert(block_id);

        // Process instruction effects
        let instr_ids: Vec<_> = block.instructions.iter().copied().collect();
        for instr_id in &instr_ids {
            let instr = &func.instructions[instr_id.index()];
            let instr_eval_order = instr.id;
            let effects = match instr.effects.as_ref() {
                Some(e) => e.clone_in(env.allocator),
                None => continue,
            };
            for effect in &effects {
                match effect {
                    AliasingEffect::Create { into, .. } => {
                        state.create(into, NodeValue::Object);
                    }
                    AliasingEffect::CreateFunction { into, function_id, .. } => {
                        state.create(into, NodeValue::Function { function_id: *function_id });
                    }
                    AliasingEffect::CreateFrom { from, into } => {
                        state.create_from(index, from, into);
                        index += 1;
                    }
                    AliasingEffect::Assign { from, into } => {
                        if !state.nodes.contains_key(&into.identifier) {
                            state.create(into, NodeValue::Object);
                        }
                        state.assign(index, from, into);
                        index += 1;
                    }
                    AliasingEffect::Alias { from, into } => {
                        state.assign(index, from, into);
                        index += 1;
                    }
                    AliasingEffect::MaybeAlias { from, into } => {
                        state.maybe_alias(index, from, into);
                        index += 1;
                    }
                    AliasingEffect::Capture { from, into } => {
                        state.capture(index, from, into);
                        index += 1;
                    }
                    AliasingEffect::MutateTransitive { value }
                    | AliasingEffect::MutateTransitiveConditionally { value } => {
                        let is_transitive_conditional =
                            matches!(effect, AliasingEffect::MutateTransitiveConditionally { .. });
                        mutations.push(PendingMutation {
                            index,
                            id: instr_eval_order,
                            transitive: true,
                            kind: if is_transitive_conditional {
                                MutationKind::Conditional
                            } else {
                                MutationKind::Definite
                            },
                            reason: None,
                            place: *value,
                        });
                        index += 1;
                    }
                    AliasingEffect::Mutate { value, reason } => {
                        mutations.push(PendingMutation {
                            index,
                            id: instr_eval_order,
                            transitive: false,
                            kind: MutationKind::Definite,
                            reason: *reason,
                            place: *value,
                        });
                        index += 1;
                    }
                    AliasingEffect::MutateConditionally { value } => {
                        mutations.push(PendingMutation {
                            index,
                            id: instr_eval_order,
                            transitive: false,
                            kind: MutationKind::Conditional,
                            reason: None,
                            place: *value,
                        });
                        index += 1;
                    }
                    AliasingEffect::MutateFrozen { .. }
                    | AliasingEffect::MutateGlobal { .. }
                    | AliasingEffect::Impure { .. } => {
                        if should_record_errors {
                            match effect {
                                AliasingEffect::MutateFrozen { error, .. }
                                | AliasingEffect::MutateGlobal { error, .. }
                                | AliasingEffect::Impure { error, .. } => {
                                    let diagnostic = env.aliasing_diagnostic(*error);
                                    env.record_diagnostic(diagnostic);
                                }
                                _ => unreachable!(),
                            }
                        }
                        function_effects.push(effect.clone_in(env.allocator));
                    }
                    AliasingEffect::Render { place } => {
                        renders.push(PendingRender { index, place: *place });
                        index += 1;
                        function_effects.push(effect.clone_in(env.allocator));
                    }
                    // Other effects (Freeze, ImmutableCapture, Apply) are no-ops here
                    _ => {}
                }
            }
        }

        // Process pending phis for this block
        let block = &func.body.blocks[&block_id];
        if let Some(block_phis) = pending_phis.remove(&block_id) {
            for pending in block_phis {
                state.assign(pending.index, &pending.from, &pending.into);
            }
        }

        // Handle return terminal
        let terminal = &block.terminal;
        if let Terminal::Return { value, .. } = terminal {
            state.assign(index, value, &func.returns);
            index += 1;
        }

        // Handle terminal effects (MaybeThrow and Return)
        let terminal_effects = match terminal {
            Terminal::MaybeThrow { effects, .. } | Terminal::Return { effects, .. } => {
                effects.as_ref().map(|v| v.clone_in(env.allocator))
            }
            _ => None,
        };
        if let Some(effects) = terminal_effects {
            for effect in &effects {
                match effect {
                    AliasingEffect::Alias { from, into } => {
                        state.assign(index, from, into);
                        index += 1;
                    }
                    AliasingEffect::Freeze { .. } => {
                        // Expected for MaybeThrow terminals, skip
                    }
                    _ => {
                        // TS: Diagnostics.invariant(effect.kind === 'Freeze', ...)
                        // We skip non-Alias, non-Freeze effects
                    }
                }
            }
        }
    }

    // Process mutations
    for mutation in &mutations {
        state.mutate(
            mutation.index,
            mutation.place.identifier,
            Some(mutation.id + 1),
            mutation.transitive,
            mutation.kind,
            mutation.place.span,
            mutation.reason,
            env,
            should_record_errors,
        );
    }

    // Process renders
    for render in &renders {
        if should_record_errors {
            state.render(render.index, render.place.identifier, env);
        }
    }

    // Collect function effects for context vars and params
    // NOTE: TS iterates [...fn.context, ...fn.params] — context first, then params
    for ctx in &func.context {
        collect_param_effects(&state, ctx, &mut function_effects);
    }
    for param in &func.params {
        let place = match param {
            ParamPattern::Place(p) => p,
            ParamPattern::Spread(s) => &s.place,
        };
        collect_param_effects(&state, place, &mut function_effects);
    }

    // Set effect on mutated params/context vars
    // We need to do this in a separate pass because we need to know which params
    // were mutated before setting effects
    let mut captured_params: FxHashSet<IdentifierId> = FxHashSet::default();
    for param in &func.params {
        let place = match param {
            ParamPattern::Place(p) => p,
            ParamPattern::Spread(s) => &s.place,
        };
        if let Some(node) = state.nodes.get(&place.identifier)
            && (node.local.is_some() || node.transitive.is_some())
        {
            captured_params.insert(place.identifier);
        }
    }
    for ctx in &func.context {
        if let Some(node) = state.nodes.get(&ctx.identifier)
            && (node.local.is_some() || node.transitive.is_some())
        {
            captured_params.insert(ctx.identifier);
        }
    }

    // Now mutate the effects on params/context in place
    for param in &mut func.params {
        let place = match param {
            ParamPattern::Place(p) => p,
            ParamPattern::Spread(s) => &mut s.place,
        };
        if captured_params.contains(&place.identifier) {
            place.effect = Effect::Capture;
        }
    }
    for ctx in &mut func.context {
        if captured_params.contains(&ctx.identifier) {
            ctx.effect = Effect::Capture;
        }
    }

    // =========================================================================
    // Part 2: Add legacy operand-specific effects based on instruction effects
    //         and mutable ranges. Also fix up mutable range start values.
    // =========================================================================
    // Part 2 loop
    for &block_id in &block_order {
        let block = &func.body.blocks[&block_id];

        // Process phis
        let phi_data: Vec<_> = block
            .phis
            .iter()
            .map(|phi| {
                let first_instr_id = block
                    .instructions
                    .first()
                    .map(|id| func.instructions[id.index()].id)
                    .unwrap_or_else(|| block.terminal.evaluation_order());

                let is_mutated_after_creation =
                    env.identifiers[phi.place.identifier].mutable_range.end > first_instr_id;

                (
                    phi.place.identifier,
                    phi.operands.values().map(|o| o.identifier).collect::<Vec<_>>(),
                    is_mutated_after_creation,
                    first_instr_id,
                )
            })
            .collect();

        for (phi_id, _operand_ids, is_mutated_after_creation, first_instr_id) in &phi_data {
            // Set phi place effect to Store
            // We need to find this phi in the block and set it
            let block = func.body.blocks.get_mut(&block_id).unwrap();
            for phi in &mut block.phis {
                if phi.place.identifier == *phi_id {
                    phi.place.effect = Effect::Store;
                    for operand in phi.operands.values_mut() {
                        operand.effect =
                            if *is_mutated_after_creation { Effect::Capture } else { Effect::Read };
                    }
                    break;
                }
            }

            if *is_mutated_after_creation {
                let ident = &mut env.identifiers[*phi_id];
                if ident.mutable_range.start == EvaluationOrder::UNSET {
                    ident.mutable_range.start =
                        EvaluationOrder::from_usize(first_instr_id.index().saturating_sub(1));
                }
            }
        }

        let block = &func.body.blocks[&block_id];
        let instr_ids: Vec<_> = block.instructions.iter().copied().collect();

        for instr_id in &instr_ids {
            let instr = &func.instructions[instr_id.index()];
            let eval_order = instr.id;

            // Set lvalue effect to ConditionallyMutate and fix up mutable range
            // This covers the top-level lvalue
            let lvalue_id = instr.lvalue.identifier;
            {
                let ident = &mut env.identifiers[lvalue_id];
                if ident.mutable_range.start == EvaluationOrder::UNSET {
                    ident.mutable_range.start = eval_order;
                }
                if ident.mutable_range.end == EvaluationOrder::UNSET {
                    ident.mutable_range.end = (eval_order + 1).max(ident.mutable_range.end);
                }
            }
            func.instructions[instr_id.index()].lvalue.effect = Effect::ConditionallyMutate;

            // Also handle value-level lvalues (DeclareLocal, StoreLocal, etc.)
            let value_lvalue_ids: Vec<IdentifierId> =
                each_instruction_value_lvalue(&func.instructions[instr_id.index()].value)
                    .into_iter()
                    .map(|p| p.identifier)
                    .collect();
            for vlid in &value_lvalue_ids {
                let ident = &mut env.identifiers[*vlid];
                if ident.mutable_range.start == EvaluationOrder::UNSET {
                    ident.mutable_range.start = eval_order;
                }
                if ident.mutable_range.end == EvaluationOrder::UNSET {
                    ident.mutable_range.end = (eval_order + 1).max(ident.mutable_range.end);
                }
            }
            for_each_instruction_value_lvalue_mut(
                &mut func.instructions[instr_id.index()].value,
                &mut |place| {
                    place.effect = Effect::ConditionallyMutate;
                },
            );

            // Set operand effects to Read
            for_each_instruction_value_operand_mut(
                &mut func.instructions[instr_id.index()].value,
                &mut |place| {
                    place.effect = Effect::Read;
                },
            );

            let instr = &func.instructions[instr_id.index()];
            if instr.effects.is_none() {
                continue;
            }

            // Compute operand effects from instruction effects
            let effects = instr.effects.as_ref().unwrap().clone_in(env.allocator);
            let mut operand_effects: FxHashMap<IdentifierId, Effect> = FxHashMap::default();

            for effect in &effects {
                match effect {
                    AliasingEffect::Assign { from, into, .. }
                    | AliasingEffect::Alias { from, into }
                    | AliasingEffect::Capture { from, into }
                    | AliasingEffect::CreateFrom { from, into }
                    | AliasingEffect::MaybeAlias { from, into } => {
                        let is_mutated_or_reassigned =
                            env.identifiers[into.identifier].mutable_range.end > eval_order;
                        if is_mutated_or_reassigned {
                            operand_effects.insert(from.identifier, Effect::Capture);
                            operand_effects.insert(into.identifier, Effect::Store);
                        } else {
                            operand_effects.insert(from.identifier, Effect::Read);
                            operand_effects.insert(into.identifier, Effect::Store);
                        }
                    }
                    AliasingEffect::CreateFunction { .. } | AliasingEffect::Create { .. } => {
                        // no-op
                    }
                    AliasingEffect::Mutate { value, .. } => {
                        operand_effects.insert(value.identifier, Effect::Store);
                    }
                    AliasingEffect::Apply { .. } => {
                        return Err(diagnostics::invariant_analyze_functions_expected_apply_effects_replaced_more_precise_effects_2());
                    }
                    AliasingEffect::MutateTransitive { value, .. }
                    | AliasingEffect::MutateConditionally { value }
                    | AliasingEffect::MutateTransitiveConditionally { value } => {
                        operand_effects.insert(value.identifier, Effect::ConditionallyMutate);
                    }
                    AliasingEffect::Freeze { value, .. } => {
                        operand_effects.insert(value.identifier, Effect::Freeze);
                    }
                    AliasingEffect::ImmutableCapture { .. } => {
                        // no-op, Read is the default
                    }
                    AliasingEffect::Impure { .. }
                    | AliasingEffect::Render { .. }
                    | AliasingEffect::MutateFrozen { .. }
                    | AliasingEffect::MutateGlobal { .. } => {
                        // no-op
                    }
                }
            }

            // Apply operand effects to top-level lvalue
            let instr = &mut func.instructions[instr_id.index()];
            let lvalue_id = instr.lvalue.identifier;
            if let Some(&effect) = operand_effects.get(&lvalue_id) {
                instr.lvalue.effect = effect;
            }
            // Apply operand effects to value-level lvalues
            for_each_instruction_value_lvalue_mut(&mut instr.value, &mut |place| {
                if let Some(&effect) = operand_effects.get(&place.identifier) {
                    place.effect = effect;
                }
            });

            // Apply operand effects to value operands and fix up mutable ranges
            {
                let mut apply = |place: &mut Place| {
                    // Fix up mutable range start
                    let ident = &env.identifiers[place.identifier];
                    if ident.mutable_range.end > eval_order
                        && ident.mutable_range.start == EvaluationOrder::UNSET
                    {
                        env.identifiers[place.identifier].mutable_range.start = eval_order;
                    }
                    // Apply effect
                    if let Some(&effect) = operand_effects.get(&place.identifier) {
                        place.effect = effect;
                    }
                };
                for_each_instruction_value_operand_mut(&mut instr.value, &mut apply);

                // FunctionExpression/ObjectMethod context variables are operands that
                // require env access (they live in env.functions[func_id].context).
                if let InstructionValue::FunctionExpression { lowered_func, .. }
                | InstructionValue::ObjectMethod { lowered_func, .. } = &instr.value
                {
                    let func_id = lowered_func.func;
                    let ctx_ids: Vec<IdentifierId> =
                        env.functions[func_id].context.iter().map(|c| c.identifier).collect();
                    for ctx_id in &ctx_ids {
                        let ident = &env.identifiers[*ctx_id];
                        if ident.mutable_range.end > eval_order
                            && ident.mutable_range.start == EvaluationOrder::UNSET
                        {
                            env.identifiers[*ctx_id].mutable_range.start = eval_order;
                        }
                        let effect = operand_effects.get(ctx_id).copied().unwrap_or(Effect::Read);
                        let inner_func = &mut env.functions[func_id];
                        for ctx_place in &mut inner_func.context {
                            if ctx_place.identifier == *ctx_id {
                                ctx_place.effect = effect;
                            }
                        }
                    }
                }
            }

            // Handle StoreContext case: extend rvalue range if needed
            let instr = &func.instructions[instr_id.index()];
            if let InstructionValue::StoreContext { value, .. } = &instr.value {
                let val_id = value.identifier;
                let val_range_end = env.identifiers[val_id].mutable_range.end;
                if val_range_end <= eval_order {
                    env.identifiers[val_id].mutable_range.end = eval_order + 1;
                }
            }
        }

        // Set terminal operand effects
        let block = func.body.blocks.get_mut(&block_id).unwrap();
        match &mut block.terminal {
            Terminal::Return { value, .. } => {
                value.effect = if is_function_expression { Effect::Read } else { Effect::Freeze };
            }
            terminal => {
                for_each_terminal_operand_mut(terminal, &mut |place| {
                    place.effect = Effect::Read;
                });
            }
        }
    }

    // =========================================================================
    // Part 3: Finish populating the externally visible effects
    // =========================================================================
    let returns_id = func.returns.identifier;
    let returns_type_id = env.identifiers[returns_id].type_;
    let returns_type = &env.types[returns_type_id];
    let return_value_kind = if is_primitive_type(returns_type) {
        ValueKind::Primitive
    } else if is_jsx_type(returns_type) {
        ValueKind::Frozen
    } else {
        ValueKind::Mutable
    };

    function_effects.push(AliasingEffect::Create {
        into: func.returns,
        value: return_value_kind,
        reason: ValueReason::KnownReturnSignature,
    });

    // Determine precise data-flow effects by simulating transitive mutations
    let mut tracked: Vec<Place> = Vec::new();
    for param in &func.params {
        let place = match param {
            ParamPattern::Place(p) => *p,
            ParamPattern::Spread(s) => s.place,
        };
        tracked.push(place);
    }
    for ctx in &func.context {
        tracked.push(*ctx);
    }
    tracked.push(func.returns);

    let returns_identifier_id = func.returns.identifier;

    for i in 0..tracked.len() {
        let into = tracked[i];
        let mutation_index = index;
        index += 1;

        state.mutate(
            mutation_index,
            into.identifier,
            None, // simulated mutation
            true,
            MutationKind::Conditional,
            into.span,
            None,
            env,
            false, // never record errors for simulated mutations
        );

        for from in &tracked {
            if from.identifier == into.identifier || from.identifier == returns_identifier_id {
                continue;
            }

            let from_node = state.nodes.get(&from.identifier);
            assert!(
                from_node.is_some(),
                "Expected a node to exist for all parameters and context variables"
            );
            let from_node = from_node.unwrap();

            if from_node.last_mutated == mutation_index {
                if into.identifier == returns_identifier_id {
                    function_effects.push(AliasingEffect::Alias { from: *from, into });
                } else {
                    function_effects.push(AliasingEffect::Capture { from: *from, into });
                }
            }
        }
    }

    Ok(function_effects)
}

// =============================================================================
// Helper: collect param/context mutation effects
// =============================================================================

fn collect_param_effects<'a>(
    state: &AliasingState,
    place: &Place,
    function_effects: &mut Vec<AliasingEffect<'a>>,
) {
    let node = match state.nodes.get(&place.identifier) {
        Some(n) => n,
        None => return,
    };

    if let Some(ref local) = node.local {
        match local.kind {
            MutationKind::Conditional => {
                function_effects.push(AliasingEffect::MutateConditionally {
                    value: Place { span: local.span, ..*place },
                });
            }
            MutationKind::Definite => {
                function_effects.push(AliasingEffect::Mutate {
                    value: Place { span: local.span, ..*place },
                    reason: node.mutation_reason,
                });
            }
        }
    }

    if let Some(ref transitive) = node.transitive {
        match transitive.kind {
            MutationKind::Conditional => {
                function_effects.push(AliasingEffect::MutateTransitiveConditionally {
                    value: Place { span: transitive.span, ..*place },
                });
            }
            MutationKind::Definite => {
                function_effects.push(AliasingEffect::MutateTransitive {
                    value: Place { span: transitive.span, ..*place },
                });
            }
        }
    }
}
