use twui::Colors;
use wyncast_baseball::matchup::CategoryState;

pub const HOME_COLOR: iced::Color = iced::Color {
    r: 0.392,
    g: 0.584,
    b: 0.929,
    a: 1.0,
};

pub const AWAY_COLOR: iced::Color = iced::Color {
    r: 1.0,
    g: 0.647,
    b: 0.0,
    a: 1.0,
};

pub fn tied_color() -> iced::Color {
    Colors::Warning.rgb()
}

pub fn state_color(state: &CategoryState) -> iced::Color {
    match state {
        CategoryState::HomeWinning => HOME_COLOR,
        CategoryState::AwayWinning => AWAY_COLOR,
        CategoryState::Tied => tied_color(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_color_maps_each_variant() {
        assert_eq!(state_color(&CategoryState::HomeWinning), HOME_COLOR);
        assert_eq!(state_color(&CategoryState::AwayWinning), AWAY_COLOR);
        assert_eq!(state_color(&CategoryState::Tied), tied_color());
    }

    #[test]
    fn home_away_separate_colors() {
        assert_ne!(HOME_COLOR, AWAY_COLOR);
    }
}
