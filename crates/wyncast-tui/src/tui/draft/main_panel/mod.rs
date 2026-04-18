pub mod analysis;
pub mod available;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::draft::pick::DraftPick;
use crate::protocol::TabId;
use crate::tui::TeamSummary;
use crate::tui::action::Action;
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;
use crate::valuation::zscore::PlayerValuation;

use analysis::{AnalysisPanel, AnalysisPanelMessage};
use available::{AvailablePanel, AvailablePanelMessage};
use super::draft_log::{DraftLogPanel, DraftLogMessage};
use super::teams::{TeamsPanel, TeamsMessage};

/// Messages handled by the MainPanel component.
#[derive(Debug, Clone)]
pub enum MainPanelMessage {
    SwitchTab(TabId),
    Analysis(AnalysisPanelMessage),
    Available(AvailablePanelMessage),
    DraftLog(DraftLogMessage),
    Teams(TeamsMessage),
}

/// Mid-level component that composes the four tab panels and owns tab state.
pub struct MainPanel {
    active_tab: TabId,
    pub analysis: AnalysisPanel,
    pub available: AvailablePanel,
    pub draft_log: DraftLogPanel,
    pub teams: TeamsPanel,
}

impl MainPanel {
    pub fn new() -> Self {
        Self {
            active_tab: TabId::Analysis,
            analysis: AnalysisPanel::new(),
            available: AvailablePanel::new(),
            draft_log: DraftLogPanel::new(),
            teams: TeamsPanel::new(),
        }
    }

    /// The currently active tab.
    pub fn active_tab(&self) -> TabId {
        self.active_tab
    }

    /// Declare keybindings for the subscription system.
    ///
    /// Only the active tab's subscription is returned — inactive panels are
    /// not visible and don't need to listen for events.
    pub fn subscription(&self, kb: &mut KeybindManager) -> Subscription<MainPanelMessage> {
        match self.active_tab {
            TabId::Available => self
                .available
                .subscription(kb)
                .map(MainPanelMessage::Available),
            // Other tabs have no subscriptions yet.
            TabId::Analysis | TabId::DraftLog | TabId::Teams => Subscription::none(),
        }
    }

    pub fn update(&mut self, msg: MainPanelMessage) -> Option<Action> {
        match msg {
            MainPanelMessage::SwitchTab(tab) => {
                self.active_tab = tab;
                None
            }
            MainPanelMessage::Analysis(m) => self.analysis.update(m),
            MainPanelMessage::Available(m) => self.available.update(m),
            MainPanelMessage::DraftLog(m) => self.draft_log.update(m),
            MainPanelMessage::Teams(m) => self.teams.update(m),
        }
    }

    /// Render the active tab's content into the given area.
    #[allow(clippy::too_many_arguments)]
    pub fn view(
        &self,
        frame: &mut Frame,
        area: Rect,
        available_players: &[PlayerValuation],
        nominated_name: Option<&str>,
        draft_log: &[DraftPick],
        team_summaries: &[TeamSummary],
        focused: bool,
    ) {
        match self.active_tab {
            TabId::Analysis => self.analysis.view(frame, area, focused),
            TabId::Available => {
                self.available.view(frame, area, available_players, nominated_name, focused);
            }
            TabId::DraftLog => {
                self.draft_log.view(frame, area, draft_log, available_players, focused);
            }
            TabId::Teams => {
                self.teams.view(frame, area, team_summaries, focused);
            }
        }
    }
}

impl Default for MainPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TabId;
    use crate::tui::llm_stream::LlmStreamMessage;
    use crate::tui::scroll::ScrollDirection;

    #[test]
    fn new_starts_on_analysis_tab() {
        let panel = MainPanel::new();
        assert_eq!(panel.active_tab(), TabId::Analysis);
    }

    #[test]
    fn default_starts_on_analysis_tab() {
        let panel = MainPanel::default();
        assert_eq!(panel.active_tab(), TabId::Analysis);
    }

    #[test]
    fn switch_tab_updates_active_tab() {
        let mut panel = MainPanel::new();
        let result = panel.update(MainPanelMessage::SwitchTab(TabId::Teams));
        assert_eq!(result, None);
        assert_eq!(panel.active_tab(), TabId::Teams);
    }

    #[test]
    fn switch_tab_returns_none() {
        let mut panel = MainPanel::new();
        assert!(panel.update(MainPanelMessage::SwitchTab(TabId::Available)).is_none());
    }

    #[test]
    fn analysis_message_delegates() {
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::Analysis(
            AnalysisPanelMessage::Stream(LlmStreamMessage::TokenReceived("hello".into())),
        ));
        assert_eq!(panel.analysis.text(), "hello");
    }

    #[test]
    fn available_message_delegates() {
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::Available(
            AvailablePanelMessage::Scroll(ScrollDirection::Down),
        ));
        assert_eq!(panel.available.scroll_offset(), 1);
    }

    #[test]
    fn draft_log_message_delegates() {
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::DraftLog(
            DraftLogMessage::Scroll(ScrollDirection::Down),
        ));
        // DraftLogPanel scroll changes offset
    }

    #[test]
    fn teams_message_delegates() {
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::Teams(
            TeamsMessage::Scroll(ScrollDirection::Down),
        ));
        // TeamsPanel scroll changes offset
    }

    #[test]
    fn view_does_not_panic_analysis() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let panel = MainPanel::new();
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, &[], &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_available() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, &[], &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_draft_log() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::SwitchTab(TabId::DraftLog));
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, &[], &[], false))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_teams() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut panel = MainPanel::new();
        panel.update(MainPanelMessage::SwitchTab(TabId::Teams));
        terminal
            .draw(|frame| panel.view(frame, frame.area(), &[], None, &[], &[], false))
            .unwrap();
    }
}
