use aivi_hir::ItemId as HirItemId;

use crate::{
    BuiltinAppendCarrier, BuiltinApplicativeCarrier, BuiltinApplyCarrier, BuiltinBifunctorCarrier,
    BuiltinClassMemberIntrinsic, BuiltinFilterableCarrier, BuiltinFoldableCarrier,
    BuiltinFunctorCarrier, BuiltinMonadCarrier, BuiltinOrdSubject, BuiltinTraversableCarrier,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinExecutableClass {
    Eq,
    Ord,
    Semigroup,
    Monoid,
    Functor,
    Bifunctor,
    Applicative,
    Apply,
    Chain,
    Monad,
    Foldable,
    Traversable,
    Filterable,
}

impl BuiltinExecutableClass {
    pub const fn doc_label(self) -> &'static str {
        match self {
            Self::Functor => "Functor",
            Self::Apply => "Apply",
            Self::Applicative => "Applicative",
            Self::Monad => "Monad",
            Self::Foldable => "Foldable",
            Self::Traversable => "Traversable",
            Self::Filterable => "Filterable",
            Self::Bifunctor => "Bifunctor",
            Self::Eq => "Eq",
            Self::Ord => "Ord",
            Self::Semigroup => "Semigroup",
            Self::Monoid => "Monoid",
            Self::Chain => "Chain",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltinExecutableCarrier {
    Int,
    Float,
    Decimal,
    BigInt,
    Bool,
    Text,
    Ordering,
    List,
    Option,
    Result,
    Validation,
    Signal,
    Task,
}

impl BuiltinExecutableCarrier {
    pub const fn doc_label(self) -> &'static str {
        match self {
            Self::Int => "Int",
            Self::Float => "Float",
            Self::Decimal => "Decimal",
            Self::BigInt => "BigInt",
            Self::Bool => "Bool",
            Self::Text => "Text",
            Self::Ordering => "Ordering",
            Self::List => "List",
            Self::Option => "Option",
            Self::Result => "Result E",
            Self::Validation => "Validation E",
            Self::Signal => "Signal",
            Self::Task => "Task E",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinExecutableClassSupport {
    Supported,
    Unsupported(&'static str),
}

impl BuiltinExecutableClassSupport {
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Supported)
    }

    pub const fn reason(self) -> Option<&'static str> {
        match self {
            Self::Supported => None,
            Self::Unsupported(reason) => Some(reason),
        }
    }
}

pub const HIGHER_KINDED_DOC_CLASSES: [BuiltinExecutableClass; 8] = [
    BuiltinExecutableClass::Functor,
    BuiltinExecutableClass::Apply,
    BuiltinExecutableClass::Applicative,
    BuiltinExecutableClass::Monad,
    BuiltinExecutableClass::Foldable,
    BuiltinExecutableClass::Traversable,
    BuiltinExecutableClass::Filterable,
    BuiltinExecutableClass::Bifunctor,
];

pub const HIGHER_KINDED_DOC_CARRIERS: [BuiltinExecutableCarrier; 6] = [
    BuiltinExecutableCarrier::List,
    BuiltinExecutableCarrier::Option,
    BuiltinExecutableCarrier::Result,
    BuiltinExecutableCarrier::Validation,
    BuiltinExecutableCarrier::Signal,
    BuiltinExecutableCarrier::Task,
];

pub const TRAVERSE_RESULT_APPLICATIVE_CARRIERS: [BuiltinExecutableCarrier; 6] = [
    BuiltinExecutableCarrier::List,
    BuiltinExecutableCarrier::Option,
    BuiltinExecutableCarrier::Result,
    BuiltinExecutableCarrier::Validation,
    BuiltinExecutableCarrier::Signal,
    BuiltinExecutableCarrier::Task,
];

pub const fn builtin_executable_class_support(
    class: BuiltinExecutableClass,
    carrier: BuiltinExecutableCarrier,
) -> BuiltinExecutableClassSupport {
    match class {
        BuiltinExecutableClass::Eq => BuiltinExecutableClassSupport::Supported,
        BuiltinExecutableClass::Ord => match carrier {
            BuiltinExecutableCarrier::Int
            | BuiltinExecutableCarrier::Float
            | BuiltinExecutableCarrier::Decimal
            | BuiltinExecutableCarrier::BigInt
            | BuiltinExecutableCarrier::Bool
            | BuiltinExecutableCarrier::Text
            | BuiltinExecutableCarrier::Ordering => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports compare for Int, Float, Decimal, BigInt, Bool, Text, and Ordering",
            ),
        },
        BuiltinExecutableClass::Semigroup => match carrier {
            BuiltinExecutableCarrier::Text | BuiltinExecutableCarrier::List => {
                BuiltinExecutableClassSupport::Supported
            }
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports append for Text and List",
            ),
        },
        BuiltinExecutableClass::Monoid => match carrier {
            BuiltinExecutableCarrier::Text | BuiltinExecutableCarrier::List => {
                BuiltinExecutableClassSupport::Supported
            }
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports empty for Text and List",
            ),
        },
        BuiltinExecutableClass::Functor => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Validation
            | BuiltinExecutableCarrier::Signal
            | BuiltinExecutableCarrier::Task => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports map for List, Option, Result, Validation, Signal, and Task",
            ),
        },
        BuiltinExecutableClass::Bifunctor => match carrier {
            BuiltinExecutableCarrier::Result | BuiltinExecutableCarrier::Validation => {
                BuiltinExecutableClassSupport::Supported
            }
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports bimap for Result and Validation",
            ),
        },
        BuiltinExecutableClass::Applicative => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Validation
            | BuiltinExecutableCarrier::Signal
            | BuiltinExecutableCarrier::Task => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports pure for List, Option, Result, Validation, Signal, and Task",
            ),
        },
        BuiltinExecutableClass::Apply => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Validation
            | BuiltinExecutableCarrier::Signal
            | BuiltinExecutableCarrier::Task => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports apply for List, Option, Result, Validation, Signal, and Task",
            ),
        },
        BuiltinExecutableClass::Chain => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Task => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports chain for List, Option, Result, and Task",
            ),
        },
        BuiltinExecutableClass::Monad => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Task => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports join for List, Option, Result, and Task",
            ),
        },
        BuiltinExecutableClass::Foldable => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Validation => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports reduce for List, Option, Result, and Validation",
            ),
        },
        BuiltinExecutableClass::Traversable => match carrier {
            BuiltinExecutableCarrier::List
            | BuiltinExecutableCarrier::Option
            | BuiltinExecutableCarrier::Result
            | BuiltinExecutableCarrier::Validation => BuiltinExecutableClassSupport::Supported,
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports traverse for List, Option, Result, and Validation",
            ),
        },
        BuiltinExecutableClass::Filterable => match carrier {
            BuiltinExecutableCarrier::List | BuiltinExecutableCarrier::Option => {
                BuiltinExecutableClassSupport::Supported
            }
            _ => BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports filterMap for List and Option",
            ),
        },
    }
}

pub const fn builtin_traverse_result_applicative_support(
    carrier: BuiltinExecutableCarrier,
) -> BuiltinExecutableClassSupport {
    match carrier {
        BuiltinExecutableCarrier::List
        | BuiltinExecutableCarrier::Option
        | BuiltinExecutableCarrier::Result
        | BuiltinExecutableCarrier::Validation
        | BuiltinExecutableCarrier::Signal => BuiltinExecutableClassSupport::Supported,
        _ => BuiltinExecutableClassSupport::Unsupported(
            "runtime lowering only supports traverse results in List, Option, Result, Validation, and Signal applicatives",
        ),
    }
}

pub fn builtin_compare_intrinsic(
    carrier: BuiltinExecutableCarrier,
    ordering_item: HirItemId,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Ord, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Compare {
                    subject: builtin_ord_subject(carrier).expect(
                        "supported compare carriers always map to builtin compare subjects",
                    ),
                    ordering_item,
                })
            },
            Err,
        )
}

pub fn builtin_append_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Semigroup, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Append(
                    builtin_append_carrier(carrier)
                        .expect("supported append carriers always map to builtin append carriers"),
                ))
            },
            Err,
        )
}

pub fn builtin_empty_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Monoid, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Empty(
                    builtin_append_carrier(carrier)
                        .expect("supported empty carriers always map to builtin append carriers"),
                ))
            },
            Err,
        )
}

pub fn builtin_map_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Functor, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Map(
                    builtin_functor_carrier(carrier)
                        .expect("supported map carriers always map to builtin functor carriers"),
                ))
            },
            Err,
        )
}

pub fn builtin_bimap_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Bifunctor, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Bimap(
                    builtin_bifunctor_carrier(carrier).expect(
                        "supported bimap carriers always map to builtin bifunctor carriers",
                    ),
                ))
            },
            Err,
        )
}

pub fn builtin_pure_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Applicative, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Pure(
                    builtin_applicative_carrier(carrier).expect(
                        "supported pure carriers always map to builtin applicative carriers",
                    ),
                ))
            },
            Err,
        )
}

pub fn builtin_apply_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Apply, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Apply(
                    builtin_apply_carrier(carrier)
                        .expect("supported apply carriers always map to builtin apply carriers"),
                ))
            },
            Err,
        )
}

pub fn builtin_chain_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Chain, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Chain(
                    builtin_monad_carrier(carrier)
                        .expect("supported chain carriers always map to builtin monad carriers"),
                ))
            },
            Err,
        )
}

pub fn builtin_join_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Monad, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Join(
                    builtin_monad_carrier(carrier)
                        .expect("supported join carriers always map to builtin monad carriers"),
                ))
            },
            Err,
        )
}

pub fn builtin_reduce_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Foldable, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::Reduce(
                    builtin_foldable_carrier(carrier).expect(
                        "supported reduce carriers always map to builtin foldable carriers",
                    ),
                ))
            },
            Err,
        )
}

pub fn builtin_traverse_intrinsic(
    traversable: BuiltinExecutableCarrier,
    applicative: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    if let Some(reason) =
        builtin_executable_class_support(BuiltinExecutableClass::Traversable, traversable).reason()
    {
        return Err(reason);
    }
    if let Some(reason) = builtin_traverse_result_applicative_support(applicative).reason() {
        return Err(reason);
    }
    Ok(BuiltinClassMemberIntrinsic::Traverse {
        traversable: builtin_traversable_carrier(traversable)
            .expect("supported traverse carriers always map to builtin traversable carriers"),
        applicative: builtin_applicative_carrier(applicative)
            .expect("supported traverse applicatives always map to builtin applicative carriers"),
    })
}

pub fn builtin_filter_map_intrinsic(
    carrier: BuiltinExecutableCarrier,
) -> Result<BuiltinClassMemberIntrinsic, &'static str> {
    builtin_executable_class_support(BuiltinExecutableClass::Filterable, carrier)
        .reason()
        .map_or_else(
            || {
                Ok(BuiltinClassMemberIntrinsic::FilterMap(
                    builtin_filterable_carrier(carrier).expect(
                        "supported filterMap carriers always map to builtin filterable carriers",
                    ),
                ))
            },
            Err,
        )
}

pub fn render_higher_kinded_builtin_support_markdown() -> String {
    let mut markdown = String::new();
    markdown.push_str("| Builtin carrier |");
    for class in HIGHER_KINDED_DOC_CLASSES {
        markdown.push(' ');
        markdown.push_str(class.doc_label());
        markdown.push_str(" |");
    }
    markdown.push('\n');
    markdown.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for carrier in HIGHER_KINDED_DOC_CARRIERS {
        markdown.push_str("| `");
        markdown.push_str(carrier.doc_label());
        markdown.push_str("` |");
        for class in HIGHER_KINDED_DOC_CLASSES {
            debug_assert_eq!(
                builtin_executable_class_support(BuiltinExecutableClass::Chain, carrier)
                    .is_supported(),
                builtin_executable_class_support(BuiltinExecutableClass::Monad, carrier)
                    .is_supported(),
                "Chain and Monad registry entries must stay aligned for docs",
            );
            markdown.push(' ');
            markdown.push_str(
                if builtin_executable_class_support(class, carrier).is_supported() {
                    "yes"
                } else {
                    "—"
                },
            );
            markdown.push_str(" |");
        }
        markdown.push('\n');
    }
    markdown.push('\n');
    markdown.push_str("- The `Monad` column means builtin executable lowering for `chain` and `join`; `Chain` uses the same registry entries.\n");
    markdown.push_str("- `—` means the canonical executable-support registry marks that builtin class/carrier pair unsupported.\n");
    markdown.push_str("- `Signal` is intentionally **not** a `Monad`: executable signals keep a static dependency graph.\n");
    markdown.push_str("- `Validation E` is intentionally **not** a `Monad`: independent accumulation stays applicative (`&|>` / `zipValidation`), while dependent `!|>` checks are a dedicated pipe primitive rather than class-backed `bind`.\n");
    markdown.push_str(
        "- Traverse result applicatives are builtin-supported for `List`, `Option`, `Result`, `Validation`, and `Signal`, but not for `Task`.\n",
    );
    markdown
}

fn builtin_ord_subject(carrier: BuiltinExecutableCarrier) -> Option<BuiltinOrdSubject> {
    match carrier {
        BuiltinExecutableCarrier::Int => Some(BuiltinOrdSubject::Int),
        BuiltinExecutableCarrier::Float => Some(BuiltinOrdSubject::Float),
        BuiltinExecutableCarrier::Decimal => Some(BuiltinOrdSubject::Decimal),
        BuiltinExecutableCarrier::BigInt => Some(BuiltinOrdSubject::BigInt),
        BuiltinExecutableCarrier::Bool => Some(BuiltinOrdSubject::Bool),
        BuiltinExecutableCarrier::Text => Some(BuiltinOrdSubject::Text),
        BuiltinExecutableCarrier::Ordering => Some(BuiltinOrdSubject::Ordering),
        _ => None,
    }
}

fn builtin_append_carrier(carrier: BuiltinExecutableCarrier) -> Option<BuiltinAppendCarrier> {
    match carrier {
        BuiltinExecutableCarrier::Text => Some(BuiltinAppendCarrier::Text),
        BuiltinExecutableCarrier::List => Some(BuiltinAppendCarrier::List),
        _ => None,
    }
}

fn builtin_functor_carrier(carrier: BuiltinExecutableCarrier) -> Option<BuiltinFunctorCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinFunctorCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinFunctorCarrier::Option),
        BuiltinExecutableCarrier::Result => Some(BuiltinFunctorCarrier::Result),
        BuiltinExecutableCarrier::Validation => Some(BuiltinFunctorCarrier::Validation),
        BuiltinExecutableCarrier::Signal => Some(BuiltinFunctorCarrier::Signal),
        BuiltinExecutableCarrier::Task => Some(BuiltinFunctorCarrier::Task),
        _ => None,
    }
}

fn builtin_bifunctor_carrier(carrier: BuiltinExecutableCarrier) -> Option<BuiltinBifunctorCarrier> {
    match carrier {
        BuiltinExecutableCarrier::Result => Some(BuiltinBifunctorCarrier::Result),
        BuiltinExecutableCarrier::Validation => Some(BuiltinBifunctorCarrier::Validation),
        _ => None,
    }
}

fn builtin_applicative_carrier(
    carrier: BuiltinExecutableCarrier,
) -> Option<BuiltinApplicativeCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinApplicativeCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinApplicativeCarrier::Option),
        BuiltinExecutableCarrier::Result => Some(BuiltinApplicativeCarrier::Result),
        BuiltinExecutableCarrier::Validation => Some(BuiltinApplicativeCarrier::Validation),
        BuiltinExecutableCarrier::Signal => Some(BuiltinApplicativeCarrier::Signal),
        BuiltinExecutableCarrier::Task => Some(BuiltinApplicativeCarrier::Task),
        _ => None,
    }
}

fn builtin_apply_carrier(carrier: BuiltinExecutableCarrier) -> Option<BuiltinApplyCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinApplyCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinApplyCarrier::Option),
        BuiltinExecutableCarrier::Result => Some(BuiltinApplyCarrier::Result),
        BuiltinExecutableCarrier::Validation => Some(BuiltinApplyCarrier::Validation),
        BuiltinExecutableCarrier::Signal => Some(BuiltinApplyCarrier::Signal),
        BuiltinExecutableCarrier::Task => Some(BuiltinApplyCarrier::Task),
        _ => None,
    }
}

fn builtin_monad_carrier(carrier: BuiltinExecutableCarrier) -> Option<BuiltinMonadCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinMonadCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinMonadCarrier::Option),
        BuiltinExecutableCarrier::Result => Some(BuiltinMonadCarrier::Result),
        BuiltinExecutableCarrier::Task => Some(BuiltinMonadCarrier::Task),
        _ => None,
    }
}

fn builtin_foldable_carrier(carrier: BuiltinExecutableCarrier) -> Option<BuiltinFoldableCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinFoldableCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinFoldableCarrier::Option),
        BuiltinExecutableCarrier::Result => Some(BuiltinFoldableCarrier::Result),
        BuiltinExecutableCarrier::Validation => Some(BuiltinFoldableCarrier::Validation),
        _ => None,
    }
}

fn builtin_traversable_carrier(
    carrier: BuiltinExecutableCarrier,
) -> Option<BuiltinTraversableCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinTraversableCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinTraversableCarrier::Option),
        BuiltinExecutableCarrier::Result => Some(BuiltinTraversableCarrier::Result),
        BuiltinExecutableCarrier::Validation => Some(BuiltinTraversableCarrier::Validation),
        _ => None,
    }
}

fn builtin_filterable_carrier(
    carrier: BuiltinExecutableCarrier,
) -> Option<BuiltinFilterableCarrier> {
    match carrier {
        BuiltinExecutableCarrier::List => Some(BuiltinFilterableCarrier::List),
        BuiltinExecutableCarrier::Option => Some(BuiltinFilterableCarrier::Option),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{
        BuiltinExecutableCarrier, BuiltinExecutableClass, BuiltinExecutableClassSupport,
        builtin_executable_class_support, builtin_traverse_result_applicative_support,
        render_higher_kinded_builtin_support_markdown,
    };

    fn typeclasses_manual_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("manual")
            .join("guide")
            .join("typeclasses.md")
    }

    #[test]
    fn registry_keeps_signal_and_validation_non_monadic_without_reducing_task_support() {
        assert_eq!(
            builtin_executable_class_support(
                BuiltinExecutableClass::Monad,
                BuiltinExecutableCarrier::Signal,
            ),
            BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports join for List, Option, Result, and Task",
            )
        );
        assert_eq!(
            builtin_executable_class_support(
                BuiltinExecutableClass::Monad,
                BuiltinExecutableCarrier::Validation,
            ),
            BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports join for List, Option, Result, and Task",
            )
        );
        assert_eq!(
            builtin_executable_class_support(
                BuiltinExecutableClass::Functor,
                BuiltinExecutableCarrier::Task,
            ),
            BuiltinExecutableClassSupport::Supported
        );
        assert_eq!(
            builtin_executable_class_support(
                BuiltinExecutableClass::Apply,
                BuiltinExecutableCarrier::Task,
            ),
            BuiltinExecutableClassSupport::Supported
        );
        assert_eq!(
            builtin_executable_class_support(
                BuiltinExecutableClass::Chain,
                BuiltinExecutableCarrier::Task,
            ),
            BuiltinExecutableClassSupport::Supported
        );
        assert_eq!(
            builtin_executable_class_support(
                BuiltinExecutableClass::Monad,
                BuiltinExecutableCarrier::Task,
            ),
            BuiltinExecutableClassSupport::Supported
        );
        assert_eq!(
            builtin_traverse_result_applicative_support(BuiltinExecutableCarrier::Task),
            BuiltinExecutableClassSupport::Unsupported(
                "runtime lowering only supports traverse results in List, Option, Result, Validation, and Signal applicatives",
            )
        );
    }

    #[test]
    fn rendered_support_section_matches_the_manual() {
        let manual = fs::read_to_string(typeclasses_manual_path())
            .expect("typeclasses guide should be readable");
        let begin = "<!-- BEGIN builtin-executable-support -->";
        let end = "<!-- END builtin-executable-support -->";
        let start = manual
            .find(begin)
            .expect("typeclasses guide should mark the builtin support section")
            + begin.len();
        let finish = manual[start..]
            .find(end)
            .map(|offset| start + offset)
            .expect("typeclasses guide should close the builtin support section");
        let actual = manual[start..finish].trim();
        let expected = render_higher_kinded_builtin_support_markdown();
        assert_eq!(actual, expected.trim());
    }
}
