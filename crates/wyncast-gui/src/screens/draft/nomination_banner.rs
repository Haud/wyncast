use iced::{Element, Length, Padding};
use twui::{
    BoxStyle, Colors, StackAlign, StackGap, StackStyle, TextColor, TextSize, TextStyle,
    frame, h_stack, text, v_stack,
};
use wyncast_app::protocol::NominationInfo;
use wyncast_baseball::valuation::zscore::PlayerValuation;

/// Render the nomination banner.
///
/// When a nomination is active, shows player name + position chip, bid info,
/// and (if the player is found in the available pool) a dollar-value and
/// verdict badge. When idle, shows a dim placeholder.
pub fn view<'a, Message: Clone + 'a>(
    nomination: Option<&'a NominationInfo>,
    inflation_rate: f64,
    available_players: &'a [PlayerValuation],
) -> Element<'a, Message> {
    let content: Element<Message> = match nomination {
        Some(nom) => active_banner(nom, inflation_rate, available_players),
        None => idle_banner(),
    };

    frame(
        content,
        BoxStyle {
            width: Length::Fill,
            background: Some(Colors::BgElevated),
            padding: Padding::new(8.0).left(12.0).right(12.0),
            ..Default::default()
        },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Active nomination layout
// ---------------------------------------------------------------------------

fn active_banner<'a, Message: Clone + 'a>(
    nom: &'a NominationInfo,
    inflation_rate: f64,
    available_players: &'a [PlayerValuation],
) -> Element<'a, Message> {
    // Look up the player in the available pool to get their valuation.
    let valuation = available_players
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(&nom.player_name));

    // Row 1: player name + position chip
    let headline_row = headline_row::<Message>(&nom.player_name, &nom.position);

    // Row 2: bid info + optional values + verdict
    let details_row = details_row::<Message>(nom, valuation, inflation_rate);

    v_stack(
        vec![headline_row, details_row],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}

fn headline_row<'a, Message: Clone + 'a>(
    player_name: &str,
    position: &str,
) -> Element<'a, Message> {
    let name_elem: Element<Message> = text(
        player_name,
        TextStyle {
            size: TextSize::Xl2,
            weight: twui::TextWeight::Bold,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into();

    let pos_chip = position_chip::<Message>(position);

    h_stack(
        vec![name_elem, pos_chip],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}

fn details_row<'a, Message: Clone + 'a>(
    nom: &NominationInfo,
    valuation: Option<&PlayerValuation>,
    inflation_rate: f64,
) -> Element<'a, Message> {
    let mut items: Vec<Element<Message>> = Vec::new();

    // Nominated by
    items.push(
        text(
            format!("nom. by {}", nom.nominated_by),
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into(),
    );

    // Separator
    items.push(separator::<Message>());

    // Current bid
    let bid_label: Element<Message> = text(
        "Bid:",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();
    let bid_value: Element<Message> = text(
        format!("${}", nom.current_bid),
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Default,
            weight: twui::TextWeight::Semibold,
            ..Default::default()
        },
    )
    .into();
    items.push(
        h_stack(
            vec![bid_label, bid_value],
            StackStyle {
                gap: StackGap::Xs,
                align: StackAlign::Center,
                ..Default::default()
            },
        )
        .into(),
    );

    // Current bidder (if set)
    if let Some(ref bidder) = nom.current_bidder {
        items.push(separator::<Message>());
        items.push(
            text(
                format!("by {bidder}"),
                TextStyle {
                    size: TextSize::Sm,
                    color: TextColor::Dimmed,
                    ..Default::default()
                },
            )
            .into(),
        );
    }

    // Dollar value + adjusted value + verdict badge (when player is in pool)
    if let Some(pv) = valuation {
        let adjusted = pv.dollar_value * inflation_rate;

        items.push(separator::<Message>());

        let val_label: Element<Message> = text(
            "Val:",
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into();
        let val_value: Element<Message> = text(
            format!("${:.0}", pv.dollar_value),
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Default,
                ..Default::default()
            },
        )
        .into();
        items.push(
            h_stack(
                vec![val_label, val_value],
                StackStyle {
                    gap: StackGap::Xs,
                    align: StackAlign::Center,
                    ..Default::default()
                },
            )
            .into(),
        );

        items.push(separator::<Message>());

        let adj_label: Element<Message> = text(
            "Adj:",
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into();
        let adj_value: Element<Message> = text(
            format!("${:.0}", adjusted),
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Default,
                ..Default::default()
            },
        )
        .into();
        items.push(
            h_stack(
                vec![adj_label, adj_value],
                StackStyle {
                    gap: StackGap::Xs,
                    align: StackAlign::Center,
                    ..Default::default()
                },
            )
            .into(),
        );

        items.push(separator::<Message>());
        items.push(verdict_badge::<Message>(nom.current_bid, adjusted));
    }

    h_stack(
        items,
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn idle_banner<'a, Message: Clone + 'a>() -> Element<'a, Message> {
    text(
        "— no active nomination —",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into()
}

fn separator<'a, Message: Clone + 'a>() -> Element<'a, Message> {
    text(
        "·",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into()
}

fn position_chip<'a, Message: Clone + 'a>(position: &str) -> Element<'a, Message> {
    let bg = position_chip_color(position);
    let label: Element<Message> = text(
        position,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::White,
            weight: twui::TextWeight::Semibold,
            ..Default::default()
        },
    )
    .into();

    frame(
        label,
        BoxStyle {
            background: Some(bg),
            padding: Padding::new(3.0).left(8.0).right(8.0),
            ..Default::default()
        },
    )
    .into()
}

fn position_chip_color(position: &str) -> Colors {
    match position {
        "C" => Colors::Warning,
        "SP" | "RP" | "P" => Colors::Secondary,
        _ => Colors::Primary,
    }
}

/// Verdict badge based on current bid vs adjusted dollar value.
fn verdict_badge<'a, Message: Clone + 'a>(current_bid: u32, adjusted_value: f64) -> Element<'a, Message> {
    let bid = current_bid as f64;
    let (label, bg) = if adjusted_value <= 0.0 {
        ("PASS", Colors::Destructive)
    } else if bid <= adjusted_value * 0.90 {
        ("STRONG TARGET", Colors::Success)
    } else if bid <= adjusted_value * 1.05 {
        ("CONDITIONAL", Colors::Warning)
    } else {
        ("PASS", Colors::Destructive)
    };

    let label_elem: Element<Message> = text(
        label,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::White,
            weight: twui::TextWeight::Semibold,
            ..Default::default()
        },
    )
    .into();

    frame(
        label_elem,
        BoxStyle {
            background: Some(bg),
            padding: Padding::new(3.0).left(8.0).right(8.0),
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
    use std::collections::HashMap;
    use super::*;
    use wyncast_baseball::draft::pick::Position;
    use wyncast_baseball::valuation::zscore::{CategoryZScores, ProjectionData};
    use wyncast_core::stats::CategoryValues;

    fn make_nomination(bid: u32) -> NominationInfo {
        NominationInfo {
            player_name: "Mike Trout".to_string(),
            position: "CF".to_string(),
            nominated_by: "Team Alpha".to_string(),
            current_bid: bid,
            current_bidder: Some("Team Beta".to_string()),
            time_remaining: Some(30),
            eligible_slots: vec![],
        }
    }

    fn make_player(name: &str, dollar_value: f64) -> PlayerValuation {
        PlayerValuation {
            name: name.to_string(),
            team: "LAA".to_string(),
            positions: vec![Position::CenterField],
            is_pitcher: false,
            is_two_way: false,
            pitcher_type: None,
            projection: ProjectionData { values: HashMap::new() },
            total_zscore: 2.5,
            category_zscores: CategoryZScores::Hitter {
                zscores: CategoryValues::zeros(0),
                total: 0.0,
            },
            vor: 10.0,
            initial_vor: 10.0,
            best_position: Some(Position::CenterField),
            dollar_value,
        }
    }

    #[test]
    fn view_idle_does_not_panic() {
        let _elem: Element<String> = view(None, 1.0, &[]);
    }

    #[test]
    fn view_active_without_player_in_pool() {
        let nom = make_nomination(45);
        let _elem: Element<String> = view(Some(&nom), 1.0, &[]);
    }

    #[test]
    fn view_active_with_player_in_pool() {
        let nom = make_nomination(45);
        let players = [make_player("Mike Trout", 55.0)];
        let _elem: Element<String> = view(Some(&nom), 1.10, &players);
    }

    #[test]
    fn position_chip_color_catcher() {
        assert_eq!(position_chip_color("C"), Colors::Warning);
    }

    #[test]
    fn position_chip_color_pitcher() {
        assert_eq!(position_chip_color("SP"), Colors::Secondary);
        assert_eq!(position_chip_color("RP"), Colors::Secondary);
        assert_eq!(position_chip_color("P"), Colors::Secondary);
    }

    #[test]
    fn position_chip_color_hitter() {
        assert_eq!(position_chip_color("1B"), Colors::Primary);
        assert_eq!(position_chip_color("CF"), Colors::Primary);
        assert_eq!(position_chip_color("SS"), Colors::Primary);
    }

    #[test]
    fn verdict_badge_strong_target() {
        // bid $40 vs adjusted $50 → 80% of value → STRONG TARGET
        let _elem: Element<String> = verdict_badge(40, 50.0);
    }

    #[test]
    fn verdict_badge_conditional() {
        // bid $50 vs adjusted $50 → 100% of value → CONDITIONAL
        let _elem: Element<String> = verdict_badge(50, 50.0);
    }

    #[test]
    fn verdict_badge_pass() {
        // bid $60 vs adjusted $50 → 120% of value → PASS
        let _elem: Element<String> = verdict_badge(60, 50.0);
    }

    #[test]
    fn verdict_badge_zero_adjusted() {
        // adjusted $0 → PASS (guard against division)
        let _elem: Element<String> = verdict_badge(5, 0.0);
    }
}
