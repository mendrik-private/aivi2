use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use aivi_backend::{DetachedRuntimeValue, RuntimeSumValue, RuntimeValue};
use aivi_base::SourceDatabase;
use aivi_hir::{Item, lower_module as lower_hir_module};
use aivi_lambda::lower_module as lower_lambda_module;
use aivi_runtime::{
    SourceProviderManager, assemble_hir_runtime, link_backend_runtime, providers::WindowKeyEvent,
};
use aivi_syntax::parse_module;

struct LoweredStack {
    hir: aivi_hir::LoweringResult,
    core: aivi_core::Module,
    backend: aivi_backend::Program,
}

fn lower_text(path: &str, text: &str) -> LoweredStack {
    let mut sources = SourceDatabase::new();
    let file_id = sources.add_file(path, text);
    let parsed = parse_module(&sources[file_id]);
    assert!(
        !parsed.has_errors(),
        "fixture {path} should parse: {:?}",
        parsed.all_diagnostics().collect::<Vec<_>>()
    );
    let hir = lower_hir_module(&parsed.module);
    assert!(
        !hir.has_errors(),
        "fixture {path} should lower to HIR: {:?}",
        hir.diagnostics()
    );
    let core = aivi_core::lower_module(hir.module()).expect("typed-core lowering should succeed");
    let lambda = lower_lambda_module(&core).expect("lambda lowering should succeed");
    let backend = aivi_backend::lower_module(&lambda).expect("backend lowering should succeed");
    LoweredStack { hir, core, backend }
}

fn item_id(module: &aivi_hir::Module, name: &str) -> aivi_hir::ItemId {
    module
        .items()
        .iter()
        .find_map(|(item_id, item)| match item {
            Item::Value(item) if item.name.text() == name => Some(item_id),
            Item::Function(item) if item.name.text() == name => Some(item_id),
            Item::Signal(item) if item.name.text() == name => Some(item_id),
            Item::Type(item) if item.name.text() == name => Some(item_id),
            Item::Class(item) if item.name.text() == name => Some(item_id),
            Item::Domain(item) if item.name.text() == name => Some(item_id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected item named {name}"))
}

fn spin_until(
    linked: &mut aivi_runtime::BackendLinkedRuntime,
    signal: aivi_runtime::SignalHandle,
    timeout: Duration,
) -> Option<RuntimeValue> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        linked.tick().expect("runtime tick should succeed");
        if let Some(value) = linked.runtime().current_value(signal).unwrap() {
            return Some(value.clone());
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

#[test]
fn window_key_scan_updates_direction_signal() {
    let lowered = lower_text(
        "runtime-window-key-direction.aivi",
        r#"
type Key =
  | Key Text

type Direction =
  | Up
  | Down
  | Left
  | Right

value arrowKey:(Option Direction) key:Key =>
    key
     ||> Key "ArrowUp"    => Some Up
     ||> Key "ArrowDown"  => Some Down
     ||> Key "ArrowLeft"  => Some Left
     ||> Key "ArrowRight" => Some Right
     ||> _                => None

value filterDirection:Direction current:Direction opt:(Option Direction) =>
    opt
     ||> Some dir => dir
     ||> None     => current

value updateDirection:Direction key:Key current:Direction =>
    arrowKey key
     |> filterDirection current

@source window.keyDown with {
    repeat: False
    focusOnly: True
}
signal keyDown : Signal Key

signal direction : Signal Direction =
    keyDown
     |> scan Right updateDirection
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(assembly, &lowered.core, Arc::new(lowered.backend))
        .expect("startup link should succeed");
    let actions = linked
        .tick_with_source_lifecycle()
        .expect("linked runtime tick should succeed");
    let mut providers = SourceProviderManager::new();
    providers
        .apply_actions(actions.source_actions())
        .expect("window key source should execute");
    providers.dispatch_window_key_event(WindowKeyEvent {
        name: "ArrowDown".into(),
        repeated: false,
    });

    let direction_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "direction"))
        .expect("direction signal binding should exist")
        .signal();
    let constructor = lowered
        .hir
        .module()
        .sum_constructor_handle(item_id(lowered.hir.module(), "Direction"), "Down")
        .expect("Direction.Down constructor should resolve");
    let value = spin_until(&mut linked, direction_signal, Duration::from_millis(200))
        .expect("window key source should update direction");
    assert_eq!(
        value,
        RuntimeValue::Sum(RuntimeSumValue {
            item: constructor.item,
            type_name: constructor.type_name.clone(),
            variant_name: constructor.variant_name.clone(),
            fields: Vec::new(),
        })
    );
}

#[test]
fn window_key_space_restart_is_consumed_after_one_game_over() {
    let lowered = lower_text(
        "runtime-window-key-space-restart.aivi",
        r#"
use aivi.defaults (Option)

type Key =
  | Key Text

type Direction =
  | Left
  | Right

type Status =
  | Running
  | GameOver

type Game = {
    status: Status,
    steps: Int
}

type GameTickState = {
    game: Game,
    seenRestartCount: Int
}

value initialGame:Game = {
    status: Running,
    steps: 0
}

value initialGameTickState:GameTickState = {
    game: initialGame,
    seenRestartCount: 0
}

value arrowKey:(Option Direction) key:Key =>
    key
     ||> Key "ArrowLeft"  => Some Left
     ||> Key "ArrowRight" => Some Right
     ||> _                => None

value filterDirection:Direction current:Direction opt:(Option Direction) =>
    opt
     ||> Some dir => dir
     ||> None     => current

value updateDirection:Direction key:Key current:Direction =>
    arrowKey key
     |> filterDirection current

value restartKey:Bool key:Key =>
    key
     ||> Key "Space" => True
     ||> _           => False

value updateDirectionOrRestart:Direction key:Key current:Direction =>
    restartKey key
     T|> Right
     F|> updateDirection key current

value updateRestartCount:Int key:Key current:Int =>
    restartKey key
     T|> current + 1
     F|> current

value hasPendingRestart:Bool restartCount:Int seenRestartCount:Int =>
    restartCount != seenRestartCount

value stepRunning:Game direction:Direction game:Game =>
    direction
     ||> Left  => { status: GameOver, steps: game.steps + 1 }
     ||> Right => { status: Running, steps: game.steps + 1 }

value restartGame:Game restart:Bool game:Game =>
    restart
     T|> initialGame
     F|> game

value stepGame:Game restart:Bool direction:Direction game:Game =>
    game.status
     ||> GameOver => restartGame restart game
     ||> Running  => stepRunning direction game

value stepTickState:GameTickState restartCount:Int direction:Direction state:GameTickState =>
    {
        game: stepGame (hasPendingRestart restartCount state.seenRestartCount) direction state.game,
        seenRestartCount: restartCount
    }

value stepOnTick:GameTickState tick:Int state:GameTickState =>
    stepTickState restartCount direction state

value gameValue:Game state:GameTickState =>
    state.game

value statusText:Text game:Game =>
    game.status
     ||> Running  => "Running"
     ||> GameOver => "GameOver"

value stepCount:Int game:Game =>
    game.steps

provider custom.tick
    wakeup: providerTrigger

@source custom.tick
signal tick : Signal Int

@source window.keyDown with {
    repeat: False
    focusOnly: True
}
signal keyDown : Signal Key

signal direction : Signal Direction =
    keyDown
     |> scan Right updateDirectionOrRestart

signal restartCount : Signal Int =
    keyDown
     |> scan 0 updateRestartCount

signal gameState : Signal GameTickState =
    tick
     |> scan initialGameTickState stepOnTick

signal game : Signal Game =
    gameState
     |> gameValue

signal statusLine : Signal Text =
    game
     |> statusText

signal steps : Signal Int =
    game
     |> stepCount
"#,
    );
    let assembly =
        assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
    let mut linked = link_backend_runtime(assembly, &lowered.core, Arc::new(lowered.backend))
        .expect("startup link should succeed");
    let first = linked
        .tick_with_source_lifecycle()
        .expect("initial source lifecycle tick should succeed");
    let tick_instance = linked
        .source_by_owner(item_id(lowered.hir.module(), "tick"))
        .expect("tick source binding should exist")
        .instance;
    let key_instance = linked
        .source_by_owner(item_id(lowered.hir.module(), "keyDown"))
        .expect("keyDown source binding should exist")
        .instance;
    let mut tick_port = None;
    let mut providers = SourceProviderManager::new();
    for action in first.source_actions() {
        match action {
            aivi_runtime::LinkedSourceLifecycleAction::Activate { instance, port, .. }
            | aivi_runtime::LinkedSourceLifecycleAction::Reconfigure { instance, port, .. } => {
                if *instance == tick_instance {
                    tick_port = Some(port.clone());
                }
            }
            aivi_runtime::LinkedSourceLifecycleAction::Suspend { .. } => {}
        }
        if action.instance() == key_instance {
            providers
                .apply_actions(std::slice::from_ref(action))
                .expect("window key source should activate");
        }
    }
    let tick_port = tick_port.expect("custom tick source should activate");
    let direction_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "direction"))
        .expect("direction signal binding should exist")
        .signal();
    let status_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "statusLine"))
        .expect("statusLine signal binding should exist")
        .signal();
    let steps_signal = linked
        .assembly()
        .signal(item_id(lowered.hir.module(), "steps"))
        .expect("steps signal binding should exist")
        .signal();
    let left = lowered
        .hir
        .module()
        .sum_constructor_handle(item_id(lowered.hir.module(), "Direction"), "Left")
        .expect("Direction.Left constructor should resolve");
    let right = lowered
        .hir
        .module()
        .sum_constructor_handle(item_id(lowered.hir.module(), "Direction"), "Right")
        .expect("Direction.Right constructor should resolve");
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("Running".into()))
    );
    assert_eq!(
        linked.runtime().current_value(steps_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );

    providers.dispatch_window_key_event(WindowKeyEvent {
        name: "ArrowLeft".into(),
        repeated: false,
    });
    let left_value = spin_until(&mut linked, direction_signal, Duration::from_millis(200))
        .expect("left key should update direction");
    assert_eq!(
        left_value,
        RuntimeValue::Sum(RuntimeSumValue {
            item: left.item,
            type_name: left.type_name.clone(),
            variant_name: left.variant_name.clone(),
            fields: Vec::new(),
        })
    );

    tick_port
        .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            1,
        )))
        .expect("first tick publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("first tick should update the game");
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("GameOver".into()))
    );
    assert_eq!(
        linked.runtime().current_value(steps_signal).unwrap(),
        Some(&RuntimeValue::Int(1))
    );

    providers.dispatch_window_key_event(WindowKeyEvent {
        name: "Space".into(),
        repeated: false,
    });
    let right_value = spin_until(&mut linked, direction_signal, Duration::from_millis(200))
        .expect("space key should reset direction");
    assert_eq!(
        right_value,
        RuntimeValue::Sum(RuntimeSumValue {
            item: right.item,
            type_name: right.type_name.clone(),
            variant_name: right.variant_name.clone(),
            fields: Vec::new(),
        })
    );

    tick_port
        .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            2,
        )))
        .expect("restart tick publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("restart tick should reset the game");
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("Running".into()))
    );
    assert_eq!(
        linked.runtime().current_value(steps_signal).unwrap(),
        Some(&RuntimeValue::Int(0))
    );

    tick_port
        .publish(DetachedRuntimeValue::from_runtime_owned(RuntimeValue::Int(
            3,
        )))
        .expect("post-restart tick publication should queue");
    linked
        .tick_with_source_lifecycle()
        .expect("post-restart tick should advance normally");
    assert_eq!(
        linked.runtime().current_value(status_signal).unwrap(),
        Some(&RuntimeValue::Text("Running".into()))
    );
    assert_eq!(
        linked.runtime().current_value(steps_signal).unwrap(),
        Some(&RuntimeValue::Int(1)),
        "restart requests must be consumed so later deaths do not auto-restart without a new Space press"
    );
}
