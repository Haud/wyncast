pub mod nomination_plan;
pub mod roster;
pub mod scarcity;

use iced::{Element, Length, Padding, Task};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::draft::roster::RosterSlot;
use wyncast_baseball::valuation::scarcity::ScarcityEntry;
use twui::{StackGap, StackStyle, v_stack};

use crate::focus::FocusTarget;
use nomination_plan::{PlanMessage, PlanPanel};
use roster::{RosterMessage, RosterPanel};
use scarcity::ScarcityPanel;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SidebarMessage {
    Roster(RosterMessage),
    Plan(PlanMessage),
    ScarcityScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct Sidebar {
    pub roster: RosterPanel,
    pub scarcity: ScarcityPanel,
    pub plan: PlanPanel,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            roster: RosterPanel::new(),
            scarcity: ScarcityPanel::new(),
            plan: PlanPanel::new(),
        }
    }

    pub fn update(&mut self, msg: SidebarMessage) -> Task<SidebarMessage> {
        match msg {
            SidebarMessage::Roster(m) => self.roster.update(m).map(SidebarMessage::Roster),
            SidebarMessage::Plan(m) => self.plan.update(m).map(SidebarMessage::Plan),
            SidebarMessage::ScarcityScrollBy(dir) => {
                self.scarcity.scroll_by::<SidebarMessage>(dir)
            }
        }
    }

    pub fn view<'a>(
        &'a self,
        focus: FocusTarget,
        my_roster: &'a [RosterSlot],
        positional_scarcity: &'a [ScarcityEntry],
        nominated_position: Option<&'a str>,
    ) -> Element<'a, SidebarMessage> {
        let roster = self
            .roster
            .view(focus == FocusTarget::Roster, my_roster, nominated_position)
            .map(SidebarMessage::Roster);

        let scarcity = self
            .scarcity
            .view::<SidebarMessage>(
                focus == FocusTarget::Scarcity,
                positional_scarcity,
                nominated_position,
            );

        let plan = self
            .plan
            .view(focus == FocusTarget::NominationPlan)
            .map(SidebarMessage::Plan);

        v_stack(
            vec![roster, scarcity, plan],
            StackStyle {
                gap: StackGap::Xs,
                width: Length::Fill,
                height: Length::Fill,
                padding: Padding::new(4.0),
                ..Default::default()
            },
        )
        .into()
    }
}
