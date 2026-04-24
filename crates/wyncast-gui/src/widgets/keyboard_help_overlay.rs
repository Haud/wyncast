/// Keyboard shortcut help overlay.
///
/// Renders a centered modal listing keybindings grouped by section.
/// Triggered by `?`; dismissed by Esc or backdrop click.
///
/// Flagged for upstreaming to twui as `keyboard_help_overlay`.
use iced::widget::{container, mouse_area, row, stack, text as itext};
use iced::{Border, Color, Element, Length, Padding, alignment};
use twui::{
    Colors, StackGap, StackStyle, TextColor, TextSize, TextStyle, hdivider, section_header, text,
    v_stack,
};

/// A section of related keybindings.
pub struct KeySection {
    pub title: &'static str,
    pub bindings: Vec<(&'static str, &'static str)>,
}

impl KeySection {
    pub fn new(title: &'static str, bindings: Vec<(&'static str, &'static str)>) -> Self {
        Self { title, bindings }
    }
}

/// Renders the keyboard help overlay on top of the current screen.
///
/// `sections` is a list of grouped keybinding sections (consumed here).
/// `on_dismiss` fires when the user clicks the backdrop.
pub fn keyboard_help_overlay<'a, Message: Clone + 'a>(
    sections: Vec<KeySection>,
    on_dismiss: Message,
) -> Element<'a, Message> {
    let panel = build_panel(sections);

    let backdrop_color = Color::from_rgba(0.0, 0.0, 0.0, 0.6);
    let backdrop: Element<Message> = mouse_area(
        container(iced::widget::Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(backdrop_color)),
                ..Default::default()
            }),
    )
    .on_press(on_dismiss)
    .into();

    let centered: Element<Message> = container(panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .into();

    stack![backdrop, centered].into()
}

fn build_panel<'a, Message: Clone + 'a>(sections: Vec<KeySection>) -> Element<'a, Message> {
    let bg = Colors::BgElevated.rgb();
    let border_color = Colors::BorderDefault.rgb();

    let mut rows: Vec<Element<Message>> = Vec::new();

    for (i, section) in sections.into_iter().enumerate() {
        if i > 0 {
            rows.push(hdivider::<Message>().into());
        }
        rows.push(section_header(section.title).into());
        for (key, action) in section.bindings {
            rows.push(binding_row::<Message>(key, action));
        }
    }

    let content = v_stack(
        rows,
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fixed(480.0),
            padding: Padding::new(16.0),
            ..Default::default()
        },
    );

    container(content)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: Border {
                color: border_color,
                width: 1.0,
                radius: iced::border::Radius::new(8.0),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
                offset: iced::Vector::new(0.0, 8.0),
                blur_radius: 24.0,
            },
            ..Default::default()
        })
        .into()
}

fn binding_row<'a, Message: Clone + 'a>(key: &'a str, action: &'a str) -> Element<'a, Message> {
    let chip_bg = Colors::Slate800.rgb();
    let chip_border = Colors::Slate600.rgb();

    let key_label: Element<Message> = itext(key)
        .size(12)
        .color(Colors::Warning.rgb())
        .into();

    let chip: Element<Message> = container(key_label)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(chip_bg)),
            border: Border {
                color: chip_border,
                width: 1.0,
                radius: iced::border::Radius::new(3.0),
            },
            ..Default::default()
        })
        .padding(Padding::new(2.0).left(6.0).right(6.0))
        .into();

    let action_text: Element<Message> = text(
        action,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into();

    let r: Element<Message> = row![chip, action_text]
        .spacing(8.0)
        .align_y(alignment::Vertical::Center)
        .width(Length::Fill)
        .into();

    container(r).padding(Padding::new(2.0)).width(Length::Fill).into()
}

/// Returns the keybindings for the Draft screen help overlay.
pub fn draft_sections() -> Vec<KeySection> {
    vec![
        KeySection::new(
            "Navigation",
            vec![
                ("1 / 2 / 3 / 4", "Switch tab"),
                ("Tab", "Focus next panel"),
                ("Shift+Tab", "Focus prev panel"),
            ],
        ),
        KeySection::new(
            "Scrolling",
            vec![
                ("↑ / k", "Scroll up"),
                ("↓ / j", "Scroll down"),
                ("PgUp / PgDn", "Scroll page"),
            ],
        ),
        KeySection::new(
            "Available Players",
            vec![
                ("/", "Open text filter"),
                ("p", "Position filter modal"),
                ("Esc", "Close filter / modal"),
            ],
        ),
        KeySection::new(
            "Global",
            vec![
                ("q", "Quit (with confirm)"),
                (",", "Open settings"),
                ("r", "Resync from backend"),
                ("?", "Show / hide this help"),
            ],
        ),
    ]
}

/// Returns the keybindings for the Matchup screen help overlay.
pub fn matchup_sections() -> Vec<KeySection> {
    vec![
        KeySection::new(
            "Navigation",
            vec![
                ("1 / 2 / 3 / 4", "Switch tab"),
                ("Tab", "Toggle focus (main / tracker)"),
                ("← / →", "Previous / next day (Daily Stats)"),
            ],
        ),
        KeySection::new(
            "Scrolling",
            vec![
                ("↑ / k", "Scroll up"),
                ("↓ / j", "Scroll down"),
                ("PgUp / PgDn", "Scroll page"),
            ],
        ),
        KeySection::new(
            "Global",
            vec![
                ("q", "Quit (with confirm)"),
                ("?", "Show / hide this help"),
            ],
        ),
    ]
}

/// Returns the keybindings for the Settings screen help overlay.
pub fn settings_sections() -> Vec<KeySection> {
    vec![
        KeySection::new(
            "Navigation",
            vec![
                ("Tab / Shift+Tab", "Next / previous section"),
            ],
        ),
        KeySection::new(
            "Actions",
            vec![
                ("Esc / ,", "Cancel and exit settings"),
                ("?", "Show / hide this help"),
            ],
        ),
    ]
}

/// Returns the keybindings for the Onboarding screen help overlay.
pub fn onboarding_sections() -> Vec<KeySection> {
    vec![KeySection::new(
        "Wizard",
        vec![
            ("Enter", "Next step"),
            ("Esc", "Previous step"),
            ("Tab", "Cycle form fields"),
        ],
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_sections_non_empty() {
        let sections = draft_sections();
        assert!(!sections.is_empty());
        for s in &sections {
            assert!(!s.bindings.is_empty(), "section '{}' has no bindings", s.title);
        }
    }

    #[test]
    fn matchup_sections_non_empty() {
        assert!(!matchup_sections().is_empty());
    }

    #[test]
    fn key_section_new() {
        let s = KeySection::new("Test", vec![("?", "help")]);
        assert_eq!(s.title, "Test");
        assert_eq!(s.bindings.len(), 1);
    }
}
