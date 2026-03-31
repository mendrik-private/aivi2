use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct TempFile {
    path: PathBuf,
}

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
        .join("frontend")
        .join(relative)
}

fn stdlib_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("stdlib")
        .join(relative)
}

impl TempFile {
    fn new(prefix: &str, contents: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "aivi-{prefix}-{}-{unique}.aivi",
            std::process::id()
        ));
        fs::write(&path, contents).expect("temporary input should be writable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn fmt_normalizes_reactive_update_items() {
    let input = TempFile::new(
        "fmt-reactive-update",
        concat!(
            "signal total:Signal Int\n",
            "signal ready:Signal Bool\n",
            "when   ready=>total<-signal1+signal2\n",
            "when ready and True=>total<-result{\n",
            "next<-Ok signal1\n",
            "next+signal2\n",
            "}\n",
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(
        output.status.success(),
        "fmt should succeed for reactive update items, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        concat!(
            "signal total : Signal Int\n",
            "signal ready : Signal Bool\n",
            "\n",
            "when ready => total <- signal1 + signal2\n",
            "when ready and True => total <-\n",
            "    result {\n",
            "        next <- Ok signal1\n",
            "        next + signal2\n",
            "    }\n",
        )
    );
}

#[test]
fn fmt_normalizes_pattern_armed_reactive_update_items() {
    let input = TempFile::new(
        "fmt-pattern-reactive-update",
        concat!(
            "type Direction=Up|Down\n",
            "type Event=Turn Direction|Tick\n",
            "signal heading:Signal Direction\n",
            "signal tickSeen:Signal Bool\n",
            "signal event=Turn Down\n",
            "when event\n",
            "  ||>Turn dir=>heading<-dir\n",
            "  ||>Tick=>tickSeen<-True\n",
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(
        output.status.success(),
        "fmt should succeed for pattern-armed reactive update items, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        concat!(
            "type Direction = Up | Down\n",
            "\n",
            "type Event =\n",
            "  | Turn Direction\n",
            "  | Tick\n",
            "\n",
            "signal heading : Signal Direction\n",
            "signal tickSeen : Signal Bool\n",
            "signal event = Turn Down\n",
            "\n",
            "when event\n",
            "  ||> Turn dir => heading <- dir\n",
            "  ||> Tick => tickSeen <- True\n",
        )
    );
}

#[test]
fn fmt_normalizes_markup_layout() {
    let input = TempFile::new(
        "fmt-normalize",
        "value dashboard=<fragment><Label text=\"Inbox\"/><show when={True} keepMounted={True}><with value={formatCount count} as={label}><Label text={label}/></with></show></fragment>\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(
        output.status.success(),
        "fmt should succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        concat!(
            "value dashboard =\n",
            "    <fragment>\n",
            "        <Label text=\"Inbox\" />\n",
            "        <show when={True} keepMounted={True}>\n",
            "            <with value={formatCount count} as={label}>\n",
            "                <Label text={label} />\n",
            "            </with>\n",
            "        </show>\n",
            "    </fragment>\n",
        )
    );
}

#[test]
fn fmt_fails_on_syntax_errors() {
    let input = TempFile::new("fmt-error", "value broken = <show when={True}>\n");

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(!output.status.success(), "fmt should fail on syntax errors");
    assert!(
        !output.stderr.is_empty(),
        "fmt should report diagnostics on stderr"
    );
}

#[test]
fn fmt_normalizes_grouped_exports() {
    let input = TempFile::new(
        "fmt-grouped-export",
        "type Greeting=Text\ntype Farewell=Text\nexport(Greeting,Farewell)\nexport (Greeting)\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg(input.path())
        .output()
        .expect("fmt command should run");

    assert!(
        output.status.success(),
        "fmt should succeed, stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        concat!(
            "type Greeting = Text\n",
            "\n",
            "type Farewell = Text\n",
            "\n",
            "export (Greeting, Farewell)\n",
            "export Greeting\n",
        )
    );
}

#[test]
fn fmt_check_accepts_stdlib_modules() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aivi"));
    command.arg("fmt").arg("--check");
    for relative in [
        "aivi/bundledsmokesupport.aivi",
        "aivi/bundledsmoketest.aivi",
        "aivi/color.aivi",
        "aivi/defaults.aivi",
        "aivi/duration.aivi",
        "aivi/list.aivi",
        "aivi/nonEmpty.aivi",
        "aivi/option.aivi",
        "aivi/order.aivi",
        "aivi/path.aivi",
        "aivi/prelude.aivi",
        "aivi/result.aivi",
        "aivi/text.aivi",
        "aivi/url.aivi",
        "aivi/validation.aivi",
    ] {
        command.arg(stdlib_path(relative));
    }

    let output = command.output().expect("fmt --check command should run");

    assert!(
        output.status.success(),
        "expected stdlib modules to already be formatted, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fmt_check_accepts_order_helper_surfaces() {
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg("--check")
        .arg(stdlib_path("aivi/order.aivi"))
        .arg(stdlib_path("aivi/prelude.aivi"))
        .arg(stdlib_path("tests/foundation-validation/main.aivi"))
        .arg(stdlib_path("tests/runtime-stdlib-validation/main.aivi"))
        .arg(fixture_path(
            "milestone-2/valid/bundled-root-prelude-stdlib/main.aivi",
        ))
        .output()
        .expect("fmt --check command should run");

    assert!(
        output.status.success(),
        "expected order helper surfaces to already be formatted, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fmt_check_accepts_list_pattern_fixtures() {
    let output = Command::new(env!("CARGO_BIN_EXE_aivi"))
        .arg("fmt")
        .arg("--check")
        .arg(fixture_path("milestone-2/valid/list-patterns/main.aivi"))
        .arg(fixture_path(
            "milestone-2/valid/markup-list-patterns/main.aivi",
        ))
        .output()
        .expect("fmt --check command should run");

    assert!(
        output.status.success(),
        "expected list pattern fixtures to already be formatted, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fmt_check_accepts_catalog_examples() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aivi"));
    command.arg("fmt").arg("--check");
    for relative in [
        "catalog/automata/automata_nfa_to_dfa/main.aivi",
        "catalog/dp/dp_edit_distance/main.aivi",
        "catalog/dp/dp_knapsack_01/main.aivi",
        "catalog/dp/dp_lis_patience/main.aivi",
        "catalog/foundation/surface_values_patterns/main.aivi",
        "catalog/foundation/surface_collections_pipes/main.aivi",
        "catalog/foundation/surface_workspace_imports/main.aivi",
        "catalog/foundation/surface_workspace_imports/shared/catalog.aivi",
        "catalog/graph/graph_bellman_ford/main.aivi",
        "catalog/graph/graph_dijkstra/main.aivi",
        "catalog/graph/graph_floyd_warshall/main.aivi",
        "catalog/graph/graph_tarjan_scc/main.aivi",
        "catalog/graph/graph_toposort_kahn/main.aivi",
        "catalog/graph/graph_union_find_kruskal/main.aivi",
        "catalog/heap/heap_priority_queue/main.aivi",
        "catalog/math/math_bigint/main.aivi",
        "catalog/math/math_fft/main.aivi",
        "catalog/math/math_matrix_lu/main.aivi",
        "catalog/math/math_mod_arith_ntt/main.aivi",
        "catalog/parsing/parser_pratt_expr/main.aivi",
        "catalog/parsing/parser_shunting_yard/main.aivi",
        "catalog/runtime/interpreter_stack_vm/main.aivi",
        "catalog/search/backtracking_nqueens_bitset/main.aivi",
        "catalog/search/backtracking_sudoku/main.aivi",
        "catalog/sorting/select_quickselect/main.aivi",
        "catalog/sorting/sort_introsort/main.aivi",
        "catalog/string/string_aho_corasick/main.aivi",
        "catalog/string/string_kmp/main.aivi",
        "catalog/string/string_suffix_array/main.aivi",
        "catalog/string/string_z_algorithm/main.aivi",
        "catalog/tree/tree_fenwick/main.aivi",
        "catalog/tree/tree_segment_tree_lazy/main.aivi",
    ] {
        command.arg(fixture_path(relative));
    }

    let output = command.output().expect("fmt --check command should run");

    assert!(
        output.status.success(),
        "expected catalog examples to already be formatted, stdout was: {}, stderr was: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
