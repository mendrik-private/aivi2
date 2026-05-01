use std::collections::BTreeSet;

use aivi_hir::{ItemId as HirItemId, SumConstructorHandle};

use super::{
    DetachedRuntimeValue, EvaluationError, RuntimeDbCommitPlan, RuntimeDbConnection,
    RuntimeDbQueryPlan, RuntimeDbStatement, RuntimeDbTaskPlan, RuntimeMap, RuntimeMapEntry,
    RuntimeRecordField, RuntimeSumValue, RuntimeValue, append_validation_errors, structural_eq,
};
use crate::{ItemId, KernelExprId, KernelId};

#[test]
fn display_formats_nested_runtime_values_without_intermediate_joining() {
    let value = RuntimeValue::Record(vec![
        RuntimeRecordField {
            label: "status".into(),
            value: RuntimeValue::OptionSome(Box::new(RuntimeValue::ResultOk(Box::new(
                RuntimeValue::Tuple(vec![RuntimeValue::Int(1), RuntimeValue::Text("ok".into())]),
            )))),
        },
        RuntimeRecordField {
            label: "metadata".into(),
            value: RuntimeValue::Map(RuntimeMap::from_entries(vec![RuntimeMapEntry {
                key: RuntimeValue::Text("attempts".into()),
                value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
            }])),
        },
    ]);

    assert_eq!(
        value.display_text(),
        "{status: Some Ok (1, ok), metadata: {attempts: [2, 3]}}"
    );
    assert_eq!(
        format!("{value}"),
        "{status: Some Ok (1, ok), metadata: {attempts: [2, 3]}}"
    );
}

#[test]
fn display_preserves_runtime_map_entry_order() {
    let value = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("zeta".into()),
            value: RuntimeValue::Int(1),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("alpha".into()),
            value: RuntimeValue::Int(2),
        },
    ]));

    assert_eq!(value.display_text(), "{zeta: 1, alpha: 2}");
    assert_eq!(format!("{value}"), "{zeta: 1, alpha: 2}");
}

#[test]
fn display_handles_deep_signal_nesting_without_recursion() {
    let mut value = RuntimeValue::Int(1);
    for _ in 0..10_000 {
        value = RuntimeValue::Signal(Box::new(value));
    }

    let rendered = format!("{value}");
    assert!(rendered.starts_with("Signal("));
    let suffix = "1".to_owned() + &")".repeat(10_000);
    assert!(rendered.ends_with(&suffix));
}

#[test]
fn display_formats_user_sum_values() {
    let value = RuntimeValue::Sum(RuntimeSumValue {
        item: HirItemId::from_raw(3),
        type_name: "ResultLike".into(),
        variant_name: "Pair".into(),
        fields: vec![RuntimeValue::Int(1), RuntimeValue::Text("ok".into())],
    });

    assert_eq!(value.display_text(), "Pair(1, ok)");
}

#[test]
fn display_formats_user_sum_constructors() {
    let value = RuntimeValue::Callable(super::RuntimeCallable::SumConstructor {
        handle: SumConstructorHandle {
            item: HirItemId::from_raw(3),
            type_name: "Status".into(),
            variant_name: "Ready".into(),
            field_count: 0,
        },
        bound_arguments: Vec::new(),
    });

    assert_eq!(format!("{value}"), "<constructor Status.Ready>");
}

#[test]
fn missing_item_body_display_includes_item_name() {
    let error = EvaluationError::MissingItemBody {
        item: ItemId::from_raw(7),
        name: "renderView".into(),
    };

    assert_eq!(
        format!("{error}"),
        "backend item 7 (`renderView`) has no lowered body kernel"
    );
}

#[test]
fn db_task_plan_display_formats_query_work() {
    let plan = RuntimeDbTaskPlan::Query(RuntimeDbQueryPlan {
        connection: RuntimeDbConnection {
            database: "/var/lib/app.sqlite".into(),
        },
        statement: RuntimeDbStatement {
            sql: "select * from users where id = ?".into(),
            arguments: vec![RuntimeValue::Int(7)],
        },
    });

    assert_eq!(
        format!("{plan}"),
        "db.query(db.connection(/var/lib/app.sqlite), sql(select * from users where id = ?; args: [7]))"
    );
    assert_eq!(
        format!("{plan:?}"),
        r#"Query(RuntimeDbQueryPlan { connection: RuntimeDbConnection { database: "/var/lib/app.sqlite" }, statement: RuntimeDbStatement { sql: "select * from users where id = ?", arguments: [Int(7)] } })"#
    );
}

#[test]
fn db_task_plan_display_formats_commit_work_deterministically() {
    let plan = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
        connection: RuntimeDbConnection {
            database: "/var/lib/app.sqlite".into(),
        },
        statements: vec![
            RuntimeDbStatement {
                sql: "insert into users(id, name) values (?, ?)".into(),
                arguments: vec![RuntimeValue::Int(7), RuntimeValue::Text("Ada".into())],
            },
            RuntimeDbStatement {
                sql: "insert into audit_log(message) values (?)".into(),
                arguments: vec![RuntimeValue::Text("created user".into())],
            },
        ],
        changed_tables: ["users", "audit_log"].into_iter().map(Into::into).collect(),
    });

    assert_eq!(
        format!("{plan}"),
        "db.commit(db.connection(/var/lib/app.sqlite), [sql(insert into users(id, name) values (?, ?); args: [7, Ada]), sql(insert into audit_log(message) values (?); args: [created user])]; changes: [audit_log, users])"
    );
}

#[test]
fn db_commit_plan_equality_normalizes_changed_table_order() {
    let left = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
        connection: RuntimeDbConnection {
            database: "/var/lib/app.sqlite".into(),
        },
        statements: vec![RuntimeDbStatement {
            sql: "update users set active = ? where id = ?".into(),
            arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
        }],
        changed_tables: ["users", "audit_log"].into_iter().map(Into::into).collect(),
    });
    let right = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
        connection: RuntimeDbConnection {
            database: "/var/lib/app.sqlite".into(),
        },
        statements: vec![RuntimeDbStatement {
            sql: "update users set active = ? where id = ?".into(),
            arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
        }],
        changed_tables: ["audit_log", "users"].into_iter().map(Into::into).collect(),
    });

    assert_eq!(left, right);
}

#[test]
fn db_commit_plan_equality_tracks_invalidation_and_statement_payload() {
    let base_connection = RuntimeDbConnection {
        database: "/var/lib/app.sqlite".into(),
    };
    let base_statement = RuntimeDbStatement {
        sql: "update users set active = ? where id = ?".into(),
        arguments: vec![RuntimeValue::Bool(true), RuntimeValue::Int(7)],
    };
    let left = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
        connection: base_connection.clone(),
        statements: vec![base_statement.clone()],
        changed_tables: BTreeSet::from(["users".into(), "audit_log".into()]),
    });
    let different_tables = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
        connection: base_connection.clone(),
        statements: vec![base_statement.clone()],
        changed_tables: BTreeSet::from(["users".into()]),
    });
    let different_statement = RuntimeDbTaskPlan::Commit(RuntimeDbCommitPlan {
        connection: base_connection,
        statements: vec![RuntimeDbStatement {
            sql: "update users set active = ? where id = ?".into(),
            arguments: vec![RuntimeValue::Bool(false), RuntimeValue::Int(7)],
        }],
        changed_tables: BTreeSet::from(["users".into(), "audit_log".into()]),
    });

    assert_ne!(left, different_tables);
    assert_ne!(left, different_statement);
}

#[test]
fn structural_equality_handles_bytes_maps_and_sets() {
    let kernel = KernelId::from_raw(0);
    let expr = KernelExprId::from_raw(0);

    assert!(
        structural_eq(
            kernel,
            expr,
            &RuntimeValue::Bytes([1, 2, 3].into()),
            &RuntimeValue::Bytes([1, 2, 3].into()),
        )
        .expect("bytes should compare structurally")
    );

    let left_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("left".into()),
            value: RuntimeValue::Int(1),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("right".into()),
            value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
        },
    ]));
    let right_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("right".into()),
            value: RuntimeValue::List(vec![RuntimeValue::Int(2), RuntimeValue::Int(3)]),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("left".into()),
            value: RuntimeValue::Int(1),
        },
    ]));
    assert!(
        structural_eq(kernel, expr, &left_map, &right_map)
            .expect("maps should compare structurally regardless of insertion order")
    );

    let left_set = RuntimeValue::Set(vec![RuntimeValue::Int(1), RuntimeValue::Int(2)]);
    let right_set = RuntimeValue::Set(vec![RuntimeValue::Int(2), RuntimeValue::Int(1)]);
    assert!(
        structural_eq(kernel, expr, &left_set, &right_set)
            .expect("sets should compare structurally regardless of insertion order")
    );
}

#[test]
fn validation_error_accumulation_appends_non_empty_payloads() {
    let left = RuntimeValue::Sum(RuntimeSumValue {
        item: HirItemId::from_raw(11),
        type_name: "NonEmptyList".into(),
        variant_name: "NonEmptyList".into(),
        fields: vec![
            RuntimeValue::Text("missing name".into()),
            RuntimeValue::List(Vec::new()),
        ],
    });
    let right = RuntimeValue::Sum(RuntimeSumValue {
        item: HirItemId::from_raw(11),
        type_name: "NonEmptyList".into(),
        variant_name: "NonEmptyList".into(),
        fields: vec![
            RuntimeValue::Text("missing email".into()),
            RuntimeValue::List(vec![RuntimeValue::Text("missing age".into())]),
        ],
    });

    let accumulated =
        append_validation_errors(left, right).expect("non-empty validation errors should append");

    assert_eq!(
        accumulated,
        RuntimeValue::Sum(RuntimeSumValue {
            item: HirItemId::from_raw(11),
            type_name: "NonEmptyList".into(),
            variant_name: "NonEmptyList".into(),
            fields: vec![
                RuntimeValue::Text("missing name".into()),
                RuntimeValue::List(vec![
                    RuntimeValue::Text("missing email".into()),
                    RuntimeValue::Text("missing age".into()),
                ]),
            ],
        })
    );
}

#[test]
fn detached_runtime_values_copy_text_storage_at_boundary() {
    let original = RuntimeValue::Signal(Box::new(RuntimeValue::Text("hello".into())));
    let detached = DetachedRuntimeValue::from_runtime_copy(&original);

    let RuntimeValue::Signal(original_inner) = &original else {
        panic!("expected wrapped signal value")
    };
    let RuntimeValue::Text(original_text) = original_inner.as_ref() else {
        panic!("expected wrapped text payload")
    };
    let RuntimeValue::Signal(detached_inner) = detached.as_runtime() else {
        panic!("expected detached wrapped signal value")
    };
    let RuntimeValue::Text(detached_text) = detached_inner.as_ref() else {
        panic!("expected detached wrapped text payload")
    };

    assert_eq!(detached, original);
    assert_ne!(
        original_text.as_ptr(),
        detached_text.as_ptr(),
        "detaching must copy boundary text storage instead of preserving addresses"
    );
}

#[test]
fn structural_equality_matches_bytes_maps_and_sets() {
    let kernel = KernelId::from_raw(0);
    let expr = KernelExprId::from_raw(0);

    assert!(
        structural_eq(
            kernel,
            expr,
            &RuntimeValue::Bytes(Box::from(*b"abc")),
            &RuntimeValue::Bytes(Box::from(*b"abc")),
        )
        .expect("bytes equality should be supported")
    );

    let left_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("first".into()),
            value: RuntimeValue::Int(1),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("second".into()),
            value: RuntimeValue::List(vec![RuntimeValue::Bool(true), RuntimeValue::Bool(false)]),
        },
    ]));
    let right_map = RuntimeValue::Map(RuntimeMap::from_entries(vec![
        RuntimeMapEntry {
            key: RuntimeValue::Text("second".into()),
            value: RuntimeValue::List(vec![RuntimeValue::Bool(true), RuntimeValue::Bool(false)]),
        },
        RuntimeMapEntry {
            key: RuntimeValue::Text("first".into()),
            value: RuntimeValue::Int(1),
        },
    ]));
    assert!(
        structural_eq(kernel, expr, &left_map, &right_map)
            .expect("map equality should be order-independent")
    );

    let left_set = RuntimeValue::Set(vec![
        RuntimeValue::Int(1),
        RuntimeValue::Text("two".into()),
        RuntimeValue::Bool(true),
    ]);
    let right_set = RuntimeValue::Set(vec![
        RuntimeValue::Bool(true),
        RuntimeValue::Int(1),
        RuntimeValue::Text("two".into()),
    ]);
    assert!(
        structural_eq(kernel, expr, &left_set, &right_set)
            .expect("set equality should be order-independent")
    );
}
