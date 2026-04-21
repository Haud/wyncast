use iced::{Element, Length, Padding};
use twui::{Colors, TextColor, TextSize, TextStyle, frame, h_stack, text, BoxStyle, StackAlign, StackGap, StackStyle};

pub fn view<'a, Message: Clone + 'a>() -> Element<'a, Message> {
    let keys: &[(&str, &str)] = &[
        ("1-4", "Switch tab"),
        ("Tab", "Focus next"),
        ("Shift+Tab", "Focus prev"),
        ("q", "Quit"),
    ];

    let chips: Vec<Element<Message>> = keys
        .iter()
        .map(|(key, action)| {
            let key_text: Element<Message> = text(
                *key,
                TextStyle {
                    size: TextSize::Xs,
                    color: TextColor::Yellow,
                    ..Default::default()
                },
            )
            .into();
            let sep: Element<Message> = text(
                " ",
                TextStyle {
                    size: TextSize::Xs,
                    ..Default::default()
                },
            )
            .into();
            let action_text: Element<Message> = text(
                *action,
                TextStyle {
                    size: TextSize::Xs,
                    color: TextColor::Dimmed,
                    ..Default::default()
                },
            )
            .into();
            h_stack(
                vec![key_text, sep, action_text],
                StackStyle {
                    gap: StackGap::None,
                    align: StackAlign::Center,
                    ..Default::default()
                },
            )
            .into()
        })
        .collect();

    // Intersperse with separators
    let mut children: Vec<Element<Message>> = Vec::new();
    for (i, chip) in chips.into_iter().enumerate() {
        if i > 0 {
            children.push(
                text(
                    "  ·  ",
                    TextStyle {
                        size: TextSize::Xs,
                        color: TextColor::Dimmed,
                        ..Default::default()
                    },
                )
                .into(),
            );
        }
        children.push(chip);
    }

    let row: Element<Message> = h_stack(
        children,
        StackStyle {
            gap: StackGap::None,
            align: StackAlign::Center,
            width: Length::Fill,
            padding: Padding::new(4.0).left(8.0).right(8.0),
            ..Default::default()
        },
    )
    .into();

    frame(
        row,
        BoxStyle {
            width: Length::Fill,
            background: Some(Colors::Slate900),
            ..Default::default()
        },
    )
    .into()
}
