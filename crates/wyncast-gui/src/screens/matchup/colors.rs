use twui::{Colors, TextColor};
use wyncast_baseball::matchup::CategoryState;

pub const HOME_BAR_COLOR: Colors = Colors::Success;
pub const AWAY_BAR_COLOR: Colors = Colors::Destructive;
pub const TIED_BAR_COLOR: Colors = Colors::Warning;

pub const HOME_TEXT_COLOR: TextColor = TextColor::Default;
pub const AWAY_TEXT_COLOR: TextColor = TextColor::Error;
pub const TIED_TEXT_COLOR: TextColor = TextColor::Yellow;

pub fn state_text_color(state: &CategoryState) -> TextColor {
    match state {
        CategoryState::HomeWinning => HOME_TEXT_COLOR,
        CategoryState::AwayWinning => AWAY_TEXT_COLOR,
        CategoryState::Tied => TIED_TEXT_COLOR,
    }
}

pub fn bar_colors(state: &CategoryState) -> (Colors, Colors) {
    match state {
        CategoryState::HomeWinning => (HOME_BAR_COLOR, AWAY_BAR_COLOR),
        CategoryState::AwayWinning => (AWAY_BAR_COLOR, HOME_BAR_COLOR),
        CategoryState::Tied => (TIED_BAR_COLOR, TIED_BAR_COLOR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_text_color_maps_each_variant() {
        assert_eq!(state_text_color(&CategoryState::HomeWinning), HOME_TEXT_COLOR);
        assert_eq!(state_text_color(&CategoryState::AwayWinning), AWAY_TEXT_COLOR);
        assert_eq!(state_text_color(&CategoryState::Tied), TIED_TEXT_COLOR);
    }

    #[test]
    fn bar_colors_swaps_for_away_winning() {
        let (h, a) = bar_colors(&CategoryState::HomeWinning);
        assert!(matches!(h, HOME_BAR_COLOR));
        assert!(matches!(a, AWAY_BAR_COLOR));

        let (h, a) = bar_colors(&CategoryState::AwayWinning);
        assert!(matches!(h, AWAY_BAR_COLOR));
        assert!(matches!(a, HOME_BAR_COLOR));
    }

    #[test]
    fn bar_colors_tied_returns_same_color() {
        let (h, a) = bar_colors(&CategoryState::Tied);
        assert!(matches!(h, TIED_BAR_COLOR));
        assert!(matches!(a, TIED_BAR_COLOR));
    }
}
