use std::{fs, path::PathBuf};

use aivi_base::SourceDatabase;
use aivi_gtk::{
    ChildOp, ChildUpdateMode, EventHookStrategy, PlanNodeKind, PropertyPlan, RepeatedChildPolicy,
    SetterSource, ShowMountPolicy, lower_markup_expr,
};
use aivi_hir::{ExprKind, Item, lower_module};
use aivi_syntax::parse_module;

fn lower_text(path: &str, text: &str) -> aivi_hir::LoweringResult {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse before HIR lowering: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    lower_module(&parsed.module)
}

fn find_value_item<'a>(module: &'a aivi_hir::Module, name: &str) -> &'a aivi_hir::ValueItem {
    module
        .root_items()
        .iter()
        .find_map(|item_id| match &module.items()[*item_id] {
            Item::Value(value) if value.name.text() == name => Some(value),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected to find value item `{name}`"))
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
}

fn child_id(op: ChildOp) -> aivi_gtk::PlanNodeId {
    op.child()
}

#[test]
fn lowers_schema_declared_event_attributes_as_direct_event_hooks() {
    let hir = lower_text(
        "event-attrs.aivi",
        r#"
value isVisible = True
value clickHandler = True
value view =
    <Button label="Save" visible={isVisible} onClick={clickHandler} />
"#,
    );
    assert!(
        !hir.has_errors(),
        "HIR lowering should succeed before GTK lowering: {:?}",
        hir.diagnostics()
    );

    let module = hir.module();
    let value = find_value_item(module, "view");
    let plan = lower_markup_expr(module, value.body).expect("markup should lower to a widget plan");

    let root = plan.node(plan.root()).expect("root node should exist");
    let PlanNodeKind::Widget(widget) = &root.kind else {
        panic!("expected root widget, found {:?}", root.kind.tag());
    };

    assert_eq!(widget.widget.to_string(), "Button");
    assert_eq!(widget.properties.len(), 2);
    assert_eq!(widget.event_hooks.len(), 1);

    assert!(matches!(
        &widget.properties[0],
        PropertyPlan::Static(static_prop)
            if static_prop.name.text() == "label"
    ));
    assert!(matches!(
        &widget.properties[1],
        PropertyPlan::Setter(setter)
            if setter.name.text() == "visible"
                && matches!(setter.source, SetterSource::Expr(_))
    ));
    assert!(matches!(
        &widget.event_hooks[0],
        aivi_gtk::EventHookPlan {
            hookup: EventHookStrategy::DirectSignal,
            ..
        }
    ));
    assert_eq!(widget.event_hooks[0].name.text(), "onClick");
}

#[test]
fn leaves_unsupported_widget_event_names_as_ordinary_attributes() {
    let hir = lower_text(
        "unsupported-widget-event.aivi",
        r#"
value clickHandler = True
value view =
    <Label onClick={clickHandler} />
"#,
    );
    assert!(
        !hir.has_errors(),
        "HIR lowering should succeed before GTK lowering: {:?}",
        hir.diagnostics()
    );

    let module = hir.module();
    let value = find_value_item(module, "view");
    let plan = lower_markup_expr(module, value.body).expect("markup should lower to a widget plan");

    let root = plan.node(plan.root()).expect("root node should exist");
    let PlanNodeKind::Widget(widget) = &root.kind else {
        panic!("expected root widget, found {:?}", root.kind.tag());
    };

    assert_eq!(widget.widget.to_string(), "Label");
    assert!(widget.event_hooks.is_empty());
    assert_eq!(widget.properties.len(), 1);
    assert!(matches!(
        &widget.properties[0],
        PropertyPlan::Setter(setter)
            if setter.name.text() == "onClick"
                && matches!(setter.source, SetterSource::Expr(_))
    ));
}

#[test]
fn lowers_expanded_catalog_widgets_with_entry_events_and_scrolled_children() {
    let hir = lower_text(
        "expanded-widget-catalog.aivi",
        r#"
value query = "Draft"
value canEdit = False
value submit = True
value view =
    <ScrolledWindow>
        <Entry text={query} placeholderText="Search" editable={canEdit} onActivate={submit} />
    </ScrolledWindow>
"#,
    );
    assert!(
        !hir.has_errors(),
        "HIR lowering should succeed before GTK lowering: {:?}",
        hir.diagnostics()
    );

    let module = hir.module();
    let value = find_value_item(module, "view");
    let plan = lower_markup_expr(module, value.body).expect("markup should lower to a widget plan");

    let root = plan.node(plan.root()).expect("root node should exist");
    let PlanNodeKind::Widget(scrolled_window) = &root.kind else {
        panic!("expected root widget, found {:?}", root.kind.tag());
    };
    assert_eq!(scrolled_window.widget.to_string(), "ScrolledWindow");
    assert_eq!(scrolled_window.children.len(), 1);

    let entry = plan
        .node(child_id(scrolled_window.children[0]))
        .expect("scrolled window child should exist");
    let PlanNodeKind::Widget(entry) = &entry.kind else {
        panic!("expected entry widget child, found {:?}", entry.kind.tag());
    };
    assert_eq!(entry.widget.to_string(), "Entry");
    assert_eq!(entry.properties.len(), 3);
    assert_eq!(entry.event_hooks.len(), 1);
    assert!(matches!(
        &entry.properties[0],
        PropertyPlan::Setter(setter)
            if setter.name.text() == "text"
                && matches!(setter.source, SetterSource::Expr(_))
    ));
    assert!(matches!(
        &entry.properties[1],
        PropertyPlan::Static(static_prop)
            if static_prop.name.text() == "placeholderText"
    ));
    assert!(matches!(
        &entry.properties[2],
        PropertyPlan::Setter(setter)
            if setter.name.text() == "editable"
                && matches!(setter.source, SetterSource::Expr(_))
    ));
    assert!(matches!(
        &entry.event_hooks[0],
        aivi_gtk::EventHookPlan {
            hookup: EventHookStrategy::DirectSignal,
            ..
        }
    ));
    assert_eq!(entry.event_hooks[0].name.text(), "onActivate");
}

#[test]
fn lowers_markup_control_fixture_into_explicit_control_nodes() {
    let fixture = fixture_root()
        .join("milestone-2")
        .join("valid")
        .join("markup-control-nodes")
        .join("main.aivi");
    let hir = lower_text(
        fixture.to_string_lossy().as_ref(),
        &fs::read_to_string(&fixture).expect("fixture should be readable"),
    );
    assert!(
        !hir.has_errors(),
        "fixture should lower into HIR cleanly: {:?}",
        hir.diagnostics()
    );

    let module = hir.module();
    let value = find_value_item(module, "screenView");
    let ExprKind::Markup(_) = module.exprs()[value.body].kind else {
        panic!("screenView should remain a markup expression");
    };
    let plan = lower_markup_expr(module, value.body).expect("fixture markup should lower");
    assert_eq!(
        plan.len(),
        14,
        "fixture should lower each explicit control/widget site once"
    );

    let root = plan.node(plan.root()).expect("root node should exist");
    let PlanNodeKind::Fragment(fragment) = &root.kind else {
        panic!("expected fragment root, found {:?}", root.kind.tag());
    };
    assert_eq!(fragment.children.len(), 2);

    let header = plan
        .node(child_id(fragment.children[0]))
        .expect("header child should exist");
    assert!(matches!(header.kind, PlanNodeKind::Widget(_)));

    let show = plan
        .node(child_id(fragment.children[1]))
        .expect("show child should exist");
    let PlanNodeKind::Show(show_node) = &show.kind else {
        panic!("expected show node, found {:?}", show.kind.tag());
    };
    assert!(matches!(
        show_node.mount,
        ShowMountPolicy::KeepMounted { .. }
    ));

    let with_node = plan
        .node(child_id(show_node.children[0]))
        .expect("with child should exist");
    let PlanNodeKind::With(with_node) = &with_node.kind else {
        panic!("expected with node, found {:?}", with_node.kind.tag());
    };

    let match_node = plan
        .node(child_id(with_node.children[0]))
        .expect("match child should exist");
    let PlanNodeKind::Match(match_node) = &match_node.kind else {
        panic!("expected match node, found {:?}", match_node.kind.tag());
    };
    assert_eq!(match_node.cases.len(), 3);

    let ready_case = plan
        .node(
            *match_node
                .cases
                .iter()
                .nth(1)
                .expect("ready case should exist"),
        )
        .expect("ready case node should exist");
    let PlanNodeKind::Case(ready_case) = &ready_case.kind else {
        panic!("expected case node, found {:?}", ready_case.kind.tag());
    };

    let each = plan
        .node(child_id(ready_case.children[0]))
        .expect("each child should exist");
    let PlanNodeKind::Each(each_node) = &each.kind else {
        panic!("expected each node, found {:?}", each.kind.tag());
    };
    assert!(matches!(
        each_node.child_policy,
        RepeatedChildPolicy::Keyed {
            updates: ChildUpdateMode::Localized,
            ..
        }
    ));
    let empty = each_node
        .empty_branch
        .map(|branch| plan.node(branch).expect("empty branch should exist"))
        .expect("each should include an explicit empty branch");
    assert!(matches!(empty.kind, PlanNodeKind::Empty(_)));

    let failed_case = plan
        .node(
            *match_node
                .cases
                .iter()
                .nth(2)
                .expect("failed case should exist"),
        )
        .expect("failed case node should exist");
    let PlanNodeKind::Case(failed_case) = &failed_case.kind else {
        panic!("expected case node, found {:?}", failed_case.kind.tag());
    };
    let failed_label = plan
        .node(child_id(failed_case.children[0]))
        .expect("failed label should exist");
    let PlanNodeKind::Widget(failed_label) = &failed_label.kind else {
        panic!(
            "expected failed label widget, found {:?}",
            failed_label.kind.tag()
        );
    };
    assert!(matches!(
        &failed_label.properties[0],
        PropertyPlan::Setter(setter)
            if setter.name.text() == "text"
                && matches!(setter.source, SetterSource::InterpolatedText(_))
    ));
}
