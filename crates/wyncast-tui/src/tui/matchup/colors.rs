use ratatui::style::Color;

use crate::matchup::CategoryState;

pub const HOME_WINNING_COLOR: Color = Color::Green;
pub const AWAY_WINNING_COLOR: Color = Color::Red;
pub const TIED_COLOR: Color = Color::Yellow;

pub fn state_color(state: CategoryState) -> Color {
    match state {
        CategoryState::HomeWinning => HOME_WINNING_COLOR,
        CategoryState::AwayWinning => AWAY_WINNING_COLOR,
        CategoryState::Tied => TIED_COLOR,
    }
}

pub fn home_away_colors(state: CategoryState) -> (Color, Color) {
    match state {
        CategoryState::HomeWinning => (HOME_WINNING_COLOR, AWAY_WINNING_COLOR),
        CategoryState::AwayWinning => (AWAY_WINNING_COLOR, HOME_WINNING_COLOR),
        CategoryState::Tied => (TIED_COLOR, TIED_COLOR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_color_maps_each_variant() {
        assert_eq!(state_color(CategoryState::HomeWinning), HOME_WINNING_COLOR);
        assert_eq!(state_color(CategoryState::AwayWinning), AWAY_WINNING_COLOR);
        assert_eq!(state_color(CategoryState::Tied), TIED_COLOR);
    }

    #[test]
    fn home_away_colors_swaps_for_away_winning() {
        let (h, a) = home_away_colors(CategoryState::HomeWinning);
        assert_eq!(h, HOME_WINNING_COLOR);
        assert_eq!(a, AWAY_WINNING_COLOR);

        let (h, a) = home_away_colors(CategoryState::AwayWinning);
        assert_eq!(h, AWAY_WINNING_COLOR);
        assert_eq!(a, HOME_WINNING_COLOR);
    }

    #[test]
    fn home_away_colors_tied_returns_same_color() {
        let (h, a) = home_away_colors(CategoryState::Tied);
        assert_eq!(h, a);
        assert_eq!(h, TIED_COLOR);
    }
}
