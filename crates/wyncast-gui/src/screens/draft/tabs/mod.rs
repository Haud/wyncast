pub mod analysis;
pub mod available;
pub mod draft_log;
pub mod teams;

use iced::Element;
use twui::{Tab, TabBarStyle, tab_bar};
use wyncast_app::protocol::TabId;

use super::DraftMessage;

pub fn view<'a>(active_tab: TabId) -> Element<'a, DraftMessage> {
    let tabs = vec![
        Tab::new("1: Analysis", DraftMessage::TabSelected(TabId::Analysis)),
        Tab::new("2: Available", DraftMessage::TabSelected(TabId::Available)),
        Tab::new("3: Draft Log", DraftMessage::TabSelected(TabId::DraftLog)),
        Tab::new("4: Teams", DraftMessage::TabSelected(TabId::Teams)),
    ];

    let selected = tab_id_to_index(active_tab);

    tab_bar(tabs, selected, TabBarStyle::default()).into()
}

/// Disabled tab bar shown when the draft is in the disconnected state.
pub fn view_disabled<'a>() -> Element<'a, DraftMessage> {
    let tabs = vec![
        Tab::new("1: Analysis", DraftMessage::TabSelected(TabId::Analysis)),
        Tab::new("2: Available", DraftMessage::TabSelected(TabId::Available)),
        Tab::new("3: Draft Log", DraftMessage::TabSelected(TabId::DraftLog)),
        Tab::new("4: Teams", DraftMessage::TabSelected(TabId::Teams)),
    ];

    tab_bar(tabs, 0, TabBarStyle::default().disabled(true)).into()
}

fn tab_id_to_index(tab: TabId) -> usize {
    match tab {
        TabId::Analysis => 0,
        TabId::Available => 1,
        TabId::DraftLog => 2,
        TabId::Teams => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_id_to_index_all_tabs() {
        assert_eq!(tab_id_to_index(TabId::Analysis), 0);
        assert_eq!(tab_id_to_index(TabId::Available), 1);
        assert_eq!(tab_id_to_index(TabId::DraftLog), 2);
        assert_eq!(tab_id_to_index(TabId::Teams), 3);
    }
}
