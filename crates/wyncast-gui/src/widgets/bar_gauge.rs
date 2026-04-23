use iced::{Element, Length, Padding};
use twui::{
    BoxStyle, Colors, StackGap, StackStyle, TextColor, TextSize, TextStyle, frame, h_stack, text,
    v_stack,
};

/// Layout style for [`bar_gauge`].
///
/// - `Compact` — single row: label left · gauge · count right (used in Scarcity list).
/// - `Stacked` — two rows: label + count header above the gauge (used by Budget bar, Phase 3.6).
///
/// Flagged for upstreaming to twui.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarGaugeStyle {
    Compact,
    #[allow(dead_code)]
    Stacked,
}

/// Horizontal progress gauge with a label and optional count.
///
/// `value / max` determines fill fraction, clamped to 0–1. `color` sets the fill
/// color. `style` controls whether the label is inline (`Compact`) or stacked above
/// the bar (`Stacked`).
///
/// Flagged for upstreaming to twui.
pub fn bar_gauge<'a, Message: Clone + 'a>(
    label: &str,
    value: f32,
    max: f32,
    color: Colors,
    style: BarGaugeStyle,
) -> Element<'a, Message> {
    match style {
        BarGaugeStyle::Compact => compact(label, value, max, color),
        BarGaugeStyle::Stacked => stacked(label, value, max, color),
    }
}

fn fill_frac(value: f32, max: f32) -> f32 {
    if max > 0.0 {
        (value / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn bar_elem<'a, Message: Clone + 'a>(value: f32, max: f32, color: Colors) -> Element<'a, Message> {
    let frac = fill_frac(value, max);
    let fill_color = color.rgb();
    let bg_color = Colors::Slate800.rgb();

    // Two-segment row: fill | background, using FillPortion to avoid
    // iced::widget::progress_bar which has API incompatibilities with the
    // composite renderer used when the "advanced" feature is enabled.
    let scale: u16 = 1000;
    let fill_w = ((frac * scale as f32).round() as u16).clamp(1, scale - 1);
    let bg_w = scale - fill_w;

    let fill_seg: Element<'a, Message> = iced::widget::container(
        iced::widget::Space::new(),
    )
    .width(Length::FillPortion(fill_w))
    .height(Length::Fixed(8.0))
    .style(move |_| iced::widget::container::Style {
        background: Some(iced::Background::Color(fill_color)),
        ..Default::default()
    })
    .into();

    let bg_seg: Element<'a, Message> = iced::widget::container(
        iced::widget::Space::new(),
    )
    .width(Length::FillPortion(bg_w))
    .height(Length::Fixed(8.0))
    .style(move |_| iced::widget::container::Style {
        background: Some(iced::Background::Color(bg_color)),
        ..Default::default()
    })
    .into();

    iced::widget::Row::new()
        .push(fill_seg)
        .push(bg_seg)
        .height(Length::Fixed(8.0))
        .into()
}

fn compact<'a, Message: Clone + 'a>(
    label: &str,
    value: f32,
    max: f32,
    color: Colors,
) -> Element<'a, Message> {
    let label_text: Element<Message> = text(
        label,
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into();
    let label_cell: Element<Message> = frame(
        label_text,
        BoxStyle {
            width: Length::Fixed(40.0),
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into();

    let count_str = format!("{}", value as u32);
    let count_text: Element<Message> = text(
        &count_str,
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();
    let count_cell: Element<Message> = frame(
        count_text,
        BoxStyle {
            width: Length::Fixed(24.0),
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into();

    let bar = bar_elem(value, max, color);

    h_stack(
        vec![label_cell, bar, count_cell],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}

fn stacked<'a, Message: Clone + 'a>(
    label: &str,
    value: f32,
    max: f32,
    color: Colors,
) -> Element<'a, Message> {
    let label_elem: Element<Message> = text(
        label,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into();

    let count_str = format!("{}", value as u32);
    let count_elem: Element<Message> = text(
        &count_str,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let header: Element<Message> = h_stack(
        vec![
            frame(label_elem, BoxStyle { width: Length::Fill, ..Default::default() }).into(),
            count_elem,
        ],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let bar = bar_elem(value, max, color);

    v_stack(
        vec![header, bar],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
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

    #[test]
    fn fill_frac_zero_max_returns_zero() {
        assert_eq!(fill_frac(5.0, 0.0), 0.0);
    }

    #[test]
    fn fill_frac_clamps_above_one() {
        assert_eq!(fill_frac(25.0, 10.0), 1.0);
    }

    #[test]
    fn fill_frac_clamps_below_zero() {
        assert_eq!(fill_frac(-5.0, 10.0), 0.0);
    }

    #[test]
    fn fill_frac_half() {
        let v = fill_frac(10.0, 20.0);
        assert!((v - 0.5).abs() < 1e-6);
    }

    #[test]
    fn fill_frac_full() {
        assert_eq!(fill_frac(20.0, 20.0), 1.0);
    }
}
