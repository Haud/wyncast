use iced::widget::{Id, column, container, mouse_area, row, scrollable};
use iced::{alignment, Background, Element, Length, Padding};
use twui::{Colors, Opacity, TextAlign, TextColor, TextSize, TextStyle, TextWeight, text};

pub const ROW_HEIGHT: f32 = 36.0;

#[derive(Debug, Clone)]
pub struct Column {
    pub label: String,
    pub width: Length,
    pub align: TextAlign,
}

impl Column {
    pub fn new(label: impl Into<String>, width: Length, align: TextAlign) -> Self {
        Self { label: label.into(), width, align }
    }
}

#[derive(Debug, Clone)]
pub struct DataTableStyle {
    pub alternate_rows: bool,
}

impl Default for DataTableStyle {
    fn default() -> Self {
        Self { alternate_rows: true }
    }
}

/// Fixed-header, scrollable-body data table.
///
/// `columns` specifies header labels and cell widths.
/// `rows` is a vec of rows, each row a vec of cell elements (same order as columns).
/// `scroll_id` is used for programmatic scroll via `iced::widget::operation`.
/// `highlighted_index` gives the row (0-based) a yellow highlight background.
/// `on_row_click` — optional; each row becomes a clickable mouse_area when provided.
///
/// Flagged for upstream to twui (referenced in Phase 3.4 and 3.9).
pub fn data_table<'a, Message: Clone + 'a>(
    columns: Vec<Column>,
    rows: Vec<Vec<Element<'a, Message>>>,
    scroll_id: Id,
    highlighted_index: Option<usize>,
    style: DataTableStyle,
    on_row_click: Option<Box<dyn Fn(usize) -> Message + 'a>>,
) -> Element<'a, Message> {
    let header = build_header(&columns);
    let body = build_body(&columns, rows, highlighted_index, style, on_row_click);

    let scrollable_body: Element<'a, Message> = scrollable::Scrollable::new(body)
        .id(scroll_id)
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

    column![header, scrollable_body]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn build_header<'a, Message: Clone + 'a>(columns: &[Column]) -> Element<'a, Message> {
    let cells: Vec<Element<'a, Message>> = columns
        .iter()
        .map(|col| {
            let label: Element<'a, Message> = text(
                col.label.clone(),
                TextStyle {
                    size: TextSize::Xs,
                    color: TextColor::Dimmed,
                    weight: TextWeight::Semibold,
                    ..Default::default()
                },
            )
            .into();

            container(label)
                .width(col.width)
                .height(Length::Fixed(ROW_HEIGHT))
                .align_x(to_horiz(col.align))
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

fn build_body<'a, Message: Clone + 'a>(
    columns: &[Column],
    rows: Vec<Vec<Element<'a, Message>>>,
    highlighted_index: Option<usize>,
    style: DataTableStyle,
    on_row_click: Option<Box<dyn Fn(usize) -> Message + 'a>>,
) -> Element<'a, Message> {
    let row_elements: Vec<Element<'a, Message>> = rows
        .into_iter()
        .enumerate()
        .map(|(idx, cells)| {
            let bg = row_bg(idx, highlighted_index, style.alternate_rows);

            let row_cells: Vec<Element<'a, Message>> = cells
                .into_iter()
                .zip(columns.iter())
                .map(|(cell, col)| {
                    container(cell)
                        .width(col.width)
                        .height(Length::Fixed(ROW_HEIGHT))
                        .align_x(to_horiz(col.align))
                        .align_y(alignment::Vertical::Center)
                        .padding(Padding::new(0.0).left(6.0).right(6.0))
                        .into()
                })
                .collect();

            let row_content: Element<'a, Message> =
                row(row_cells).width(Length::Fill).into();

            let styled: Element<'a, Message> = container(row_content)
                .width(Length::Fill)
                .style(move |_| iced::widget::container::Style {
                    background: Some(Background::Color(bg)),
                    ..Default::default()
                })
                .into();

            if let Some(ref f) = on_row_click {
                mouse_area(styled).on_press(f(idx)).into()
            } else {
                styled
            }
        })
        .collect();

    column(row_elements).width(Length::Fill).into()
}

fn row_bg(idx: usize, highlighted: Option<usize>, alternate: bool) -> iced::Color {
    if highlighted == Some(idx) {
        Colors::Warning.rgba(Opacity::O30)
    } else if alternate && idx % 2 == 1 {
        Colors::Slate800.rgb()
    } else {
        Colors::Slate900.rgb()
    }
}

fn to_horiz(align: TextAlign) -> alignment::Horizontal {
    match align {
        TextAlign::Right => alignment::Horizontal::Right,
        TextAlign::Center => alignment::Horizontal::Center,
        _ => alignment::Horizontal::Left,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_col(label: &str, width: Length, align: TextAlign) -> Column {
        Column::new(label, width, align)
    }

    #[test]
    fn row_bg_highlighted_is_warning() {
        let bg = row_bg(2, Some(2), true);
        let expected = Colors::Warning.rgba(Opacity::O30);
        assert_eq!(bg, expected);
    }

    #[test]
    fn row_bg_alternating_odd() {
        let bg = row_bg(1, None, true);
        assert_eq!(bg, Colors::Slate800.rgb());
    }

    #[test]
    fn row_bg_alternating_even() {
        let bg = row_bg(0, None, true);
        assert_eq!(bg, Colors::Slate900.rgb());
    }

    #[test]
    fn row_bg_no_alternate() {
        let bg = row_bg(1, None, false);
        assert_eq!(bg, Colors::Slate900.rgb());
    }

    #[test]
    fn to_horiz_right() {
        assert_eq!(to_horiz(TextAlign::Right), alignment::Horizontal::Right);
    }

    #[test]
    fn to_horiz_left() {
        assert_eq!(to_horiz(TextAlign::Left), alignment::Horizontal::Left);
    }

    #[test]
    fn data_table_produces_element_empty() {
        let cols = vec![
            make_col("#", Length::Fixed(36.0), TextAlign::Right),
            make_col("Name", Length::FillPortion(3), TextAlign::Left),
        ];
        let _elem: Element<'_, String> = data_table(
            cols,
            vec![],
            Id::unique(),
            None,
            DataTableStyle::default(),
            None,
        );
    }

    #[test]
    fn data_table_produces_element_with_rows() {
        let cols = vec![
            make_col("#", Length::Fixed(36.0), TextAlign::Right),
            make_col("Name", Length::FillPortion(3), TextAlign::Left),
        ];
        let row1: Vec<Element<'_, String>> = vec![
            iced::widget::text("1").into(),
            iced::widget::text("Alice").into(),
        ];
        let _elem: Element<'_, String> = data_table(
            cols,
            vec![row1],
            Id::unique(),
            Some(0),
            DataTableStyle::default(),
            None,
        );
    }
}
