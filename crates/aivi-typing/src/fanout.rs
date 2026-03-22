//! Focused RFC §11.5 fan-out carrier planning.
//!
//! The current compiler wave still stops at resolved HIR plus focused typing helpers, but it can
//! make the `*|>` / `<|*` carrier split explicit and testable: map stages preserve ordinary-vs-
//! `Signal` flow around mapped collections, and join stages preserve the same carrier while
//! reducing each mapped collection value.

/// The outer carrier seen by one fan-out segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FanoutCarrier {
    Ordinary,
    Signal,
}

/// The focused stage kinds covered by RFC §11.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FanoutStageKind {
    Map,
    Join,
}

/// The result-shape decision chosen for one fan-out stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FanoutResultKind {
    MappedCollection,
    JoinedValue,
}

/// Focused carrier plan for one `*|>` or `<|*` stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FanoutPlan {
    carrier: FanoutCarrier,
    stage: FanoutStageKind,
    result: FanoutResultKind,
    lifts_pointwise: bool,
}

impl FanoutPlan {
    /// Compute the RFC §11.5 behavior for one known carrier and stage kind.
    pub const fn for_stage(stage: FanoutStageKind, carrier: FanoutCarrier) -> Self {
        Self {
            carrier,
            stage,
            result: match stage {
                FanoutStageKind::Map => FanoutResultKind::MappedCollection,
                FanoutStageKind::Join => FanoutResultKind::JoinedValue,
            },
            lifts_pointwise: matches!(carrier, FanoutCarrier::Signal),
        }
    }

    pub const fn carrier(self) -> FanoutCarrier {
        self.carrier
    }

    pub const fn stage(self) -> FanoutStageKind {
        self.stage
    }

    pub const fn result(self) -> FanoutResultKind {
        self.result
    }

    pub const fn lifts_pointwise(self) -> bool {
        self.lifts_pointwise
    }
}

/// Stateless entry point used by resolved-HIR validation today and typed elaboration later.
pub struct FanoutPlanner;

impl FanoutPlanner {
    pub const fn plan(stage: FanoutStageKind, carrier: FanoutCarrier) -> FanoutPlan {
        FanoutPlan::for_stage(stage, carrier)
    }
}

#[cfg(test)]
mod tests {
    use super::{FanoutCarrier, FanoutPlan, FanoutPlanner, FanoutResultKind, FanoutStageKind};

    #[test]
    fn ordinary_map_stages_return_mapped_collections_without_lift() {
        let plan = FanoutPlanner::plan(FanoutStageKind::Map, FanoutCarrier::Ordinary);
        assert_eq!(
            plan,
            FanoutPlan::for_stage(FanoutStageKind::Map, FanoutCarrier::Ordinary)
        );
        assert_eq!(plan.carrier(), FanoutCarrier::Ordinary);
        assert_eq!(plan.stage(), FanoutStageKind::Map);
        assert_eq!(plan.result(), FanoutResultKind::MappedCollection);
        assert!(
            !plan.lifts_pointwise(),
            "ordinary fan-out maps stay in ordinary collection flow"
        );
    }

    #[test]
    fn signal_map_stages_lift_pointwise() {
        let plan = FanoutPlanner::plan(FanoutStageKind::Map, FanoutCarrier::Signal);
        assert_eq!(
            plan,
            FanoutPlan::for_stage(FanoutStageKind::Map, FanoutCarrier::Signal)
        );
        assert_eq!(plan.carrier(), FanoutCarrier::Signal);
        assert_eq!(plan.stage(), FanoutStageKind::Map);
        assert_eq!(plan.result(), FanoutResultKind::MappedCollection);
        assert!(
            plan.lifts_pointwise(),
            "signal fan-out maps must stay pointwise over signal updates"
        );
    }

    #[test]
    fn ordinary_join_stages_reduce_without_lift() {
        let plan = FanoutPlanner::plan(FanoutStageKind::Join, FanoutCarrier::Ordinary);
        assert_eq!(
            plan,
            FanoutPlan::for_stage(FanoutStageKind::Join, FanoutCarrier::Ordinary)
        );
        assert_eq!(plan.carrier(), FanoutCarrier::Ordinary);
        assert_eq!(plan.stage(), FanoutStageKind::Join);
        assert_eq!(plan.result(), FanoutResultKind::JoinedValue);
        assert!(
            !plan.lifts_pointwise(),
            "ordinary fan-in stays in ordinary flow"
        );
    }

    #[test]
    fn signal_join_stages_reduce_pointwise() {
        let plan = FanoutPlanner::plan(FanoutStageKind::Join, FanoutCarrier::Signal);
        assert_eq!(
            plan,
            FanoutPlan::for_stage(FanoutStageKind::Join, FanoutCarrier::Signal)
        );
        assert_eq!(plan.carrier(), FanoutCarrier::Signal);
        assert_eq!(plan.stage(), FanoutStageKind::Join);
        assert_eq!(plan.result(), FanoutResultKind::JoinedValue);
        assert!(
            plan.lifts_pointwise(),
            "signal fan-in must reduce each mapped collection update pointwise"
        );
    }
}
