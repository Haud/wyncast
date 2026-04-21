#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusTarget {
    #[default]
    None,
    MainPanel,
    Budget,
    Roster,
    Scarcity,
    NominationPlan,
}

impl FocusTarget {
    #[allow(dead_code)]
    pub fn cycle_forward(self) -> Self {
        match self {
            FocusTarget::None => FocusTarget::MainPanel,
            FocusTarget::MainPanel => FocusTarget::Budget,
            FocusTarget::Budget => FocusTarget::Roster,
            FocusTarget::Roster => FocusTarget::Scarcity,
            FocusTarget::Scarcity => FocusTarget::NominationPlan,
            FocusTarget::NominationPlan => FocusTarget::None,
        }
    }

    #[allow(dead_code)]
    pub fn cycle_backward(self) -> Self {
        match self {
            FocusTarget::None => FocusTarget::NominationPlan,
            FocusTarget::NominationPlan => FocusTarget::Scarcity,
            FocusTarget::Scarcity => FocusTarget::Roster,
            FocusTarget::Roster => FocusTarget::Budget,
            FocusTarget::Budget => FocusTarget::MainPanel,
            FocusTarget::MainPanel => FocusTarget::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_wraps() {
        assert_eq!(FocusTarget::NominationPlan.cycle_forward(), FocusTarget::None);
    }

    #[test]
    fn backward_wraps() {
        assert_eq!(FocusTarget::None.cycle_backward(), FocusTarget::NominationPlan);
    }

    #[test]
    fn full_forward_cycle_returns_to_start() {
        let mut f = FocusTarget::None;
        for _ in 0..6 {
            f = f.cycle_forward();
        }
        assert_eq!(f, FocusTarget::None);
    }
}
