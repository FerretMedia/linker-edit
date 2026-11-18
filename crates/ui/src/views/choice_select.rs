//! A `<select>` of a few named values, on gpui-component's Select: the library's kinds, a
//! series' workspace. Its value is a string, as an `<option value>` is.

use gpui::{AppContext, Context, Entity, SharedString, Window};
use gpui_kit::component::select::{SelectItem, SelectState};

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    pub label: SharedString,
}

impl Choice {
    pub fn new(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Choice {
            value: value.into(),
            label: label.into(),
        }
    }
}

impl SelectItem for Choice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

pub type ChoiceState = SelectState<Vec<Choice>>;

pub fn choice_state<V: 'static>(
    choices: Vec<Choice>,
    selected: &str,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Entity<ChoiceState> {
    let index = choices.iter().position(|choice| choice.value == selected);
    cx.new(|cx| {
        SelectState::new(
            choices,
            index.map(gpui_kit::component::IndexPath::new),
            window,
            cx,
        )
    })
}

/// The value picked, or "" (an `<option value="">`) when none is.
pub fn chosen(state: &Entity<ChoiceState>, cx: &gpui::App) -> String {
    state.read(cx).selected_value().cloned().unwrap_or_default()
}

/// Follows a list and a value that came from the server.
pub fn sync_choices(
    state: &Entity<ChoiceState>,
    choices: Vec<Choice>,
    selected: &str,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    state.update(cx, |state, cx| {
        state.set_items(choices, window, cx);
        state.set_selected_value(&selected.to_string(), window, cx);
    });
}
