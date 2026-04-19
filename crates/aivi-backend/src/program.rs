use std::{collections::HashMap, fmt};

use aivi_base::SourceSpan;
use aivi_core::Arena;
use aivi_hir::ItemId as HirItemId;

use crate::{
    CallingConvention, DecodePlanId, DecodeStepId, EnvSlotId, ItemId, KernelFingerprint, KernelId,
    LayoutId, PipelineId, SourceId,
    kernel::{
        BuiltinTerm, Kernel, KernelExpr, KernelExprKind, KernelOrigin, KernelOriginKind,
        describe_expr_kind,
    },
    layout::{Layout, PrimitiveType},
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Program {
    items: Arena<ItemId, Item>,
    pipelines: Arena<PipelineId, Pipeline>,
    kernels: Arena<KernelId, Kernel>,
    layouts: Arena<LayoutId, Layout>,
    sources: Arena<SourceId, SourcePlan>,
    decode_plans: Arena<DecodePlanId, DecodePlan>,
    /// Maps `(HIR domain ItemId, member index)` → backend `ItemId` for domain members
    /// that have compiled bodies. Populated during backend lowering for fast dispatch.
    #[serde(with = "domain_member_items_serde")]
    domain_member_items: HashMap<(HirItemId, usize), ItemId>,
    /// Maps each concrete named-domain layout to the concrete layout of its erased carrier.
    #[serde(with = "named_domain_carriers_serde")]
    named_domain_carriers: HashMap<LayoutId, LayoutId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeKernelMeta {
    pub fingerprint: KernelFingerprint,
    pub origin_item: ItemId,
    pub input_subject: Option<LayoutId>,
    pub environment: Vec<LayoutId>,
    pub result_layout: LayoutId,
    pub convention: CallingConvention,
    pub global_items: Vec<ItemId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BackendRuntimeMeta {
    items: Arena<ItemId, Item>,
    pipelines: Arena<PipelineId, Pipeline>,
    kernels: Arena<KernelId, RuntimeKernelMeta>,
    layouts: Arena<LayoutId, Layout>,
    sources: Arena<SourceId, SourcePlan>,
    #[serde(with = "named_domain_carriers_serde")]
    named_domain_carriers: HashMap<LayoutId, LayoutId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct DomainMemberItemWire {
    domain: HirItemId,
    member_index: usize,
    item: ItemId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NamedDomainCarrierWire {
    domain_layout: LayoutId,
    carrier_layout: LayoutId,
}

mod domain_member_items_serde {
    use std::collections::HashMap;

    use aivi_hir::ItemId as HirItemId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

    use crate::ItemId;

    use super::DomainMemberItemWire;

    pub fn serialize<S>(
        value: &HashMap<(HirItemId, usize), ItemId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries = value
            .iter()
            .map(|(&(domain, member_index), &item)| DomainMemberItemWire {
                domain,
                member_index,
                item,
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| (entry.domain.as_raw(), entry.member_index));
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<HashMap<(HirItemId, usize), ItemId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<DomainMemberItemWire>::deserialize(deserializer)?;
        let mut map = HashMap::with_capacity(entries.len());
        for entry in entries {
            let key = (entry.domain, entry.member_index);
            if map.insert(key, entry.item).is_some() {
                return Err(D::Error::custom(format!(
                    "duplicate domain member backend item mapping for domain{} member {}",
                    entry.domain, entry.member_index
                )));
            }
        }
        Ok(map)
    }
}

mod named_domain_carriers_serde {
    use std::collections::HashMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

    use crate::LayoutId;

    use super::NamedDomainCarrierWire;

    pub fn serialize<S>(
        value: &HashMap<LayoutId, LayoutId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries = value
            .iter()
            .map(|(&domain_layout, &carrier_layout)| NamedDomainCarrierWire {
                domain_layout,
                carrier_layout,
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.domain_layout.as_raw());
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<LayoutId, LayoutId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<NamedDomainCarrierWire>::deserialize(deserializer)?;
        let mut map = HashMap::with_capacity(entries.len());
        for entry in entries {
            if map
                .insert(entry.domain_layout, entry.carrier_layout)
                .is_some()
            {
                return Err(D::Error::custom(format!(
                    "duplicate named-domain carrier mapping for layout{}",
                    entry.domain_layout
                )));
            }
        }
        Ok(map)
    }
}

impl Default for Program {
    fn default() -> Self {
        Self {
            items: Arena::new(),
            pipelines: Arena::new(),
            kernels: Arena::new(),
            layouts: Arena::new(),
            sources: Arena::new(),
            decode_plans: Arena::new(),
            domain_member_items: HashMap::new(),
            named_domain_carriers: HashMap::new(),
        }
    }
}

impl Program {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &Arena<ItemId, Item> {
        &self.items
    }

    pub fn items_mut(&mut self) -> &mut Arena<ItemId, Item> {
        &mut self.items
    }

    pub fn pipelines(&self) -> &Arena<PipelineId, Pipeline> {
        &self.pipelines
    }

    pub fn pipelines_mut(&mut self) -> &mut Arena<PipelineId, Pipeline> {
        &mut self.pipelines
    }

    pub fn kernels(&self) -> &Arena<KernelId, Kernel> {
        &self.kernels
    }

    pub fn kernels_mut(&mut self) -> &mut Arena<KernelId, Kernel> {
        &mut self.kernels
    }

    pub fn layouts(&self) -> &Arena<LayoutId, Layout> {
        &self.layouts
    }

    pub fn layouts_mut(&mut self) -> &mut Arena<LayoutId, Layout> {
        &mut self.layouts
    }

    pub fn sources(&self) -> &Arena<SourceId, SourcePlan> {
        &self.sources
    }

    pub fn sources_mut(&mut self) -> &mut Arena<SourceId, SourcePlan> {
        &mut self.sources
    }

    pub fn decode_plans(&self) -> &Arena<DecodePlanId, DecodePlan> {
        &self.decode_plans
    }

    pub fn decode_plans_mut(&mut self) -> &mut Arena<DecodePlanId, DecodePlan> {
        &mut self.decode_plans
    }

    pub fn item_name(&self, item: ItemId) -> &str {
        &self.items[item].name
    }

    /// Returns the backend `ItemId` for a domain member that has a compiled body,
    /// keyed by the HIR domain `ItemId` and the member's zero-based index.
    pub fn domain_member_item(&self, domain: HirItemId, member_index: usize) -> Option<ItemId> {
        self.domain_member_items
            .get(&(domain, member_index))
            .copied()
    }

    pub fn domain_member_items_mut(&mut self) -> &mut HashMap<(HirItemId, usize), ItemId> {
        &mut self.domain_member_items
    }

    pub fn named_domain_carrier(&self, layout: LayoutId) -> Option<LayoutId> {
        self.named_domain_carriers.get(&layout).copied()
    }

    pub fn register_named_domain_carrier(&mut self, layout: LayoutId, carrier: LayoutId) {
        let prior = self.named_domain_carriers.insert(layout, carrier);
        debug_assert!(prior.is_none_or(|existing| existing == carrier));
    }

    pub fn pretty(&self) -> String {
        format!("{self}")
    }
}

impl BackendRuntimeMeta {
    pub fn items(&self) -> &Arena<ItemId, Item> {
        &self.items
    }

    pub fn pipelines(&self) -> &Arena<PipelineId, Pipeline> {
        &self.pipelines
    }

    pub fn kernels(&self) -> &Arena<KernelId, RuntimeKernelMeta> {
        &self.kernels
    }

    pub fn layouts(&self) -> &Arena<LayoutId, Layout> {
        &self.layouts
    }

    pub fn layouts_mut(&mut self) -> &mut Arena<LayoutId, Layout> {
        &mut self.layouts
    }

    pub fn sources(&self) -> &Arena<SourceId, SourcePlan> {
        &self.sources
    }

    pub fn named_domain_carrier(&self, layout: LayoutId) -> Option<LayoutId> {
        self.named_domain_carriers.get(&layout).copied()
    }

    pub fn item_name(&self, item: ItemId) -> &str {
        &self.items[item].name
    }

    pub fn to_replay_program(&self) -> Program {
        let mut kernels = Arena::new();
        for (kernel_id, kernel) in self.kernels.iter() {
            let mut exprs = Arena::new();
            let root = exprs
                .alloc(KernelExpr {
                    span: SourceSpan::new(aivi_base::FileId::new(0), aivi_base::Span::default()),
                    layout: kernel.result_layout,
                    kind: KernelExprKind::Environment(EnvSlotId::from_raw(0)),
                })
                .expect("replay kernel expression arena should not overflow");
            let inserted = kernels
                .alloc(Kernel::new(
                    KernelOrigin {
                        item: kernel.origin_item,
                        span: SourceSpan::new(
                            aivi_base::FileId::new(0),
                            aivi_base::Span::default(),
                        ),
                        kind: KernelOriginKind::ItemBody {
                            item: kernel.origin_item,
                        },
                    },
                    kernel.input_subject,
                    Vec::new(),
                    kernel.environment.clone(),
                    kernel.result_layout,
                    kernel.convention.clone(),
                    kernel.global_items.clone(),
                    root,
                    exprs,
                ))
                .expect("replay kernel arena should not overflow");
            debug_assert_eq!(inserted, kernel_id);
        }
        Program {
            items: self.items.clone(),
            pipelines: self.pipelines.clone(),
            kernels,
            layouts: self.layouts.clone(),
            sources: self.sources.clone(),
            decode_plans: Arena::new(),
            domain_member_items: HashMap::new(),
            named_domain_carriers: self.named_domain_carriers.clone(),
        }
    }
}

impl From<&Program> for BackendRuntimeMeta {
    fn from(program: &Program) -> Self {
        let mut kernels = Arena::new();
        for (kernel_id, kernel) in program.kernels().iter() {
            let inserted = kernels
                .alloc(RuntimeKernelMeta {
                    fingerprint: crate::compute_kernel_fingerprint(program, kernel_id),
                    origin_item: kernel.origin.item,
                    input_subject: kernel.input_subject,
                    environment: kernel.environment.clone(),
                    result_layout: kernel.result_layout,
                    convention: kernel.convention.clone(),
                    global_items: kernel.global_items.clone(),
                })
                .expect("runtime kernel metadata arena should not overflow");
            debug_assert_eq!(inserted, kernel_id);
        }
        Self {
            items: program.items.clone(),
            pipelines: program.pipelines.clone(),
            kernels,
            layouts: program.layouts.clone(),
            sources: program.sources.clone(),
            named_domain_carriers: program.named_domain_carriers.clone(),
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (item_id, item) in self.items.iter() {
            writeln!(f, "{} {} (item{item_id}):", item.kind.label(), item.name)?;
            if !item.parameters.is_empty() {
                write!(f, "  params = [")?;
                for (index, parameter) in item.parameters.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "layout{parameter}")?;
                }
                writeln!(f, "]")?;
            }
            if let Some(body) = item.body {
                writeln!(f, "  body = kernel{body}")?;
            }
            if let ItemKind::Signal(signal) = &item.kind {
                if !signal.dependencies.is_empty() {
                    write!(f, "  dependencies = [")?;
                    for (index, dependency) in signal.dependencies.iter().enumerate() {
                        if index > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{}", self.item_name(*dependency))?;
                        if let Some(layout) = signal.dependency_layouts.get(index) {
                            write!(f, ": layout{layout}")?;
                        }
                    }
                    writeln!(f, "]")?;
                }
                if let Some(body_kernel) = signal.body_kernel {
                    writeln!(f, "  signal-body = kernel{body_kernel}")?;
                }
                if let Some(source_id) = signal.source {
                    let source = &self.sources[source_id];
                    writeln!(
                        f,
                        "  source {} provider={} cancellation={}",
                        source.instance, source.provider, source.cancellation
                    )?;
                    for (index, argument) in source.arguments.iter().enumerate() {
                        writeln!(f, "    arg[{index}] = kernel{}", argument.kernel)?;
                    }
                    for option in &source.options {
                        writeln!(
                            f,
                            "    option {} = kernel{}",
                            option.option_name, option.kernel
                        )?;
                    }
                    if !source.reconfiguration_dependencies.is_empty() {
                        write!(f, "    reconfigure = [")?;
                        for (index, dependency) in
                            source.reconfiguration_dependencies.iter().enumerate()
                        {
                            if index > 0 {
                                f.write_str(", ")?;
                            }
                            write!(f, "{}", self.item_name(*dependency))?;
                        }
                        writeln!(f, "]")?;
                    }
                    if let Some(active_when) = &source.active_when {
                        writeln!(f, "    activeWhen = {}", active_when.option_name)?;
                    }
                    if let Some(decode) = source.decode {
                        let decode = &self.decode_plans[decode];
                        writeln!(f, "    decode {:?} root={}", decode.mode, decode.root)?;
                    }
                }
            }
            for pipeline_id in &item.pipelines {
                let pipeline = &self.pipelines[*pipeline_id];
                writeln!(
                    f,
                    "  pipeline {pipeline_id} core-pipe={}:",
                    pipeline.origin.core_pipe
                )?;
                for stage in &pipeline.stages {
                    writeln!(
                        f,
                        "    [{}] {} : layout{} -> layout{}",
                        stage.index,
                        stage.kind.label(),
                        stage.input_layout,
                        stage.result_layout
                    )?;
                    match &stage.kind {
                        StageKind::Gate(GateStage::Ordinary {
                            when_true,
                            when_false,
                        }) => {
                            writeln!(f, "      true  = kernel{when_true}")?;
                            writeln!(f, "      false = kernel{when_false}")?;
                        }
                        StageKind::Gate(GateStage::SignalFilter {
                            payload_layout,
                            predicate,
                            emits_negative_update,
                        }) => {
                            writeln!(
                                f,
                                "      predicate = kernel{predicate} payload=layout{payload_layout} [negative-update={emits_negative_update}]"
                            )?;
                        }
                        StageKind::TruthyFalsy(pair) => {
                            writeln!(
                                f,
                                "      truthy[{}/{}] => layout{}",
                                pair.truthy_stage_index,
                                pair.truthy.constructor,
                                pair.truthy.result_layout
                            )?;
                            writeln!(
                                f,
                                "      falsy [{}/{}] => layout{}",
                                pair.falsy_stage_index,
                                pair.falsy.constructor,
                                pair.falsy.result_layout
                            )?;
                        }
                        StageKind::Fanout(fanout) => {
                            writeln!(
                                f,
                                "      carrier={} element=layout{} mapped=layout{} collection=layout{}",
                                fanout.carrier,
                                fanout.element_layout,
                                fanout.mapped_element_layout,
                                fanout.mapped_collection_layout
                            )?;
                            writeln!(f, "      map = kernel{}", fanout.map)?;
                            for filter in &fanout.filters {
                                writeln!(
                                    f,
                                    "      filter[{}] = kernel{}",
                                    filter.stage_index, filter.predicate
                                )?;
                            }
                            if let Some(join) = &fanout.join {
                                writeln!(
                                    f,
                                    "      join[{}] kernel{} layout{} => layout{}",
                                    join.stage_index,
                                    join.kernel,
                                    join.collection_layout,
                                    join.kernel_result_layout
                                )?;
                            }
                        }
                        StageKind::Temporal(TemporalStage::Previous {
                            payload_layout,
                            seed,
                        }) => {
                            writeln!(
                                f,
                                "      previous seed=kernel{seed} payload=layout{payload_layout}"
                            )?;
                        }
                        StageKind::Temporal(TemporalStage::DiffFunction {
                            input_layout,
                            result_layout,
                            callable_layout,
                            diff,
                        }) => {
                            writeln!(
                                f,
                                "      diff kernel=kernel{diff} callable=layout{callable_layout} input=layout{input_layout} result=layout{result_layout}"
                            )?;
                        }
                        StageKind::Temporal(TemporalStage::DiffSeed {
                            payload_layout,
                            seed,
                        }) => {
                            writeln!(
                                f,
                                "      diff-seed seed=kernel{seed} payload=layout{payload_layout}"
                            )?;
                        }
                        StageKind::Temporal(TemporalStage::Delay {
                            payload_layout,
                            duration_layout,
                            duration,
                        }) => {
                            writeln!(
                                f,
                                "      delay duration=kernel{duration} config=layout{duration_layout} payload=layout{payload_layout}"
                            )?;
                        }
                        StageKind::Temporal(TemporalStage::Burst {
                            payload_layout,
                            every_layout,
                            count_layout,
                            every,
                            count,
                        }) => {
                            writeln!(
                                f,
                                "      burst every=kernel{every} count=kernel{count} every-layout=layout{every_layout} count-layout=layout{count_layout} payload=layout{payload_layout}"
                            )?;
                        }
                    }
                }
                if let Some(recurrence) = &pipeline.recurrence {
                    writeln!(
                        f,
                        "    recurrence target={} wakeup={}",
                        recurrence.target, recurrence.wakeup_kind
                    )?;
                    writeln!(f, "    recurrence-seed kernel{}", recurrence.seed)?;
                    writeln!(
                        f,
                        "      start[{}] = kernel{}",
                        recurrence.start.stage_index, recurrence.start.kernel
                    )?;
                    for step in &recurrence.steps {
                        writeln!(
                            f,
                            "      step [{}] = kernel{}",
                            step.stage_index, step.kernel
                        )?;
                    }
                    if let Some(witness) = &recurrence.non_source_wakeup {
                        writeln!(
                            f,
                            "      witness {} = kernel{}",
                            witness.cause, witness.kernel
                        )?;
                    }
                }
            }
            if !item.pipelines.is_empty() || matches!(item.kind, ItemKind::Signal(_)) {
                writeln!(f)?;
            }
        }

        if !self.kernels.is_empty() {
            writeln!(f, "kernels:")?;
            for (kernel_id, kernel) in self.kernels.iter() {
                writeln!(
                    f,
                    "  kernel{kernel_id} {} owner=item{} result=layout{}",
                    kernel.origin.kind, kernel.origin.item, kernel.result_layout
                )?;
                writeln!(f, "    convention = {}", kernel.convention)?;
                if let Some(input) = kernel.input_subject {
                    writeln!(f, "    input = layout{input}")?;
                }
                if !kernel.inline_subjects.is_empty() {
                    write!(f, "    inline-subjects = [")?;
                    for (index, layout) in kernel.inline_subjects.iter().enumerate() {
                        if index > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "inline{}:layout{}", index, layout)?;
                    }
                    writeln!(f, "]")?;
                }
                if !kernel.environment.is_empty() {
                    write!(f, "    env = [")?;
                    for (index, layout) in kernel.environment.iter().enumerate() {
                        if index > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "env{}:layout{}", index, layout)?;
                    }
                    writeln!(f, "]")?;
                }
                if !kernel.global_items.is_empty() {
                    write!(f, "    globals = [")?;
                    for (index, item) in kernel.global_items.iter().enumerate() {
                        if index > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "item{item}")?;
                    }
                    writeln!(f, "]")?;
                }
                writeln!(f, "    root = expr{}", kernel.root)?;
                for (expr_id, expr) in kernel.exprs().iter() {
                    writeln!(
                        f,
                        "    expr{expr_id}: layout{} {}",
                        expr.layout,
                        describe_expr_kind(&expr.kind)
                    )?;
                }
            }
            writeln!(f)?;
        }

        if !self.decode_plans.is_empty() {
            writeln!(f, "decode-plans:")?;
            for (decode_id, decode) in self.decode_plans.iter() {
                writeln!(
                    f,
                    "  decode{decode_id} owner=item{} mode={:?} root={}",
                    decode.owner, decode.mode, decode.root
                )?;
                for (step_id, step) in decode.steps().iter() {
                    writeln!(
                        f,
                        "    step{step_id}: layout{} {}",
                        step.layout,
                        step.kind.summary()
                    )?;
                }
            }
            writeln!(f)?;
        }

        if !self.layouts.is_empty() {
            writeln!(f, "layouts:")?;
            for (layout_id, layout) in self.layouts.iter() {
                writeln!(f, "  layout{layout_id}: {layout}")?;
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub origin: aivi_core::ItemId,
    pub span: SourceSpan,
    pub name: Box<str>,
    pub kind: ItemKind,
    pub parameters: Vec<LayoutId>,
    pub body: Option<KernelId>,
    pub pipelines: Vec<PipelineId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ItemKind {
    Value,
    Function,
    Signal(SignalInfo),
    Instance,
}

impl ItemKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Function => "func",
            Self::Signal(_) => "signal",
            Self::Instance => "instance",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignalInfo {
    pub dependencies: Vec<ItemId>,
    pub dependency_layouts: Vec<LayoutId>,
    pub body_kernel: Option<KernelId>,
    pub source: Option<SourceId>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PipelineOrigin {
    pub span: SourceSpan,
    pub core_pipe: aivi_core::PipeId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pipeline {
    pub owner: ItemId,
    pub origin: PipelineOrigin,
    pub stages: Vec<Stage>,
    pub recurrence: Option<Recurrence>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stage {
    pub index: usize,
    pub span: SourceSpan,
    pub input_layout: LayoutId,
    pub result_layout: LayoutId,
    pub kind: StageKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StageKind {
    Gate(GateStage),
    TruthyFalsy(TruthyFalsyStage),
    Fanout(FanoutStage),
    Temporal(TemporalStage),
}

impl StageKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Gate(_) => "gate",
            Self::TruthyFalsy(_) => "truthy-falsy",
            Self::Fanout(_) => "fanout",
            Self::Temporal(_) => "temporal",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GateStage {
    Ordinary {
        when_true: KernelId,
        when_false: KernelId,
    },
    SignalFilter {
        payload_layout: LayoutId,
        predicate: KernelId,
        emits_negative_update: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TruthyFalsyStage {
    pub truthy_stage_index: usize,
    pub truthy_stage_span: SourceSpan,
    pub falsy_stage_index: usize,
    pub falsy_stage_span: SourceSpan,
    pub truthy: TruthyFalsyBranch,
    pub falsy: TruthyFalsyBranch,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TruthyFalsyBranch {
    pub constructor: BuiltinTerm,
    pub payload_layout: Option<LayoutId>,
    pub result_layout: LayoutId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FanoutCarrier {
    Ordinary,
    Signal,
}

impl fmt::Display for FanoutCarrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ordinary => f.write_str("ordinary"),
            Self::Signal => f.write_str("signal"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanoutStage {
    pub carrier: FanoutCarrier,
    pub element_layout: LayoutId,
    pub mapped_element_layout: LayoutId,
    pub mapped_collection_layout: LayoutId,
    pub map: KernelId,
    pub filters: Vec<FanoutFilter>,
    pub join: Option<FanoutJoin>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanoutFilter {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub input_layout: LayoutId,
    pub predicate: KernelId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FanoutJoin {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub input_layout: LayoutId,
    pub collection_layout: LayoutId,
    pub kernel: KernelId,
    pub kernel_result_layout: LayoutId,
    pub result_layout: LayoutId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TemporalStage {
    Previous {
        payload_layout: LayoutId,
        seed: KernelId,
    },
    DiffFunction {
        input_layout: LayoutId,
        result_layout: LayoutId,
        callable_layout: LayoutId,
        diff: KernelId,
    },
    DiffSeed {
        payload_layout: LayoutId,
        seed: KernelId,
    },
    Delay {
        payload_layout: LayoutId,
        duration_layout: LayoutId,
        duration: KernelId,
    },
    Burst {
        payload_layout: LayoutId,
        every_layout: LayoutId,
        count_layout: LayoutId,
        every: KernelId,
        count: KernelId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RecurrenceTarget {
    Signal,
    Task,
    SourceHelper,
}

impl fmt::Display for RecurrenceTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signal => f.write_str("signal"),
            Self::Task => f.write_str("task"),
            Self::SourceHelper => f.write_str("source-helper"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RecurrenceWakeupKind {
    Timer,
    Backoff,
    SourceEvent,
    ProviderDefinedTrigger,
}

impl fmt::Display for RecurrenceWakeupKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timer => f.write_str("timer"),
            Self::Backoff => f.write_str("backoff"),
            Self::SourceEvent => f.write_str("source-event"),
            Self::ProviderDefinedTrigger => f.write_str("provider-trigger"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Recurrence {
    pub target: RecurrenceTarget,
    pub wakeup_kind: RecurrenceWakeupKind,
    pub seed: KernelId,
    pub start: RecurrenceStage,
    pub steps: Vec<RecurrenceStage>,
    pub non_source_wakeup: Option<NonSourceWakeup>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecurrenceStage {
    pub stage_index: usize,
    pub stage_span: SourceSpan,
    pub input_layout: LayoutId,
    pub result_layout: LayoutId,
    pub kernel: KernelId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NonSourceWakeupCause {
    ExplicitTimer,
    ExplicitBackoff,
}

impl fmt::Display for NonSourceWakeupCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExplicitTimer => f.write_str("explicit-timer"),
            Self::ExplicitBackoff => f.write_str("explicit-backoff"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NonSourceWakeup {
    pub cause: NonSourceWakeupCause,
    pub kernel: KernelId,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct SourceInstanceId(u32);

impl SourceInstanceId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SourceInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceProvider {
    Missing,
    Builtin(Box<str>),
    Custom(Box<str>),
    InvalidShape(Box<str>),
}

impl fmt::Display for SourceProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("<missing>"),
            Self::Builtin(key) | Self::Custom(key) | Self::InvalidShape(key) => f.write_str(key),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceTeardownPolicy {
    DisposeOnOwnerTeardown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceReplacementPolicy {
    DisposeSupersededBeforePublish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceStaleWorkPolicy {
    DropStalePublications,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SourceCancellationPolicy {
    ProviderManaged,
    CancelInFlight,
}

impl fmt::Display for SourceCancellationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProviderManaged => f.write_str("provider-managed"),
            Self::CancelInFlight => f.write_str("cancel-in-flight"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceOptionBinding {
    pub option_span: SourceSpan,
    pub option_name: Box<str>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceArgumentKernel {
    pub kernel: KernelId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceOptionKernel {
    pub option_name: Box<str>,
    pub kernel: KernelId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourcePlan {
    pub owner: ItemId,
    pub span: SourceSpan,
    pub instance: SourceInstanceId,
    pub provider: SourceProvider,
    pub teardown: SourceTeardownPolicy,
    pub replacement: SourceReplacementPolicy,
    pub arguments: Vec<SourceArgumentKernel>,
    pub options: Vec<SourceOptionKernel>,
    pub reconfiguration_dependencies: Vec<ItemId>,
    pub explicit_triggers: Vec<SourceOptionBinding>,
    pub active_when: Option<SourceOptionBinding>,
    pub cancellation: SourceCancellationPolicy,
    pub stale_work: SourceStaleWorkPolicy,
    pub decode: Option<DecodePlanId>,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum DecodeMode {
    #[default]
    Strict,
    Permissive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DecodeExtraFieldPolicy {
    Reject,
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DecodeFieldRequirement {
    Required,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DecodeSumStrategy {
    Explicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum DomainDecodeSurfaceKind {
    Direct,
    FallibleResult,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DomainDecodeSurface {
    pub member_index: usize,
    pub member_name: Box<str>,
    pub kind: DomainDecodeSurfaceKind,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecodePlan {
    pub owner: ItemId,
    pub mode: DecodeMode,
    pub root: DecodeStepId,
    steps: Arena<DecodeStepId, DecodeStep>,
}

impl DecodePlan {
    pub fn new(
        owner: ItemId,
        mode: DecodeMode,
        root: DecodeStepId,
        steps: Arena<DecodeStepId, DecodeStep>,
    ) -> Self {
        Self {
            owner,
            mode,
            root,
            steps,
        }
    }

    pub fn steps(&self) -> &Arena<DecodeStepId, DecodeStep> {
        &self.steps
    }

    pub fn steps_mut(&mut self) -> &mut Arena<DecodeStepId, DecodeStep> {
        &mut self.steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecodeStep {
    pub layout: LayoutId,
    pub kind: DecodeStepKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DecodeStepKind {
    Scalar {
        scalar: PrimitiveType,
    },
    Tuple {
        elements: Vec<DecodeStepId>,
    },
    Record {
        fields: Vec<DecodeField>,
        extra_fields: DecodeExtraFieldPolicy,
    },
    Sum {
        variants: Vec<DecodeVariant>,
        strategy: DecodeSumStrategy,
    },
    Domain {
        carrier: DecodeStepId,
        surface: DomainDecodeSurface,
    },
    List {
        element: DecodeStepId,
    },
    Option {
        element: DecodeStepId,
    },
    Result {
        error: DecodeStepId,
        value: DecodeStepId,
    },
    Validation {
        error: DecodeStepId,
        value: DecodeStepId,
    },
}

impl DecodeStepKind {
    pub fn summary(&self) -> String {
        match self {
            Self::Scalar { scalar } => format!("scalar {scalar}"),
            Self::Tuple { elements } => format!("tuple elems={}", elements.len()),
            Self::Record { fields, .. } => format!("record fields={}", fields.len()),
            Self::Sum { variants, .. } => format!("sum variants={}", variants.len()),
            Self::Domain { carrier, surface } => {
                format!("domain {} via step{carrier}", surface.member_name)
            }
            Self::List { element } => format!("list step{element}"),
            Self::Option { element } => format!("option step{element}"),
            Self::Result { error, value } => format!("result error=step{error} value=step{value}"),
            Self::Validation { error, value } => {
                format!("validation error=step{error} value=step{value}")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecodeField {
    pub name: Box<str>,
    pub requirement: DecodeFieldRequirement,
    pub step: DecodeStepId,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecodeVariant {
    pub name: Box<str>,
    pub payload: Option<DecodeStepId>,
}
