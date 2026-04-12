#[test]
fn runtime_evaluates_item_bodies_and_source_kernels() {
    let backend = lower_text(
        "backend-runtime-values.aivi",
        r#"
signal apiHost = "https://api.example.com"
signal refresh = 0
signal enabled = True
signal pollInterval = 5

fun addOne:Int = n:Int=>    n + 1

value answer =
    addOne 41

@source http.get "{apiHost}/users" with {
    refreshOn: refresh,
    activeWhen: enabled,
    refreshEvery: addOne pollInterval
}
signal users : Signal Int
"#,
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::from([
        (
            find_item(&backend, "apiHost"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Text(
                "https://api.example.com".into(),
            ))),
        ),
        (
            find_item(&backend, "refresh"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Int(0))),
        ),
        (
            find_item(&backend, "enabled"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true))),
        ),
        (
            find_item(&backend, "pollInterval"),
            RuntimeValue::Signal(Box::new(RuntimeValue::Int(5))),
        ),
    ]);

    let answer = find_item(&backend, "answer");
    assert_eq!(
        evaluator
            .evaluate_item(answer, &globals)
            .expect("item body should evaluate"),
        RuntimeValue::Int(42)
    );

    let users = find_item(&backend, "users");
    let BackendItemKind::Signal(signal) = &backend.items()[users].kind else {
        panic!("users should remain a signal item");
    };
    let source = &backend.sources()[signal.source.expect("source should exist")];

    assert_eq!(
        evaluator
            .evaluate_kernel(source.arguments[0].kernel, None, &[], &globals)
            .expect("source argument kernel should evaluate"),
        RuntimeValue::Text("https://api.example.com/users".into())
    );

    let active_when = source
        .options
        .iter()
        .find(|option| option.option_name.as_ref() == "activeWhen")
        .expect("activeWhen option should exist");
    assert_eq!(
        evaluator
            .evaluate_kernel(active_when.kernel, None, &[], &globals)
            .expect("activeWhen option should evaluate"),
        RuntimeValue::Signal(Box::new(RuntimeValue::Bool(true)))
    );

    let refresh_every = source
        .options
        .iter()
        .find(|option| option.option_name.as_ref() == "refreshEvery")
        .expect("refreshEvery option should exist");
    assert_eq!(
        evaluator
            .evaluate_kernel(refresh_every.kernel, None, &[], &globals)
            .expect("refreshEvery option should evaluate"),
        RuntimeValue::Int(6)
    );
}

#[test]
fn runtime_evaluates_builtin_overloaded_class_members() {
    let backend = lower_text(
        "backend-builtin-class-members.aivi",
        r#"
fun increment:Int = n:Int=> n + 1
value joined:Text =
    append "hel" "lo"

value lifted:Option Int =
    map increment (Some 1)

value singleton:Option Int =
    pure 3

value readyTask:Task Text Int =
    pure 3

value none:List Int =
    empty
"#,
    );

    let joined = find_item(&backend, "joined");
    let lifted = find_item(&backend, "lifted");
    let singleton = find_item(&backend, "singleton");
    let ready_task = find_item(&backend, "readyTask");
    let none = find_item(&backend, "none");
    let none_kernel_id = backend.items()[none]
        .body
        .expect("none should carry a body");

    let joined_kernel = backend.kernels()[backend.items()[joined]
        .body
        .expect("joined should carry a body")]
    .clone();
    match &joined_kernel.exprs()[joined_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &joined_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Append(
                    BuiltinAppendCarrier::Text
                )))
            ));
        }
        other => panic!("expected append body to lower into an apply tree, found {other:?}"),
    }

    let lifted_kernel = backend.kernels()[backend.items()[lifted]
        .body
        .expect("lifted should carry a body")]
    .clone();
    match &lifted_kernel.exprs()[lifted_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &lifted_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Map(
                    BuiltinFunctorCarrier::Option
                )))
            ));
        }
        other => panic!("expected map body to lower into an apply tree, found {other:?}"),
    }

    let singleton_kernel = backend.kernels()[backend.items()[singleton]
        .body
        .expect("singleton should carry a body")]
    .clone();
    match &singleton_kernel.exprs()[singleton_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &singleton_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Pure(
                    BuiltinApplicativeCarrier::Option
                )))
            ));
        }
        other => panic!("expected pure body to lower into an apply tree, found {other:?}"),
    }

    let ready_task_kernel = backend.kernels()[backend.items()[ready_task]
        .body
        .expect("readyTask should carry a body")]
    .clone();
    match &ready_task_kernel.exprs()[ready_task_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &ready_task_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Pure(
                    BuiltinApplicativeCarrier::Task
                )))
            ));
        }
        other => panic!("expected task pure body to lower into an apply tree, found {other:?}"),
    }

    assert!(matches!(
        &backend.kernels()[none_kernel_id].exprs()[backend.kernels()[none_kernel_id].root].kind,
        KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Empty(
            BuiltinAppendCarrier::List
        )))
    ));

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(joined, &globals)
            .expect("append should evaluate"),
        RuntimeValue::Text("hello".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(lifted, &globals)
            .expect("map should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(singleton, &globals)
            .expect("pure should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(3)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(ready_task, &globals)
            .expect("task pure should evaluate"),
        RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(RuntimeValue::Int(3)),
        })
    );
    assert_eq!(
        evaluator
            .evaluate_item(none, &globals)
            .expect("empty should evaluate"),
        RuntimeValue::List(Vec::new())
    );
}

#[test]
fn runtime_evaluates_builtin_monad_members() {
    let backend = lower_text(
        "backend-builtin-monad-members.aivi",
        r#"
fun nextOption:(Option Int) = n:Int=>    Some (n + 1)

fun nextResult:(Result Text Int) = n:Int=>    Ok (n + 2)

value okSeed:Result Text Int =
    Ok 4

value chainedOption:Option Int =
    chain nextOption (Some 2)

value joinedOption:Option Int =
    join (Some (Some 3))

value chainedResult:Result Text Int =
    chain nextResult okSeed

value joinedList:List Int =
    join [[1, 2], [3]]
"#,
    );

    let chained_option = find_item(&backend, "chainedOption");
    let joined_option = find_item(&backend, "joinedOption");
    let chained_result = find_item(&backend, "chainedResult");
    let joined_list = find_item(&backend, "joinedList");

    let chained_option_kernel = backend.kernels()[backend.items()[chained_option]
        .body
        .expect("chainedOption should carry a body")]
    .clone();
    match &chained_option_kernel.exprs()[chained_option_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &chained_option_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Chain(
                    BuiltinMonadCarrier::Option
                )))
            ));
        }
        other => panic!("expected chain body to lower into an apply tree, found {other:?}"),
    }

    let joined_option_kernel = backend.kernels()[backend.items()[joined_option]
        .body
        .expect("joinedOption should carry a body")]
    .clone();
    match &joined_option_kernel.exprs()[joined_option_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                &joined_option_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Join(
                    BuiltinMonadCarrier::Option
                )))
            ));
        }
        other => panic!("expected join body to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(chained_option, &globals)
            .expect("option chain should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(3)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(joined_option, &globals)
            .expect("option join should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(3)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(chained_result, &globals)
            .expect("result chain should evaluate"),
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(6)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(joined_list, &globals)
            .expect("list join should evaluate"),
        RuntimeValue::List(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ])
    );
}

#[test]
fn runtime_evaluates_task_functor_apply_and_monad_members() {
    let backend = lower_text(
        "backend-task-class-members.aivi",
        r#"
fun addOne:Int = n:Int=>    n + 1

fun liftTask:(Task Text Int) = n:Int=>    pure (n + 2)

value oneTask:Task Text Int =
    pure 1

value addOneTask:Task Text (Int -> Int) =
    pure addOne

value fourTask:Task Text Int =
    pure 4

value nestedTask:Task Text (Task Text Int) =
    pure fourTask

value mappedTask:Task Text Int =
    map addOne oneTask

value appliedTask:Task Text Int =
    apply addOneTask oneTask

value chainedTask:Task Text Int =
    chain liftTask fourTask

value joinedTask:Task Text Int =
    join nestedTask
"#,
    );

    for (name, expected_intrinsic) in [
        (
            "mappedTask",
            BuiltinClassMemberIntrinsic::Map(BuiltinFunctorCarrier::Task),
        ),
        (
            "appliedTask",
            BuiltinClassMemberIntrinsic::Apply(BuiltinApplyCarrier::Task),
        ),
        (
            "chainedTask",
            BuiltinClassMemberIntrinsic::Chain(BuiltinMonadCarrier::Task),
        ),
        (
            "joinedTask",
            BuiltinClassMemberIntrinsic::Join(BuiltinMonadCarrier::Task),
        ),
    ] {
        let item = find_item(&backend, name);
        let kernel = backend.kernels()[backend.items()[item]
            .body
            .unwrap_or_else(|| panic!("{name} should carry a body"))]
        .clone();
        let KernelExprKind::Apply { callee, .. } = &kernel.exprs()[kernel.root].kind else {
            panic!("{name} should lower to an apply tree");
        };
        assert!(matches!(
            &kernel.exprs()[*callee].kind,
            KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(intrinsic))
                if *intrinsic == expected_intrinsic
        ));
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedTask"), &globals)
            .expect("task map should evaluate"),
        RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(RuntimeValue::Int(2)),
        })
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "appliedTask"), &globals)
            .expect("task apply should evaluate"),
        RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(RuntimeValue::Int(2)),
        })
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "chainedTask"), &globals)
            .expect("task chain should evaluate"),
        RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(RuntimeValue::Int(6)),
        })
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "joinedTask"), &globals)
            .expect("task join should evaluate"),
        RuntimeValue::Task(RuntimeTaskPlan::Pure {
            value: Box::new(RuntimeValue::Int(4)),
        })
    );
}

#[test]
fn runtime_evaluates_applicative_clusters() {
    let backend = lower_text(
        "backend-applicative-clusters.aivi",
        r#"
fun add:Int = left:Int right:Int=>    left + right

value combinedOption:Option Int =
 &|> Some 2
 &|> Some 3
  |> add

value tupledOption:Option (Int, Int) =
 &|> Some 2
 &|> Some 3
"#,
    );

    let combined_option = find_item(&backend, "combinedOption");
    let tupled_option = find_item(&backend, "tupledOption");

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(combined_option, &globals)
            .expect("cluster finalizer should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(5)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(tupled_option, &globals)
            .expect("implicit tuple cluster should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Tuple(vec![
            RuntimeValue::Int(2),
            RuntimeValue::Int(3),
        ])))
    );
}

#[test]
fn runtime_evaluates_builtin_foldable_reduce_members() {
    let backend = lower_text(
        "backend-foldable-reduce.aivi",
        r#"
fun add:Int = acc:Int item:Int=>    acc + item

fun joinStep:Text = acc:Text item:Text=>    append acc item

value maybeInput:Option Int =
    Some 4

value noneInput:Option Int =
    None

value okInput:Result Text Int =
    Ok 5

value errInput:Result Text Int =
    Err "bad"

value validInput:Validation Text Int =
    Valid 6

value invalidInput:Validation Text Int =
    Invalid "missing"

value total:Int =
    reduce add 10 [1, 2, 3]

value joined:Text =
    reduce joinStep "" ["hel", "lo"]

value maybeTotal:Int =
    reduce add 10 maybeInput

value noneTotal:Int =
    reduce add 10 noneInput

value okTotal:Int =
    reduce add 10 okInput

value errTotal:Int =
    reduce add 10 errInput

value validTotal:Int =
    reduce add 10 validInput

value invalidTotal:Int =
    reduce add 10 invalidInput
"#,
    );

    let total = find_item(&backend, "total");
    let joined = find_item(&backend, "joined");
    let maybe_total = find_item(&backend, "maybeTotal");
    let none_total = find_item(&backend, "noneTotal");
    let ok_total = find_item(&backend, "okTotal");
    let err_total = find_item(&backend, "errTotal");
    let valid_total = find_item(&backend, "validTotal");
    let invalid_total = find_item(&backend, "invalidTotal");

    let total_kernel = backend.kernels()[backend.items()[total]
        .body
        .expect("total should carry a body")]
    .clone();
    match &total_kernel.exprs()[total_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 3);
            assert!(matches!(
                &total_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Reduce(
                    BuiltinFoldableCarrier::List
                )))
            ));
        }
        other => panic!("expected reduce body to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(total, &globals)
            .expect("list reduce should evaluate"),
        RuntimeValue::Int(16)
    );
    assert_eq!(
        evaluator
            .evaluate_item(joined, &globals)
            .expect("text reduce should evaluate"),
        RuntimeValue::Text("hello".into())
    );
    assert_eq!(
        evaluator
            .evaluate_item(maybe_total, &globals)
            .expect("option reduce should evaluate"),
        RuntimeValue::Int(14)
    );
    assert_eq!(
        evaluator
            .evaluate_item(none_total, &globals)
            .expect("empty option reduce should evaluate"),
        RuntimeValue::Int(10)
    );
    assert_eq!(
        evaluator
            .evaluate_item(ok_total, &globals)
            .expect("result reduce should evaluate"),
        RuntimeValue::Int(15)
    );
    assert_eq!(
        evaluator
            .evaluate_item(err_total, &globals)
            .expect("error result reduce should evaluate"),
        RuntimeValue::Int(10)
    );
    assert_eq!(
        evaluator
            .evaluate_item(valid_total, &globals)
            .expect("validation reduce should evaluate"),
        RuntimeValue::Int(16)
    );
    assert_eq!(
        evaluator
            .evaluate_item(invalid_total, &globals)
            .expect("invalid validation reduce should evaluate"),
        RuntimeValue::Int(10)
    );
}

#[test]
fn runtime_evaluates_extended_builtin_typeclass_members() {
    let backend = lower_text(
        "backend-extended-typeclass-members.aivi",
        r#"
fun addOne:Int = n:Int=>    n + 1

fun keepSmall:(Option Int) = n:Int=>    n < 3
     T|> Some n
     F|> None

fun punctuate:Text = s:Text=>    append s "!"

value okOne:Result Text Int =
    Ok 1

value errBad:Result Text Int =
    Err "bad"

value validOne:Validation Text Int =
    Valid 1

value invalidNo:Validation Text Int =
    Invalid "no"

value orderedFloat:Ordering =
    compare 1.0 2.0

value orderedBig:Ordering =
    compare 1n 2n

value mappedOk:Result Text Int =
    bimap punctuate addOne okOne

value mappedErr:Result Text Int =
    bimap punctuate addOne errBad

value mappedValid:Validation Text Int =
    bimap punctuate addOne validOne

value mappedInvalid:Validation Text Int =
    bimap punctuate addOne invalidNo

value traversedList:Option (List Int) =
    traverse keepSmall [1, 2]

value traversedSome:Option (Option Int) =
    traverse keepSmall (Some 1)

value traversedOk:Option (Result Text Int) =
    traverse keepSmall okOne

value traversedValid:Option (Validation Text Int) =
    traverse keepSmall validOne

value filteredList:List Int =
    filterMap keepSmall [1, 3, 2]

value filteredSome:Option Int =
    filterMap keepSmall (Some 2)

value filteredMissing:Option Int =
    filterMap keepSmall (Some 4)
"#,
    );

    let ordered_float = find_item(&backend, "orderedFloat");
    let mapped_ok = find_item(&backend, "mappedOk");
    let traversed_list = find_item(&backend, "traversedList");
    let filtered_list = find_item(&backend, "filteredList");

    let ordered_float_kernel = backend.kernels()[backend.items()[ordered_float]
        .body
        .expect("orderedFloat should carry a body")]
    .clone();
    match &ordered_float_kernel.exprs()[ordered_float_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &ordered_float_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Compare {
                    subject: BuiltinOrdSubject::Float,
                    ..
                })
            )));
        }
        other => panic!("expected compare body to lower into an apply tree, found {other:?}"),
    }

    let mapped_ok_kernel = backend.kernels()[backend.items()[mapped_ok]
        .body
        .expect("mappedOk should carry a body")]
    .clone();
    match &mapped_ok_kernel.exprs()[mapped_ok_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 3);
            assert!(matches!(
                &mapped_ok_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Bimap(
                    BuiltinBifunctorCarrier::Result
                )))
            ));
        }
        other => panic!("expected bimap body to lower into an apply tree, found {other:?}"),
    }

    let traversed_list_kernel = backend.kernels()[backend.items()[traversed_list]
        .body
        .expect("traversedList should carry a body")]
    .clone();
    match &traversed_list_kernel.exprs()[traversed_list_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &traversed_list_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Traverse {
                    traversable: BuiltinTraversableCarrier::List,
                    applicative: BuiltinApplicativeCarrier::Option
                })
            )));
        }
        other => panic!("expected traverse body to lower into an apply tree, found {other:?}"),
    }

    let filtered_list_kernel = backend.kernels()[backend.items()[filtered_list]
        .body
        .expect("filteredList should carry a body")]
    .clone();
    match &filtered_list_kernel.exprs()[filtered_list_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            assert!(matches!(
                &filtered_list_kernel.exprs()[*callee].kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::FilterMap(
                    BuiltinFilterableCarrier::List
                )))
            ));
        }
        other => panic!("expected filterMap body to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    match evaluator
        .evaluate_item(ordered_float, &globals)
        .expect("float compare should evaluate")
    {
        RuntimeValue::Sum(value) => {
            assert_eq!(value.type_name.as_ref(), "Ordering");
            assert_eq!(value.variant_name.as_ref(), "Less");
        }
        other => panic!("expected Ordering result, found {other:?}"),
    }
    match evaluator
        .evaluate_item(find_item(&backend, "orderedBig"), &globals)
        .expect("bigint compare should evaluate")
    {
        RuntimeValue::Sum(value) => {
            assert_eq!(value.type_name.as_ref(), "Ordering");
            assert_eq!(value.variant_name.as_ref(), "Less");
        }
        other => panic!("expected Ordering result, found {other:?}"),
    }
    assert_eq!(
        evaluator
            .evaluate_item(mapped_ok, &globals)
            .expect("result bimap should evaluate"),
        RuntimeValue::ResultOk(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedErr"), &globals)
            .expect("err bimap should evaluate"),
        RuntimeValue::ResultErr(Box::new(RuntimeValue::Text("bad!".into())))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedValid"), &globals)
            .expect("validation bimap should evaluate"),
        RuntimeValue::ValidationValid(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "mappedInvalid"), &globals)
            .expect("invalid bimap should evaluate"),
        RuntimeValue::ValidationInvalid(Box::new(RuntimeValue::Text("no!".into())))
    );
    assert_eq!(
        evaluator
            .evaluate_item(traversed_list, &globals)
            .expect("list traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::List(vec![
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
        ])))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "traversedSome"), &globals)
            .expect("option traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::OptionSome(Box::new(
            RuntimeValue::Int(1)
        ))))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "traversedOk"), &globals)
            .expect("result traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::ResultOk(Box::new(
            RuntimeValue::Int(1)
        ))))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "traversedValid"), &globals)
            .expect("validation traverse should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::ValidationValid(Box::new(
            RuntimeValue::Int(1)
        ))))
    );
    assert_eq!(
        evaluator
            .evaluate_item(filtered_list, &globals)
            .expect("list filterMap should evaluate"),
        RuntimeValue::List(vec![RuntimeValue::Int(1), RuntimeValue::Int(2)])
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "filteredSome"), &globals)
            .expect("option filterMap should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
    assert_eq!(
        evaluator
            .evaluate_item(find_item(&backend, "filteredMissing"), &globals)
            .expect("missing option filterMap should evaluate"),
        RuntimeValue::OptionNone
    );
}

#[test]
fn runtime_evaluates_validation_apply_through_backend_runtime() {
    let backend = lower_text(
        "backend-validation-apply.aivi",
        r#"
type Pair = Pair Text Text

fun pair:Pair = left:Text right:Text=>    Pair left right

value first:Validation Text Text =
    Valid "Ada"

value second:Validation Text Text =
    Valid "Lovelace"

value combined:Validation Text Pair =
    apply (map pair first) second
"#,
    );

    let combined = find_item(&backend, "combined");
    let combined_kernel = &backend.kernels()[backend.items()[combined]
        .body
        .expect("combined should carry a body")];
    assert!(
        combined_kernel.exprs().iter().any(|(_, expr)| {
            matches!(
                expr.kind,
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Apply(
                    BuiltinApplyCarrier::Validation
                )))
            )
        }),
        "expected validation applicative clusters to lower through builtin Apply"
    );

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    match evaluator
        .evaluate_item(combined, &globals)
        .expect("validation apply should evaluate")
    {
        RuntimeValue::ValidationValid(value) => match *value {
            RuntimeValue::Sum(value) => {
                assert_eq!(value.type_name.as_ref(), "Pair");
                assert_eq!(value.variant_name.as_ref(), "Pair");
                assert_eq!(
                    value.fields,
                    vec![
                        RuntimeValue::Text("Ada".into()),
                        RuntimeValue::Text("Lovelace".into()),
                    ]
                );
            }
            other => panic!("expected Pair payload, found {other:?}"),
        },
        other => panic!("expected valid validation result, found {other:?}"),
    }
}

#[test]
fn workspace_imported_builtin_class_members_lower_through_backend_runtime() {
    let backend = lower_workspace_text(
        "milestone-2/valid/workspace-type-imports/main.aivi",
        r#"
use shared.types (
    Envelope
)

fun increment:Int = n:Int=>    n + 1

value lifted:Envelope (Option Int) =
    map increment (Some 1)
"#,
    );

    let lifted = find_item(&backend, "lifted");
    let lifted_kernel_id = backend.items()[lifted]
        .body
        .expect("workspace lifted should lower a backend body kernel");
    let lifted_kernel = &backend.kernels()[lifted_kernel_id];
    match &lifted_kernel.exprs()[lifted_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            match &lifted_kernel.exprs()[*callee].kind {
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Builtin(BuiltinClassMemberIntrinsic::Map(
                    BuiltinFunctorCarrier::Option,
                ))) => {}
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Authored(item)) => {
                    assert_eq!(backend.items()[*item].name.as_ref(), "__aivi_option_map");
                }
                KernelExprKind::Item(item) => {
                    assert_eq!(backend.items()[*item].name.as_ref(), "__aivi_option_map");
                }
                other => {
                    panic!("expected workspace map call to lower through builtin evidence, authored evidence, or the ambient map item, found {other:?}");
                }
            }
        }
        other => panic!("expected workspace map call to lower into an apply tree, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    assert_eq!(
        evaluator
            .evaluate_item(lifted, &globals)
            .expect("workspace map call should evaluate"),
        RuntimeValue::OptionSome(Box::new(RuntimeValue::Int(2)))
    );
}

#[test]
fn same_module_class_member_calls_lower_through_backend_runtime() {
    let backend = lower_text(
        "backend-same-module-class-member.aivi",
        r#"
class Semigroup A
    append : A -> A -> A

type Blob = Blob Int

instance Semigroup Blob
    append left right =
        left

fun combine:Blob = left:Blob right:Blob=>    append left right

value combined:Blob =
    combine (Blob 1) (Blob 2)
"#,
    );

    let combine = find_item(&backend, "combine");
    let combine_kernel_id = backend.items()[combine]
        .body
        .expect("combine should lower to a backend kernel");
    let combine_kernel = &backend.kernels()[combine_kernel_id];
    match &combine_kernel.exprs()[combine_kernel.root].kind {
        KernelExprKind::Apply { callee, arguments } => {
            assert_eq!(arguments.len(), 2);
            match &combine_kernel.exprs()[*callee].kind {
                KernelExprKind::ExecutableEvidence(ExecutableEvidence::Authored(item)) => {
                    assert!(backend.items()[*item].name.starts_with("instance#"));
                }
                other => panic!(
                    "expected same-module class member to lower through authored executable evidence, found {other:?}"
                ),
            }
        }
        other => panic!("expected combine to lower to an apply expression, found {other:?}"),
    }

    let mut evaluator = KernelEvaluator::new(&backend);
    let globals = BTreeMap::new();
    match evaluator
        .evaluate_item(find_item(&backend, "combined"), &globals)
        .expect("same-module class member should evaluate")
    {
        RuntimeValue::Sum(value) => {
            assert_eq!(value.type_name.as_ref(), "Blob");
            assert_eq!(value.variant_name.as_ref(), "Blob");
            assert_eq!(value.fields, vec![RuntimeValue::Int(1)]);
        }
        other => panic!("expected Blob sum result, found {other:?}"),
    }
}
