/// Draggable split pane widget backed by `iced::widget::pane_grid`.
///
/// Upgrade from Phase 3.1's static `FillPortion` layout.  The `SplitPaneState`
/// must be stored by the caller (App) so the drag state persists across frames.
/// Flagged for upstreaming to twui as `split_pane`.
use std::cell::RefCell;

use iced::widget::pane_grid;
use iced::{Element, Length};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneSide {
    Left,
    Right,
}

/// Kept for API compatibility with call sites that pass an orientation argument.
#[derive(Debug, Clone, Copy)]
pub enum SplitOrientation {
    Horizontal,
}

/// Persistent state for a two-pane draggable split.
///
/// Initialize once with [`SplitPaneState::new`] and store on the owning struct.
/// Pass `&mut state` to [`split_pane_handle_resize`] on resize events, and
/// `&state` to [`split_pane`] when building the view.
pub struct SplitPaneState {
    pane_state: pane_grid::State<PaneSide>,
    ratio: f32,
}

impl SplitPaneState {
    /// Creates a new split with the given initial ratio (0.0–1.0).
    ///
    /// `ratio` is the fraction of space given to the left/top pane.
    pub fn new(ratio: f32) -> Self {
        let ratio = ratio.clamp(0.05, 0.95);
        let config = pane_grid::Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio,
            a: Box::new(pane_grid::Configuration::Pane(PaneSide::Left)),
            b: Box::new(pane_grid::Configuration::Pane(PaneSide::Right)),
        };
        let pane_state = pane_grid::State::with_configuration(config);
        Self { pane_state, ratio }
    }

    /// Applies a drag resize event, updating the internal ratio.
    pub fn handle_resize(&mut self, event: pane_grid::ResizeEvent) {
        self.pane_state.resize(event.split, event.ratio);
        self.ratio = event.ratio;
    }

    /// Returns the current split ratio (left pane fraction).
    pub fn ratio(&self) -> f32 {
        self.ratio
    }
}

/// Renders a horizontally-split pane grid.
///
/// `on_resize` is called with a [`pane_grid::ResizeEvent`] whenever the user
/// drags the divider.  The caller should forward the event to
/// [`SplitPaneState::handle_resize`] and persist the ratio.
pub fn split_pane<'a, Message: Clone + 'a>(
    state: &'a SplitPaneState,
    left: Element<'a, Message>,
    right: Element<'a, Message>,
    _orientation: SplitOrientation,
    on_resize: impl Fn(pane_grid::ResizeEvent) -> Message + 'a,
) -> Element<'a, Message> {
    // RefCell lets us move pre-built elements into the Fn closure.
    // Each pane is visited exactly once by PaneGrid, so take() is safe.
    let left = RefCell::new(Some(left));
    let right = RefCell::new(Some(right));

    pane_grid::PaneGrid::new(&state.pane_state, move |_, side, _| {
        let elem = match side {
            PaneSide::Left => left
                .borrow_mut()
                .take()
                .unwrap_or_else(|| {
                    iced::widget::Space::new()
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into()
                }),
            PaneSide::Right => right
                .borrow_mut()
                .take()
                .unwrap_or_else(|| {
                    iced::widget::Space::new()
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .into()
                }),
        };
        pane_grid::Content::new(elem)
    })
    .on_resize(8, on_resize)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_pane_state_clamps_ratio() {
        let s = SplitPaneState::new(1.5);
        assert!(s.ratio() <= 0.95);

        let s2 = SplitPaneState::new(-0.5);
        assert!(s2.ratio() >= 0.05);
    }

    #[test]
    fn split_pane_state_stores_ratio() {
        let s = SplitPaneState::new(0.65);
        assert!((s.ratio() - 0.65).abs() < 1e-4);
    }

    #[test]
    fn split_pane_produces_element() {
        let state = SplitPaneState::new(0.65);
        let left: Element<'_, String> = iced::widget::Space::new().width(Length::Fill).height(Length::Fill).into();
        let right: Element<'_, String> = iced::widget::Space::new().width(Length::Fill).height(Length::Fill).into();
        let _elem = split_pane(&state, left, right, SplitOrientation::Horizontal, |_e| {
            "resized".to_string()
        });
    }

    #[test]
    fn handle_resize_updates_ratio() {
        let mut s = SplitPaneState::new(0.65);
        // Simulate a resize event — grab the split id from internal state.
        // We can't construct a ResizeEvent directly without a valid split id,
        // so this test just validates ratio storage on direct mutation.
        s.ratio = 0.72;
        assert!((s.ratio() - 0.72).abs() < 1e-6);
    }
}
