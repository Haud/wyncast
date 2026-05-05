use ratatui::style::Color;

use crate::matchup::CategoryState;

pub const HOME_COLOR: Color = Color::LightBlue;
pub const AWAY_COLOR: Color = Color::Rgb(255, 165, 0);
pub const TIED_COLOR: Color = Color::Yellow;

pub fn state_color(state: CategoryState) -> Color {
    match state {
        CategoryState::HomeWinning => HOME_COLOR,
        CategoryState::AwayWinning => AWAY_COLOR,
        CategoryState::Tied => TIED_COLOR,
    }
}

pub fn home_away_colors(state: CategoryState) -> (Color, Color) {
    match state {
        CategoryState::HomeWinning => (HOME_COLOR, AWAY_COLOR),
        CategoryState::AwayWinning => (AWAY_COLOR, HOME_COLOR),
        CategoryState::Tied => (TIED_COLOR, TIED_COLOR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_color_maps_each_variant() {
        assert_eq!(state_color(CategoryState::HomeWinning), HOME_COLOR);
        assert_eq!(state_color(CategoryState::AwayWinning), AWAY_COLOR);
        assert_eq!(state_color(CategoryState::Tied), TIED_COLOR);
    }

    #[test]
    fn home_away_colors_swaps_for_away_winning() {
        let (h, a) = home_away_colors(CategoryState::HomeWinning);
        assert_eq!(h, HOME_COLOR);
        assert_eq!(a, AWAY_COLOR);

        let (h, a) = home_away_colors(CategoryState::AwayWinning);
        assert_eq!(h, AWAY_COLOR);
        assert_eq!(a, HOME_COLOR);
    }

    #[test]
    fn home_away_colors_tied_returns_same_color() {
        let (h, a) = home_away_colors(CategoryState::Tied);
        assert_eq!(h, a);
        assert_eq!(h, TIED_COLOR);
    }
}
