// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.

//! Default module type provider, ported from DefaultModuleTypeProvider.ts.
//!
//! Provides hardcoded type overrides for known third-party libraries.

use crate::react_compiler_utils::FxIndexMap;

use crate::react_compiler_hir::Effect;
use crate::react_compiler_hir::type_config::{
    AliasingEffectConfig, AliasingSignatureConfig, ApplyArgConfig, BuiltInTypeRef,
    FunctionTypeConfig, HookTypeConfig, ObjectTypeConfig, TypeConfig, TypeReferenceConfig,
    ValueKind, ValueReason,
};

const STRING_REPLACE_TO_ARRAY_ALIASING: AliasingSignatureConfig = AliasingSignatureConfig {
    receiver: "@callee",
    params: &["@text", "@pattern", "@callback"],
    rest: None,
    returns: "@returns",
    temporaries: &["@match", "@callbackReturn"],
    effects: &[
        AliasingEffectConfig::Create {
            into: "@returns",
            value: ValueKind::Mutable,
            reason: ValueReason::KnownReturnSignature,
        },
        AliasingEffectConfig::CreateFrom { from: "@text", into: "@match" },
        AliasingEffectConfig::Apply {
            receiver: "@callee",
            function: "@callback",
            mutates_function: false,
            args: &[ApplyArgConfig::Place("@match")],
            into: "@callbackReturn",
        },
        AliasingEffectConfig::Capture { from: "@callbackReturn", into: "@returns" },
    ],
};

/// Returns type configuration for known third-party modules. Ported from TS
/// `defaultModuleTypeProvider`.
pub fn default_module_type_provider(module_name: &str) -> Option<TypeConfig> {
    match module_name {
        "react-hook-form" => Some(TypeConfig::Object(ObjectTypeConfig {
            properties: Some(FxIndexMap::from_iter([(
                "useForm".to_string(),
                TypeConfig::Hook(HookTypeConfig {
                    return_type: Box::new(TypeConfig::Object(ObjectTypeConfig {
                        properties: Some(FxIndexMap::from_iter([(
                            "watch".to_string(),
                            TypeConfig::Function(FunctionTypeConfig {
                                positional_params: Vec::new(),
                                rest_param: Some(Effect::Read),
                                callee_effect: Effect::Read,
                                return_type: Box::new(TypeConfig::TypeReference(
                                    TypeReferenceConfig {
                                        name: BuiltInTypeRef::Any,
                                    },
                                )),
                                return_value_kind: ValueKind::Mutable,
                                no_alias: None,
                                mutable_only_if_operands_are_mutable: None,
                                impure: None,
                                canonical_name: None,
                                aliasing: None,
                                known_incompatible: Some(
                                    "React Hook Form's `useForm()` API returns a `watch()` function which cannot be memoized safely.".to_string(),
                                ),
                            }),
                        )])),
                    })),
                    positional_params: None,
                    rest_param: None,
                    return_value_kind: None,
                    no_alias: None,
                    aliasing: None,
                    known_incompatible: None,
                }),
            )])),
        })),

        "@tanstack/react-table" => Some(TypeConfig::Object(ObjectTypeConfig {
            properties: Some(FxIndexMap::from_iter([(
                "useReactTable".to_string(),
                TypeConfig::Hook(HookTypeConfig {
                    positional_params: Some(Vec::new()),
                    rest_param: Some(Effect::Read),
                    return_type: Box::new(TypeConfig::TypeReference(TypeReferenceConfig {
                        name: BuiltInTypeRef::Any,
                    })),
                    return_value_kind: None,
                    no_alias: None,
                    aliasing: None,
                    known_incompatible: Some(
                        "TanStack Table's `useReactTable()` API returns functions that cannot be memoized safely".to_string(),
                    ),
                }),
            )])),
        })),

        "@tanstack/react-virtual" => Some(TypeConfig::Object(ObjectTypeConfig {
            properties: Some(FxIndexMap::from_iter([
                (
                    "useVirtualizer".to_string(),
                    TypeConfig::Hook(HookTypeConfig {
                        positional_params: Some(Vec::new()),
                        rest_param: Some(Effect::Read),
                        return_type: Box::new(TypeConfig::TypeReference(TypeReferenceConfig {
                            name: BuiltInTypeRef::Any,
                        })),
                        return_value_kind: None,
                        no_alias: None,
                        aliasing: None,
                        known_incompatible: Some(
                            "TanStack Virtual's `useVirtualizer()` API returns functions that cannot be memoized safely".to_string(),
                        ),
                    }),
                ),
                // `useWindowVirtualizer()` wraps the same virtualizer instance as
                // `useVirtualizer()`, so its return value is incompatible for the same reason.
                (
                    "useWindowVirtualizer".to_string(),
                    TypeConfig::Hook(HookTypeConfig {
                        positional_params: Some(Vec::new()),
                        rest_param: Some(Effect::Read),
                        return_type: Box::new(TypeConfig::TypeReference(TypeReferenceConfig {
                            name: BuiltInTypeRef::Any,
                        })),
                        return_value_kind: None,
                        no_alias: None,
                        aliasing: None,
                        known_incompatible: Some(
                            "TanStack Virtual's `useWindowVirtualizer()` API returns functions that cannot be memoized safely".to_string(),
                        ),
                    }),
                ),
            ])),
        })),

        // Unlike the TS implementation, Oxc does not have a runtime module type
        // provider to model `useAnimate`'s ref-like animation scope.
        "framer-motion" => Some(TypeConfig::Object(ObjectTypeConfig {
            properties: Some(FxIndexMap::from_iter([(
                "useAnimate".to_string(),
                TypeConfig::Hook(HookTypeConfig {
                    positional_params: Some(Vec::new()),
                    rest_param: None,
                    return_type: Box::new(TypeConfig::Object(ObjectTypeConfig {
                        properties: Some(FxIndexMap::from_iter([
                            (
                                "0".to_string(),
                                TypeConfig::TypeReference(TypeReferenceConfig {
                                    name: BuiltInTypeRef::Ref,
                                }),
                            ),
                            (
                                "1".to_string(),
                                TypeConfig::TypeReference(TypeReferenceConfig {
                                    name: BuiltInTypeRef::Any,
                                }),
                            ),
                        ])),
                    })),
                    return_value_kind: None,
                    no_alias: None,
                    aliasing: None,
                    known_incompatible: None,
                }),
            )])),
        })),

        // Unlike the TS implementation, Oxc cannot evaluate this module to
        // discover that its replacement callback runs synchronously and that
        // callback results are captured into the returned array.
        "string-replace-to-array" => Some(TypeConfig::Object(ObjectTypeConfig {
            properties: Some(FxIndexMap::from_iter([(
                "default".to_string(),
                TypeConfig::Function(FunctionTypeConfig {
                    positional_params: vec![
                        Effect::Read,
                        Effect::Read,
                        Effect::ConditionallyMutate,
                    ],
                    rest_param: None,
                    callee_effect: Effect::Read,
                    return_type: Box::new(TypeConfig::TypeReference(TypeReferenceConfig {
                        name: BuiltInTypeRef::Array,
                    })),
                    return_value_kind: ValueKind::Mutable,
                    no_alias: Some(true),
                    mutable_only_if_operands_are_mutable: None,
                    impure: None,
                    canonical_name: Some("string-replace-to-array".to_string()),
                    aliasing: Some(STRING_REPLACE_TO_ARRAY_ALIASING),
                    known_incompatible: None,
                }),
            )])),
        })),

        _ => None,
    }
}
