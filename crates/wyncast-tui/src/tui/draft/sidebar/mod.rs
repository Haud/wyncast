pub mod plan;
pub mod roster;
pub mod scarcity;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::draft::roster::RosterSlot;
use crate::tui::action::Action;
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;
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

/// Mid-level component composing the three sidebar sections:
/// roster, scarcity, and nomination plan.
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

    /// Declare keybindings for the subscription system.
    ///
    /// Sidebar children (roster, scarcity, plan) are stateful scroll components
    /// with no subscription declarations yet. This method is a forward-compatible
    /// placeholder that composes child subscriptions as they are added.
    pub fn subscription(&self, _kb: &mut KeybindManager) -> Subscription<SidebarMessage> {
        // No child subscriptions exist yet. Return none.
        Subscription::none()
    }

    pub fn update(&mut self, msg: SidebarMessage) -> Option<Action> {
        match msg {
            SidebarMessage::Roster(m) => self.roster.update(m),
            SidebarMessage::Scarcity(m) => self.scarcity.update(m),
            SidebarMessage::Plan(m) => self.plan.update(m),
        }
    }

    /// Render the three sidebar sections into their respective areas.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        roster_area: Rect,
        scarcity_area: Rect,
        plan_area: Rect,
        my_roster: &[RosterSlot],
        positional_scarcity: &[ScarcityEntry],
        nominated_position: Option<&Position>,
        roster_focused: bool,
        scarcity_focused: bool,
        plan_focused: bool,
    ) {
        self.roster.view(frame, roster_area, my_roster, nominated_position, roster_focused);
        self.scarcity.view(frame, scarcity_area, positional_scarcity, nominated_position, scarcity_focused);
        self.plan.view(frame, plan_area, plan_focused);
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
