use iced::{Element, Length, Padding};
use twui::{
    BoxStyle, Colors, StackAlign, StackGap, StackStyle, TextColor, TextSize, TextStyle, TextWeight,
    frame, h_stack, text, v_stack,
};
use wyncast_baseball::matchup::{CategoryScore, CategoryState, MatchupInfo, TeamMatchupState};

use super::MatchupMessage;

/// Scoreboard band: home team card | category summary | away team card.
pub fn view<'a>(
    home_team: &'a TeamMatchupState,
    away_team: &'a TeamMatchupState,
    info: &'a MatchupInfo,
    category_scores: &'a [CategoryScore],
) -> Element<'a, MatchupMessage> {
    let home_card = team_card(home_team);
    let away_card = team_card(away_team);
    let summary = category_summary(category_scores);

    let inner: Element<MatchupMessage> = h_stack(
        vec![away_card, summary, home_card],
        StackStyle {
            gap: StackGap::Md,
            align: StackAlign::Center,
            width: Length::Fill,
            padding: Padding::new(8.0),
            ..Default::default()
        },
    )
    .into();

    let period_label = format!(
        "Matchup Period {} · {} – {}",
        info.matchup_period, info.start_date, info.end_date
    );
    let period_text: Element<MatchupMessage> = text(
        period_label,
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let outer: Element<MatchupMessage> = v_stack(
        vec![period_text, inner],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            padding: Padding::new(4.0).left(8.0).right(8.0),
            background: Some(Colors::Slate800),
            ..Default::default()
        },
    )
    .into();

    outer
}

fn team_card<'a>(team: &'a TeamMatchupState) -> Element<'a, MatchupMessage> {
    let name: Element<MatchupMessage> = text(
        team.name.clone(),
        TextStyle {
            size: TextSize::Md,
            weight: TextWeight::Bold,
            ..Default::default()
        },
    )
    .into();

    let record: Element<MatchupMessage> = text(
        format!("Season: {}", team.record),
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let score: Element<MatchupMessage> = text(
        format!("Matchup: {}", team.category_score),
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into();

    let card_content: Element<MatchupMessage> = v_stack(
        vec![name, record, score],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    frame(
        card_content,
        BoxStyle {
            width: Length::FillPortion(2),
            padding: Padding::new(8.0),
            background: Some(Colors::Slate900),
            ..Default::default()
        },
    )
    .into()
}

fn category_summary<'a>(scores: &'a [CategoryScore]) -> Element<'a, MatchupMessage> {
    let home_wins = scores.iter().filter(|s| s.state == CategoryState::HomeWinning).count();
    let away_wins = scores.iter().filter(|s| s.state == CategoryState::AwayWinning).count();
    let tied = scores.iter().filter(|s| s.state == CategoryState::Tied).count();

    let summary_str = format!("{home_wins} – {away_wins} – {tied}");

    let label: Element<MatchupMessage> = text(
        "W – L – T",
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let score: Element<MatchupMessage> = text(
        summary_str,
        TextStyle {
            size: TextSize::Xl2,
            weight: TextWeight::Bold,
            ..Default::default()
        },
    )
    .into();

    let inner: Element<MatchupMessage> = v_stack(
        vec![score, label],
        StackStyle {
            gap: StackGap::Xs,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    frame(
        inner,
        BoxStyle {
            width: Length::FillPortion(1),
            padding: Padding::new(8.0),
            ..Default::default()
        },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wyncast_baseball::matchup::TeamRecord;

    fn make_team(name: &str, w: u16, l: u16, t: u16) -> TeamMatchupState {
        TeamMatchupState {
            name: name.to_string(),
            abbrev: name[..2].to_uppercase(),
            record: TeamRecord { wins: w, losses: l, ties: t },
            category_score: TeamRecord { wins: w, losses: l, ties: t },
        }
    }

    fn make_info() -> MatchupInfo {
        use wyncast_baseball::matchup::MatchupInfo;
        MatchupInfo {
            matchup_period: 1,
            start_date: "2026-03-25".to_string(),
            end_date: "2026-04-05".to_string(),
            home_team_name: "Home Team".to_string(),
            away_team_name: "Away Team".to_string(),
            home_record: TeamRecord { wins: 1, losses: 0, ties: 0 },
            away_record: TeamRecord { wins: 0, losses: 1, ties: 0 },
        }
    }

    #[test]
    fn category_summary_wins_counted() {
        let scores = vec![
            CategoryScore { stat_abbrev: "R".to_string(), home_value: 10.0, away_value: 5.0, state: CategoryState::HomeWinning },
            CategoryScore { stat_abbrev: "HR".to_string(), home_value: 2.0, away_value: 4.0, state: CategoryState::AwayWinning },
            CategoryScore { stat_abbrev: "SB".to_string(), home_value: 3.0, away_value: 3.0, state: CategoryState::Tied },
        ];
        // 1 home win, 1 away win, 1 tie
        let home_wins = scores.iter().filter(|s| s.state == CategoryState::HomeWinning).count();
        let away_wins = scores.iter().filter(|s| s.state == CategoryState::AwayWinning).count();
        let tied = scores.iter().filter(|s| s.state == CategoryState::Tied).count();
        assert_eq!(home_wins, 1);
        assert_eq!(away_wins, 1);
        assert_eq!(tied, 1);
    }

    #[test]
    fn view_does_not_panic_empty_scores() {
        let home = make_team("Home Team", 1, 0, 0);
        let away = make_team("Away Team", 0, 1, 0);
        let info = make_info();
        let _elem = view(&home, &away, &info, &[]);
    }
}
