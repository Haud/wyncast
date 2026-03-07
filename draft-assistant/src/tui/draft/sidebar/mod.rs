pub mod plan;
pub mod roster;
pub mod scarcity;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::draft::roster::RosterSlot;
use crate::tui::action::Action;
use crate::tui::BudgetStatus;
use crate::tui::widgets;
use crate::valuation::scarcity::ScarcityEntry;

use plan::{PlanPanel, PlanPanelMessage};
use roster::{RosterPanel, RosterMessage};
use scarcity::{ScarcityPanel, ScarcityPanelMessage};

// ---------------------------------------------------------------------------
// SidebarMessage
// ---------------------------------------------------------------------------

/// Messages that can be sent to the Sidebar component.
#[derive(Debug, Clone)]
pub enum SidebarMessage {
    Roster(RosterMessage),
    Scarcity(ScarcityPanelMessage),
    Plan(PlanPanelMessage),
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/// Mid-level component composing the four sidebar sections:
/// roster, scarcity, budget (stateless), and nomination plan.
pub struct Sidebar {
    pub roster: RosterPanel,
    pub scarcity: ScarcityPanel,
    pub plan: PlanPanel,
    // budget is stateless — no owned state needed
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            roster: RosterPanel::new(),
            scarcity: ScarcityPanel::new(),
            plan: PlanPanel::new(),
        }
    }

    pub fn update(&mut self, msg: SidebarMessage) -> Option<Action> {
        match msg {
            SidebarMessage::Roster(m) => self.roster.update(m),
            SidebarMessage::Scarcity(m) => self.scarcity.update(m),
            SidebarMessage::Plan(m) => self.plan.update(m),
        }
    }

    /// Render all four sidebar sections into their respective areas.
    ///
    /// Budget rendering delegates to the stateless `widgets::budget::render()`.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        roster_area: Rect,
        scarcity_area: Rect,
        budget_area: Rect,
        plan_area: Rect,
        my_roster: &[RosterSlot],
        positional_scarcity: &[ScarcityEntry],
        nominated_position: Option<&Position>,
        budget: &BudgetStatus,
        budget_scroll_offset: usize,
        roster_focused: bool,
        scarcity_focused: bool,
        budget_focused: bool,
        plan_focused: bool,
    ) {
        self.roster.view(frame, roster_area, my_roster, nominated_position, roster_focused);
        self.scarcity.view(frame, scarcity_area, positional_scarcity, nominated_position, scarcity_focused);
        widgets::budget::render(frame, budget_area, budget, budget_scroll_offset, budget_focused);
        self.plan.view(frame, plan_area, plan_focused);
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
