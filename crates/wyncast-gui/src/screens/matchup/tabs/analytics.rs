use iced::widget::scrollable::Scrollable;
use iced::widget::{container, row, Id as WidgetId};
use iced::{alignment, Background, Element, Length, Padding, Task};
use twui::{
    BoxStyle, Colors, StackGap, StackStyle, TextColor, TextSize, TextStyle, TextWeight,
    frame, text, v_stack,
};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::matchup::{CategoryScore, CategoryState};

use crate::widgets::data_table::ROW_HEIGHT;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum AnalyticsMessage {
    ScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

pub struct AnalyticsPanel {
    scroll_id: WidgetId,
}

impl AnalyticsPanel {
    pub fn new() -> Self {
        Self { scroll_id: WidgetId::unique() }
    }

    pub fn update(&mut self, msg: AnalyticsMessage) -> Task<AnalyticsMessage> {
        match msg {
            AnalyticsMessage::ScrollBy(dir) => {
                use iced::widget::operation::{self, AbsoluteOffset};
                let dy = match dir {
                    ScrollDirection::Up => -(ROW_HEIGHT * 3.0),
                    ScrollDirection::Down => ROW_HEIGHT * 3.0,
                    ScrollDirection::PageUp => -(ROW_HEIGHT * 10.0),
                    ScrollDirection::PageDown => ROW_HEIGHT * 10.0,
                };
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view(
        &self,
        category_scores: &[CategoryScore],
        days_elapsed: usize,
        total_days: usize,
    ) -> Element<'_, AnalyticsMessage> {
        let outlook = view_category_outlook(category_scores, days_elapsed, total_days);
        let close = view_close_categories(category_scores);
        let projections = view_pace_projections(category_scores, days_elapsed, total_days);

        let body: Element<'_, AnalyticsMessage> = v_stack(
            vec![outlook, close, projections],
            StackStyle {
                gap: StackGap::Xs,
                width: Length::Fill,
                padding: Padding::new(8.0),
                ..Default::default()
            },
        )
        .into();

        let scrollable: Element<'_, AnalyticsMessage> = Scrollable::new(body)
            .id(self.scroll_id.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        frame(
            scrollable,
            BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
        )
        .into()
    }
}

impl Default for AnalyticsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

fn section_header(title: &str) -> Element<'static, AnalyticsMessage> {
    v_stack(
        vec![
            container(iced::widget::Space::new())
                .width(Length::Fill)
                .height(Length::Fixed(12.0))
                .into(),
            text(
                title.to_string(),
                TextStyle {
                    size: TextSize::Sm,
                    color: TextColor::Dimmed,
                    weight: TextWeight::Semibold,
                    ..Default::default()
                },
            )
            .into(),
        ],
        StackStyle { gap: StackGap::Xs, width: Length::Fill, ..Default::default() },
    )
    .into()
}

fn dimmed_text(msg: &str) -> Element<'static, AnalyticsMessage> {
    text(
        msg.to_string(),
        TextStyle { size: TextSize::Xs, color: TextColor::Dimmed, ..Default::default() },
    )
    .into()
}

fn view_category_outlook(
    scores: &[CategoryScore],
    days_elapsed: usize,
    total_days: usize,
) -> Element<'static, AnalyticsMessage> {
    let header = section_header(&format!(
        "CATEGORY OUTLOOK (Day {days_elapsed} of {total_days})"
    ));

    if scores.is_empty() {
        return v_stack(
            vec![header, dimmed_text("No category data available.")],
            StackStyle { gap: StackGap::Xs, width: Length::Fill, ..Default::default() },
        )
        .into();
    }

    let home_winning: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| c.state == CategoryState::HomeWinning)
        .collect();
    let away_winning: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| c.state == CategoryState::AwayWinning)
        .collect();
    let tied: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| c.state == CategoryState::Tied)
        .collect();

    fn build_column(
        title: String,
        color: TextColor,
        cats: &[&CategoryScore],
    ) -> Element<'static, AnalyticsMessage> {
        let mut entries: Vec<Element<'static, AnalyticsMessage>> = Vec::with_capacity(cats.len() + 1);
        entries.push(
            container(text(
                title,
                TextStyle {
                    size: TextSize::Xs,
                    color,
                    weight: TextWeight::Semibold,
                    ..Default::default()
                },
            ))
            .padding(Padding::new(2.0))
            .into(),
        );
        for cat in cats {
            let diff = format_diff(cat);
            entries.push(
                container(text(
                    format!("{}  {diff}", cat.stat_abbrev),
                    TextStyle { size: TextSize::Xs, color, ..Default::default() },
                ))
                .padding(Padding::new(2.0))
                .into(),
            );
        }
        container(v_stack(
            entries,
            StackStyle { gap: StackGap::None, width: Length::Fill, ..Default::default() },
        ))
        .width(Length::FillPortion(1))
        .into()
    }

    let home_col = build_column(
        format!("HOME ({})", home_winning.len()),
        TextColor::Default,
        &home_winning,
    );
    let away_col = build_column(
        format!("AWAY ({})", away_winning.len()),
        TextColor::Error,
        &away_winning,
    );
    let tied_col = build_column(
        format!("TIED ({})", tied.len()),
        TextColor::Yellow,
        &tied,
    );

    let columns: Element<'static, AnalyticsMessage> =
        row(vec![home_col, away_col, tied_col]).width(Length::Fill).into();

    v_stack(
        vec![header, columns],
        StackStyle { gap: StackGap::Xs, width: Length::Fill, ..Default::default() },
    )
    .into()
}

fn view_close_categories(scores: &[CategoryScore]) -> Element<'static, AnalyticsMessage> {
    let header = section_header("CLOSE CATEGORIES (swingable)");

    let close: Vec<&CategoryScore> = scores
        .iter()
        .filter(|c| is_close_category(c))
        .collect();

    if close.is_empty() {
        return v_stack(
            vec![header, dimmed_text("No close categories.")],
            StackStyle { gap: StackGap::Xs, width: Length::Fill, ..Default::default() },
        )
        .into();
    }

    let col_widths: &[Length] = &[
        Length::Fixed(80.0),
        Length::Fixed(72.0),
        Length::Fixed(72.0),
        Length::Fixed(80.0),
        Length::Fill,
    ];
    let col_aligns: &[alignment::Horizontal] = &[
        alignment::Horizontal::Left,
        alignment::Horizontal::Right,
        alignment::Horizontal::Right,
        alignment::Horizontal::Right,
        alignment::Horizontal::Left,
    ];

    let table_hdr = table_header_row(
        &["Category", "Home", "Away", "H-A", "Status"],
        col_widths,
        col_aligns,
    );

    let mut rows: Vec<Element<'static, AnalyticsMessage>> = vec![header, table_hdr];

    for (idx, cat) in close.iter().enumerate() {
        let precision = stat_precision(&cat.stat_abbrev);
        let is_counting = !is_rate_stat(&cat.stat_abbrev);
        let lower_better = is_lower_is_better(&cat.stat_abbrev);

        let raw_diff = cat.home_value - cat.away_value;
        let effective_diff = if lower_better { -raw_diff } else { raw_diff };
        let color = state_text_color(&cat.state);

        rows.push(table_data_row(
            &[
                (&cat.stat_abbrev, TextColor::Default),
                (&format_value(cat.home_value, precision), TextColor::Default),
                (&format_value(cat.away_value, precision), TextColor::Default),
                (&format_signed_value(raw_diff, precision), color),
                (&build_close_status(cat, is_counting, effective_diff), color),
            ],
            col_widths,
            col_aligns,
            idx,
        ));
    }

    v_stack(
        rows,
        StackStyle { gap: StackGap::None, width: Length::Fill, ..Default::default() },
    )
    .into()
}

fn view_pace_projections(
    scores: &[CategoryScore],
    days_elapsed: usize,
    total_days: usize,
) -> Element<'static, AnalyticsMessage> {
    let header = section_header("PACE PROJECTIONS");

    if days_elapsed == 0 || total_days == 0 {
        return v_stack(
            vec![header, dimmed_text("No games played yet.")],
            StackStyle { gap: StackGap::Xs, width: Length::Fill, ..Default::default() },
        )
        .into();
    }

    if scores.is_empty() {
        return v_stack(
            vec![header, dimmed_text("No category data available.")],
            StackStyle { gap: StackGap::Xs, width: Length::Fill, ..Default::default() },
        )
        .into();
    }

    let subtitle = dimmed_text(&format!(
        "Based on {days_elapsed} day(s) played, projecting over {total_days}-day period:"
    ));

    let col_widths: &[Length] = &[
        Length::Fixed(80.0),
        Length::Fixed(80.0),
        Length::Fixed(88.0),
        Length::Fixed(88.0),
        Length::Fill,
    ];
    let col_aligns: &[alignment::Horizontal] = &[
        alignment::Horizontal::Left,
        alignment::Horizontal::Right,
        alignment::Horizontal::Right,
        alignment::Horizontal::Right,
        alignment::Horizontal::Left,
    ];

    let table_hdr = table_header_row(
        &["Category", "Home", "Home Proj", "Away Proj", "Proj Result"],
        col_widths,
        col_aligns,
    );

    let mut rows: Vec<Element<'static, AnalyticsMessage>> = vec![header, subtitle, table_hdr];

    for (idx, cat) in scores.iter().enumerate() {
        let precision = stat_precision(&cat.stat_abbrev);
        let lower_better = is_lower_is_better(&cat.stat_abbrev);

        let home_proj = project_stat(cat.home_value, days_elapsed, total_days, &cat.stat_abbrev);
        let away_proj = project_stat(cat.away_value, days_elapsed, total_days, &cat.stat_abbrev);

        let proj_diff = if lower_better {
            away_proj - home_proj
        } else {
            home_proj - away_proj
        };

        let (result_label, result_color) = if proj_diff > 0.001 {
            ("HOME", TextColor::Default)
        } else if proj_diff < -0.001 {
            ("AWAY", TextColor::Error)
        } else {
            ("TIE", TextColor::Yellow)
        };

        let raw_proj_diff = home_proj - away_proj;
        let diff_str = format_signed_value(raw_proj_diff, precision);
        let result_str = format!("{result_label} ({diff_str})");

        rows.push(table_data_row(
            &[
                (&cat.stat_abbrev, TextColor::Default),
                (&format_value(cat.home_value, precision), TextColor::Default),
                (&format_value(home_proj, precision), TextColor::Default),
                (&format_value(away_proj, precision), TextColor::Default),
                (&result_str, result_color),
            ],
            col_widths,
            col_aligns,
            idx,
        ));
    }

    v_stack(
        rows,
        StackStyle { gap: StackGap::None, width: Length::Fill, ..Default::default() },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Table helpers
// ---------------------------------------------------------------------------

fn table_header_row(
    labels: &[&str],
    widths: &[Length],
    aligns: &[alignment::Horizontal],
) -> Element<'static, AnalyticsMessage> {
    let cells: Vec<Element<'static, AnalyticsMessage>> = labels
        .iter()
        .zip(widths.iter().zip(aligns.iter()))
        .map(|(label, (width, align))| {
            container(text(
                label.to_string(),
                TextStyle {
                    size: TextSize::Xs,
                    color: TextColor::Dimmed,
                    weight: TextWeight::Semibold,
                    ..Default::default()
                },
            ))
            .width(*width)
            .height(Length::Fixed(ROW_HEIGHT))
            .align_x(*align)
            .align_y(alignment::Vertical::Center)
            .padding(Padding::new(0.0).left(6.0).right(6.0))
            .style(|_| iced::widget::container::Style {
                background: Some(Background::Color(Colors::Slate900.rgb())),
                ..Default::default()
            })
            .into()
        })
        .collect();

    row(cells).width(Length::Fill).into()
}

fn table_data_row(
    cells: &[(&str, TextColor)],
    widths: &[Length],
    aligns: &[alignment::Horizontal],
    row_index: usize,
) -> Element<'static, AnalyticsMessage> {
    let cell_elems: Vec<Element<'static, AnalyticsMessage>> = cells
        .iter()
        .zip(widths.iter().zip(aligns.iter()))
        .map(|((content, color), (width, align))| {
            container(text(
                content.to_string(),
                TextStyle { size: TextSize::Xs, color: *color, ..Default::default() },
            ))
            .width(*width)
            .height(Length::Fixed(ROW_HEIGHT))
            .align_x(*align)
            .align_y(alignment::Vertical::Center)
            .padding(Padding::new(0.0).left(6.0).right(6.0))
            .into()
        })
        .collect();

    let bg = if row_index % 2 == 1 {
        Colors::Slate800.rgb()
    } else {
        Colors::Slate900.rgb()
    };

    container(row(cell_elems).width(Length::Fill))
        .width(Length::Fill)
        .style(move |_| iced::widget::container::Style {
            background: Some(Background::Color(bg)),
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Computation helpers
// ---------------------------------------------------------------------------

fn state_text_color(state: &CategoryState) -> TextColor {
    match state {
        CategoryState::HomeWinning => TextColor::Default,
        CategoryState::AwayWinning => TextColor::Error,
        CategoryState::Tied => TextColor::Yellow,
    }
}

fn stat_precision(abbrev: &str) -> usize {
    match abbrev {
        "AVG" | "OBP" | "SLG" | "OPS" => 3,
        "ERA" | "WHIP" | "K/9" | "BB/9" | "K/BB" => 2,
        "IP" => 1,
        _ => 0,
    }
}

fn is_rate_stat(abbrev: &str) -> bool {
    matches!(
        abbrev,
        "AVG" | "OBP" | "SLG" | "OPS" | "ERA" | "WHIP" | "K/9" | "BB/9" | "K/BB"
    )
}

fn is_lower_is_better(abbrev: &str) -> bool {
    matches!(abbrev, "ERA" | "WHIP")
}

fn format_value(value: f64, precision: usize) -> String {
    if precision == 0 {
        format!("{}", value as i64)
    } else {
        format!("{value:.prec$}", prec = precision)
    }
}

fn format_signed_value(value: f64, precision: usize) -> String {
    if precision == 0 {
        let v = value as i64;
        if v >= 0 { format!("+{v}") } else { format!("{v}") }
    } else if value >= 0.0 {
        format!("+{value:.prec$}", prec = precision)
    } else {
        format!("{value:.prec$}", prec = precision)
    }
}

fn format_diff(cat: &CategoryScore) -> String {
    let precision = stat_precision(&cat.stat_abbrev);
    format_signed_value(cat.home_value - cat.away_value, precision)
}

fn is_close_category(cat: &CategoryScore) -> bool {
    let diff = (cat.home_value - cat.away_value).abs();
    match cat.stat_abbrev.as_str() {
        "R" | "RBI" | "BB" => diff <= 5.0,
        "HR" | "SB" | "W" | "SV" | "HD" => diff <= 3.0,
        "K" => diff <= 10.0,
        "AVG" => diff <= 0.020,
        "ERA" => diff <= 1.00,
        "WHIP" => diff <= 0.20,
        _ => false,
    }
}

fn project_stat(current: f64, days_elapsed: usize, total_days: usize, abbrev: &str) -> f64 {
    if days_elapsed == 0 {
        return 0.0;
    }
    if is_rate_stat(abbrev) {
        current
    } else {
        project_counting_stat(current, days_elapsed, total_days)
    }
}

fn project_counting_stat(current: f64, days_elapsed: usize, total_days: usize) -> f64 {
    if days_elapsed == 0 {
        return 0.0;
    }
    (current / days_elapsed as f64) * total_days as f64
}

fn build_close_status(cat: &CategoryScore, is_counting: bool, effective_diff: f64) -> String {
    match cat.state {
        CategoryState::HomeWinning => "HOME - lead is narrow".to_string(),
        CategoryState::AwayWinning => {
            if is_counting {
                let to_tie = effective_diff.abs().ceil() as i64;
                let to_lead = to_tie + 1;
                format!(
                    "AWAY - {} {} to tie, {} to lead",
                    to_tie, cat.stat_abbrev, to_lead
                )
            } else {
                "AWAY - gap is closeable".to_string()
            }
        }
        CategoryState::Tied => {
            if is_counting {
                "TIED - 1 to lead".to_string()
            } else {
                "TIED".to_string()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cat(abbrev: &str, home: f64, away: f64, state: CategoryState) -> CategoryScore {
        CategoryScore {
            stat_abbrev: abbrev.to_string(),
            home_value: home,
            away_value: away,
            state,
        }
    }

    #[test]
    fn format_value_counting() {
        assert_eq!(format_value(5.0, 0), "5");
        assert_eq!(format_value(42.0, 0), "42");
    }

    #[test]
    fn format_value_rate() {
        assert_eq!(format_value(0.275, 3), "0.275");
        assert_eq!(format_value(3.50, 2), "3.50");
    }

    #[test]
    fn format_signed_positive() {
        assert_eq!(format_signed_value(2.0, 0), "+2");
        assert_eq!(format_signed_value(0.015, 3), "+0.015");
    }

    #[test]
    fn format_signed_negative() {
        assert_eq!(format_signed_value(-1.0, 0), "-1");
        assert_eq!(format_signed_value(-0.70, 2), "-0.70");
    }

    #[test]
    fn format_signed_zero() {
        assert_eq!(format_signed_value(0.0, 0), "+0");
        assert_eq!(format_signed_value(0.0, 3), "+0.000");
    }

    #[test]
    fn close_category_counting_threshold() {
        assert!(is_close_category(&make_cat("HR", 2.0, 4.0, CategoryState::AwayWinning)));
        assert!(!is_close_category(&make_cat("HR", 2.0, 10.0, CategoryState::AwayWinning)));
    }

    #[test]
    fn close_category_rate_threshold() {
        assert!(is_close_category(&make_cat("ERA", 3.50, 4.00, CategoryState::HomeWinning)));
        assert!(!is_close_category(&make_cat("ERA", 2.00, 5.50, CategoryState::HomeWinning)));
    }

    #[test]
    fn projection_counting_linear() {
        let result = project_counting_stat(5.0, 2, 12);
        assert!((result - 30.0).abs() < 1e-10);
    }

    #[test]
    fn projection_zero_elapsed() {
        assert!((project_counting_stat(5.0, 0, 12)).abs() < 1e-10);
        assert!((project_stat(5.0, 0, 12, "HR")).abs() < 1e-10);
    }

    #[test]
    fn projection_rate_preserves_current() {
        assert!((project_stat(3.50, 2, 12, "ERA") - 3.50).abs() < 1e-10);
        assert!((project_stat(0.275, 3, 12, "AVG") - 0.275).abs() < 1e-10);
    }

    #[test]
    fn projection_counting_with_abbrev() {
        assert!((project_stat(2.0, 2, 10, "HR") - 10.0).abs() < 1e-10);
    }

    #[test]
    fn stat_precision_known() {
        assert_eq!(stat_precision("AVG"), 3);
        assert_eq!(stat_precision("ERA"), 2);
        assert_eq!(stat_precision("IP"), 1);
        assert_eq!(stat_precision("HR"), 0);
        assert_eq!(stat_precision("R"), 0);
    }

    #[test]
    fn rate_stat_detection() {
        assert!(is_rate_stat("AVG"));
        assert!(is_rate_stat("ERA"));
        assert!(is_rate_stat("WHIP"));
        assert!(!is_rate_stat("HR"));
        assert!(!is_rate_stat("R"));
    }

    #[test]
    fn lower_is_better_detection() {
        assert!(is_lower_is_better("ERA"));
        assert!(is_lower_is_better("WHIP"));
        assert!(!is_lower_is_better("HR"));
        assert!(!is_lower_is_better("AVG"));
    }
}
