//! Focused RFC §11.3 gate carrier planning.
//!
//! The current compiler wave does not yet have full typed-core elaboration or runtime lowering,
//! but it can still make the carrier decision explicit and testable: ordinary subjects lower
//! through `Option`, while `Signal` subjects keep their carrier and suppress negative updates.

/// The input carrier seen by one `?|>` stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateCarrier {
    Ordinary,
    Signal,
}

/// The result-shape decision chosen for one gate stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateResultKind {
    OptionWrappedSubject,
    PreservedSignalSubject,
}

/// Focused carrier plan for RFC §11.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GatePlan {
    carrier: GateCarrier,
    result: GateResultKind,
    emits_negative_update: bool,
}

impl GatePlan {
    /// Compute the gate behavior for one known input carrier.
    pub const fn for_carrier(carrier: GateCarrier) -> Self {
        match carrier {
            GateCarrier::Ordinary => Self {
                carrier,
                result: GateResultKind::OptionWrappedSubject,
                emits_negative_update: false,
            },
            GateCarrier::Signal => Self {
                carrier,
                result: GateResultKind::PreservedSignalSubject,
                emits_negative_update: false,
            },
        }
    }

    pub const fn carrier(self) -> GateCarrier {
        self.carrier
    }

    pub const fn result(self) -> GateResultKind {
        self.result
    }

    pub const fn emits_negative_update(self) -> bool {
        self.emits_negative_update
    }
}

/// Stateless entry point used by HIR validation today and typed elaboration later.
pub struct GatePlanner;

impl GatePlanner {
    // TODO: Nested gate semantics not defined.
    // A gate predicate that contains another gate expression (e.g., expr |> (inner |> ?|>))
    // has no plan representation here. The current `GatePlan` is single-stage.
    // HIR gate_elaboration.rs must detect and block nested gates explicitly.
    // See CODE_REVIEW.md §5 (gate_elaboration.rs problem #6).
    pub const fn plan(carrier: GateCarrier) -> GatePlan {
        GatePlan::for_carrier(carrier)
    }
}

#[cfg(test)]
mod tests {
    use super::{GateCarrier, GatePlan, GatePlanner, GateResultKind};

    #[test]
    fn ordinary_subjects_lower_through_option() {
        let plan = GatePlanner::plan(GateCarrier::Ordinary);
        assert_eq!(plan, GatePlan::for_carrier(GateCarrier::Ordinary));
        assert_eq!(plan.carrier(), GateCarrier::Ordinary);
        assert_eq!(plan.result(), GateResultKind::OptionWrappedSubject);
        assert!(
            !plan.emits_negative_update(),
            "ordinary gates lower to `Some` / `None`, not scheduler updates"
        );
    }

    #[test]
    fn signal_subjects_preserve_signal_carrier_without_negative_updates() {
        let plan = GatePlanner::plan(GateCarrier::Signal);
        assert_eq!(plan, GatePlan::for_carrier(GateCarrier::Signal));
        assert_eq!(plan.carrier(), GateCarrier::Signal);
        assert_eq!(plan.result(), GateResultKind::PreservedSignalSubject);
        assert!(
            !plan.emits_negative_update(),
            "RFC §11.3 forbids synthetic negative signal updates"
        );
    }
}
