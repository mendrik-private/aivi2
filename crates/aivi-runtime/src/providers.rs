use std::{collections::BTreeMap, fmt, thread, time::Duration};

use aivi_backend::RuntimeValue;
use aivi_typing::BuiltinSourceProvider;

use crate::{
    LinkedSourceLifecycleAction, RuntimeSourceProvider, SourceInstanceId, SourcePublicationPort,
};

#[derive(Clone, Debug, Default)]
pub struct SourceProviderManager {
    active: BTreeMap<SourceInstanceId, RuntimeSourceProvider>,
}

impl SourceProviderManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_provider(&self, instance: SourceInstanceId) -> Option<&RuntimeSourceProvider> {
        self.active.get(&instance)
    }

    pub fn apply_actions(
        &mut self,
        actions: &[LinkedSourceLifecycleAction],
    ) -> Result<(), SourceProviderExecutionError> {
        for action in actions {
            match action {
                LinkedSourceLifecycleAction::Activate {
                    instance,
                    port,
                    config,
                }
                | LinkedSourceLifecycleAction::Reconfigure {
                    instance,
                    port,
                    config,
                } => {
                    self.start_provider(*instance, config, port.clone())?;
                }
                LinkedSourceLifecycleAction::Suspend { instance } => {
                    self.active.remove(instance);
                }
            }
        }
        Ok(())
    }

    fn start_provider(
        &mut self,
        instance: SourceInstanceId,
        config: &crate::EvaluatedSourceConfig,
        port: SourcePublicationPort<RuntimeValue>,
    ) -> Result<(), SourceProviderExecutionError> {
        match &config.provider {
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::TimerEvery) => {
                let plan = TimerPlan::parse(instance, BuiltinSourceProvider::TimerEvery, config)?;
                spawn_timer_every(port, plan);
            }
            RuntimeSourceProvider::Builtin(BuiltinSourceProvider::TimerAfter) => {
                let plan = TimerPlan::parse(instance, BuiltinSourceProvider::TimerAfter, config)?;
                spawn_timer_after(port, plan);
            }
            provider => {
                return Err(SourceProviderExecutionError::UnsupportedProvider {
                    instance,
                    provider: provider.clone(),
                });
            }
        }
        self.active.insert(instance, config.provider.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceProviderExecutionError {
    UnsupportedProvider {
        instance: SourceInstanceId,
        provider: RuntimeSourceProvider,
    },
    InvalidTimerArgumentCount {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        expected: usize,
        found: usize,
    },
    InvalidTimerArgument {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        index: usize,
        value: RuntimeValue,
    },
    InvalidTimerOption {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: Box<str>,
        value: RuntimeValue,
    },
    UnsupportedTimerOption {
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        option_name: Box<str>,
    },
    ZeroTimerInterval {
        instance: SourceInstanceId,
    },
}

impl fmt::Display for SourceProviderExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProvider { instance, provider } => write!(
                f,
                "source instance {} uses unsupported runtime provider {:?}",
                instance.as_raw(),
                provider
            ),
            Self::InvalidTimerArgumentCount {
                instance,
                provider,
                expected,
                found,
            } => write!(
                f,
                "source instance {} provider {} expects {expected} timer argument(s), found {found}",
                instance.as_raw(),
                provider.key()
            ),
            Self::InvalidTimerArgument {
                instance,
                provider,
                index,
                value,
            } => write!(
                f,
                "source instance {} provider {} has invalid timer argument {index}: {:?}",
                instance.as_raw(),
                provider.key(),
                value
            ),
            Self::InvalidTimerOption {
                instance,
                provider,
                option_name,
                value,
            } => write!(
                f,
                "source instance {} provider {} has invalid `{option_name}` option value {:?}",
                instance.as_raw(),
                provider.key(),
                value
            ),
            Self::UnsupportedTimerOption {
                instance,
                provider,
                option_name,
            } => write!(
                f,
                "source instance {} provider {} does not execute `{option_name}` yet",
                instance.as_raw(),
                provider.key()
            ),
            Self::ZeroTimerInterval { instance } => write!(
                f,
                "source instance {} cannot execute `timer.every` with a zero interval",
                instance.as_raw()
            ),
        }
    }
}

impl std::error::Error for SourceProviderExecutionError {}

#[derive(Clone, Copy)]
struct TimerPlan {
    delay: Duration,
    immediate: bool,
}

impl TimerPlan {
    fn parse(
        instance: SourceInstanceId,
        provider: BuiltinSourceProvider,
        config: &crate::EvaluatedSourceConfig,
    ) -> Result<Self, SourceProviderExecutionError> {
        if config.arguments.len() != 1 {
            return Err(SourceProviderExecutionError::InvalidTimerArgumentCount {
                instance,
                provider,
                expected: 1,
                found: config.arguments.len(),
            });
        }
        let delay = parse_duration(instance, provider, 0, &config.arguments[0])?;
        if provider == BuiltinSourceProvider::TimerEvery && delay.is_zero() {
            return Err(SourceProviderExecutionError::ZeroTimerInterval { instance });
        }

        let mut immediate = false;
        for option in &config.options {
            match option.option_name.as_ref() {
                "immediate" => {
                    immediate = parse_bool(instance, provider, &option.option_name, &option.value)?;
                }
                "coalesce" => {
                    let coalesced =
                        parse_bool(instance, provider, &option.option_name, &option.value)?;
                    if !coalesced {
                        return Err(SourceProviderExecutionError::UnsupportedTimerOption {
                            instance,
                            provider,
                            option_name: option.option_name.clone(),
                        });
                    }
                    // The current timer implementation already runs in the RFC's default
                    // coalesced mode, so `coalesce: True` is an honest no-op.
                }
                "activeWhen" => {}
                "jitter" => {
                    return Err(SourceProviderExecutionError::UnsupportedTimerOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
                _ => {
                    return Err(SourceProviderExecutionError::UnsupportedTimerOption {
                        instance,
                        provider,
                        option_name: option.option_name.clone(),
                    });
                }
            }
        }

        Ok(Self { delay, immediate })
    }
}

fn spawn_timer_every(port: SourcePublicationPort<RuntimeValue>, plan: TimerPlan) {
    thread::spawn(move || {
        if plan.immediate && port.publish(RuntimeValue::Unit).is_err() {
            return;
        }
        while !port.is_cancelled() {
            thread::sleep(plan.delay);
            if port.is_cancelled() {
                break;
            }
            if port.publish(RuntimeValue::Unit).is_err() {
                break;
            }
        }
    });
}

fn spawn_timer_after(port: SourcePublicationPort<RuntimeValue>, plan: TimerPlan) {
    thread::spawn(move || {
        if !plan.immediate {
            thread::sleep(plan.delay);
            if port.is_cancelled() {
                return;
            }
        }
        let _ = port.publish(RuntimeValue::Unit);
    });
}

fn strip_signal(value: &RuntimeValue) -> &RuntimeValue {
    let mut current = value;
    while let RuntimeValue::Signal(inner) = current {
        current = inner;
    }
    current
}

fn parse_bool(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    option_name: &str,
    value: &RuntimeValue,
) -> Result<bool, SourceProviderExecutionError> {
    match strip_signal(value) {
        RuntimeValue::Bool(value) => Ok(*value),
        other => Err(SourceProviderExecutionError::InvalidTimerOption {
            instance,
            provider,
            option_name: option_name.into(),
            value: other.clone(),
        }),
    }
}

fn parse_duration(
    instance: SourceInstanceId,
    provider: BuiltinSourceProvider,
    index: usize,
    value: &RuntimeValue,
) -> Result<Duration, SourceProviderExecutionError> {
    match strip_signal(value) {
        RuntimeValue::Int(value) if *value >= 0 => Ok(Duration::from_millis(*value as u64)),
        RuntimeValue::SuffixedInteger { raw, suffix } => {
            let amount = raw.parse::<u64>().map_err(|_| {
                SourceProviderExecutionError::InvalidTimerArgument {
                    instance,
                    provider,
                    index,
                    value: value.clone(),
                }
            })?;
            duration_from_suffix(amount, suffix).ok_or_else(|| {
                SourceProviderExecutionError::InvalidTimerArgument {
                    instance,
                    provider,
                    index,
                    value: value.clone(),
                }
            })
        }
        other => Err(SourceProviderExecutionError::InvalidTimerArgument {
            instance,
            provider,
            index,
            value: other.clone(),
        }),
    }
}

fn duration_from_suffix(amount: u64, suffix: &str) -> Option<Duration> {
    match suffix {
        "ns" => Some(Duration::from_nanos(amount)),
        "us" => Some(Duration::from_micros(amount)),
        "ms" => Some(Duration::from_millis(amount)),
        "s" => Some(Duration::from_secs(amount)),
        "m" => amount.checked_mul(60).map(Duration::from_secs),
        "h" => amount.checked_mul(60 * 60).map(Duration::from_secs),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use aivi_base::SourceDatabase;
    use aivi_hir::{Item, lower_module as lower_hir_module};
    use aivi_lambda::lower_module as lower_lambda_module;
    use aivi_syntax::parse_module;

    use super::*;
    use crate::{assemble_hir_runtime, link_backend_runtime};

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
        let core =
            aivi_core::lower_module(hir.module()).expect("typed-core lowering should succeed");
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

    #[test]
    fn timer_every_actions_publish_unit_immediately() {
        let lowered = lower_text(
            "runtime-provider-timer-every.aivi",
            r#"
@source timer.every 5 with {
    immediate: True
}
sig tick : Signal Unit
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");

        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("timer provider actions should execute");

        let tick_signal = linked
            .assembly()
            .signal(item_id(lowered.hir.module(), "tick"))
            .expect("tick signal binding should exist")
            .signal();
        let mut published = false;
        for _ in 0..20 {
            linked.tick().expect("runtime tick should succeed");
            if linked
                .runtime()
                .current_value(tick_signal)
                .unwrap()
                .is_some()
            {
                published = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(published, "timer.every should eventually publish Unit");
        assert_eq!(
            linked.runtime().current_value(tick_signal).unwrap(),
            Some(&RuntimeValue::Unit)
        );
    }

    #[test]
    fn unsupported_providers_fail_explicitly() {
        let lowered = lower_text(
            "runtime-provider-http-unsupported.aivi",
            r#"
@source http.get "/users"
sig users : Signal Text
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");

        let error = SourceProviderManager::new()
            .apply_actions(actions.source_actions())
            .expect_err("unsupported providers should report an explicit execution error");
        assert!(matches!(
            error,
            SourceProviderExecutionError::UnsupportedProvider {
                provider: RuntimeSourceProvider::Builtin(BuiltinSourceProvider::HttpGet),
                ..
            }
        ));
    }

    #[test]
    fn timer_every_rejects_non_coalesced_execution() {
        let lowered = lower_text(
            "runtime-provider-timer-coalesce.aivi",
            r#"
@source timer.every 5 with {
    coalesce: False
}
sig tick : Signal Unit
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");

        let error = SourceProviderManager::new()
            .apply_actions(actions.source_actions())
            .expect_err("non-coalesced timers should stay an explicit later slice");
        assert!(matches!(
            error,
            SourceProviderExecutionError::UnsupportedTimerOption { option_name, .. }
                if option_name.as_ref() == "coalesce"
        ));
    }

    #[test]
    fn timer_every_accepts_explicit_coalesced_execution() {
        let lowered = lower_text(
            "runtime-provider-timer-coalesce-true.aivi",
            r#"
@source timer.every 5 with {
    immediate: True
    coalesce: True
}
sig tick : Signal Unit
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");
        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");

        SourceProviderManager::new()
            .apply_actions(actions.source_actions())
            .expect("coalesced timer execution should stay supported");
    }

    #[test]
    fn strip_signal_handles_deep_nesting_without_recursion() {
        let mut value = RuntimeValue::Int(7);
        for _ in 0..10_000 {
            value = RuntimeValue::Signal(Box::new(value));
        }

        assert!(matches!(strip_signal(&value), RuntimeValue::Int(7)));
    }

    #[test]
    fn timer_every_stops_after_source_suspension() {
        let lowered = lower_text(
            "runtime-provider-timer-cancel.aivi",
            r#"
@source timer.every 5
sig tick : Signal Unit
"#,
        );
        let assembly =
            assemble_hir_runtime(lowered.hir.module()).expect("runtime assembly should build");
        let mut linked = link_backend_runtime(assembly, &lowered.core, &lowered.backend)
            .expect("startup link should succeed");

        let actions = linked
            .tick_with_source_lifecycle()
            .expect("linked runtime tick should succeed");
        let mut providers = SourceProviderManager::new();
        providers
            .apply_actions(actions.source_actions())
            .expect("timer provider actions should execute");

        let tick_item = item_id(lowered.hir.module(), "tick");
        let instance = linked
            .source_by_owner(tick_item)
            .expect("tick source binding should exist")
            .instance;
        let tick_signal = linked
            .assembly()
            .signal(tick_item)
            .expect("tick signal binding should exist")
            .signal();

        let mut observed_publication = false;
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(8));
            let outcome = linked.tick().expect("runtime tick should succeed");
            if outcome.committed().contains(&tick_signal) {
                observed_publication = true;
                break;
            }
        }
        assert!(
            observed_publication,
            "timer.every should publish before cancellation"
        );

        linked
            .runtime_mut()
            .suspend_source(instance)
            .expect("source suspension should cancel the active timer port");
        providers
            .apply_actions(&[crate::LinkedSourceLifecycleAction::Suspend { instance }])
            .expect("provider manager should drop suspended timer state");

        for _ in 0..5 {
            thread::sleep(Duration::from_millis(12));
            let outcome = linked
                .tick()
                .expect("runtime tick should stay quiet after timer cancellation");
            assert!(
                outcome.committed().is_empty(),
                "suspended timers should not commit further values"
            );
            assert!(
                outcome.dropped_publications().is_empty(),
                "suspended timers should stop publishing instead of producing stale drops"
            );
        }
    }
}
