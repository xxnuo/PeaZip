use crate::commands::{self, Command};
use nsh_base::{HandleEdge, resize_handle, theme};
use nsh_core::prelude::*;
use nsh_core::{AccessibilityActionDef, AccessibilityValue};

#[derive(Clone, Copy)]
pub(crate) enum Column {
    Name,
    Type,
    Size,
    Packed,
}

impl Column {
    fn index(self) -> usize {
        self as usize
    }

    fn limits(self) -> (f32, f32) {
        match self {
            Self::Name => (160.0, 2048.0),
            Self::Type => (56.0, 800.0),
            Self::Size | Self::Packed => (56.0, 800.0),
        }
    }
}

fn resized(column: Column, start: f32, offset: f32) -> Option<f32> {
    let next = start + offset;
    if !start.is_finite() || !offset.is_finite() || !next.is_finite() {
        return None;
    }
    let (min, max) = column.limits();
    Some(next.clamp(min, max))
}

#[derive(Clone, Copy)]
pub(crate) struct ColumnWidths(Signal<[Option<f32>; 4]>);

impl ColumnWidths {
    pub fn new() -> Self {
        Self(signal([None; 4]))
    }

    pub fn get(self, column: Column, fallback: f32) -> f32 {
        self.0
            .with(|widths| widths[column.index()].unwrap_or(fallback))
    }

    pub fn adjust(self) {
        self.0
            .set([Some(240.0), Some(72.0), Some(72.0), Some(72.0)]);
    }

    fn resize(self, column: Column, start: f32, offset: f32) {
        if let Some(width) = resized(column, start, offset) {
            self.0.update(|widths| widths[column.index()] = Some(width));
        }
    }

    fn reset(self, column: Column) {
        self.0.update(|widths| widths[column.index()] = None);
    }
}

pub(crate) fn header(
    widths: ColumnWidths,
    column: Column,
    command: Command,
    label: &'static str,
    width: impl Fn() -> f32 + Copy + 'static,
    busy: Signal<bool>,
    wide: Signal<bool>,
) -> impl IntoElement {
    view()
        .relative()
        .flex_row()
        .flex_shrink(0.0)
        .min_w(px(0.0))
        .style_with(move |style| style.w(px(width())))
        .child(
            commands::button(&command)
                .accessibility_label(label)
                .flex_1()
                .min_w(px(0.0))
                .px_2()
                .child(
                    text(move || match column {
                        Column::Size if width() < 110.0 => "Size",
                        Column::Packed if width() < 110.0 => "Packed",
                        _ => label,
                    })
                    .number_of_lines(1),
                ),
        )
        .child(when(
            move || wide.get(),
            move || {
                let focused = signal(false);
                let drag_start = stored(None::<f32>);
                let handle = element_ref();
                effect(move || {
                    if !focused.get() || busy.get() || !wide.get() {
                        return;
                    }
                    for (key, offset) in [
                        ("left", -10.0),
                        ("right", 10.0),
                        ("shift+left", -50.0),
                        ("shift+right", 50.0),
                        ("home", 0.0),
                    ] {
                        on_key_with(
                            key,
                            KeyOptions {
                                when: nsh_core::keymap::When::NotEditing,
                                repeat: key != "home",
                                ..Default::default()
                            },
                            move |_| {
                                if !focused.get_untracked()
                                    || busy.get_untracked()
                                    || !wide.get_untracked()
                                {
                                    return;
                                }
                                if key == "home" {
                                    widths.reset(column);
                                } else {
                                    widths.resize(column, width(), offset);
                                }
                            },
                        );
                    }
                });
                view()
                    .element_ref(&handle)
                    .absolute()
                    .right(px(0.0))
                    .top(px(0.0))
                    .bottom(px(0.0))
                    .w(px(9.0))
                    .role(AccessibilityRole::Adjustable)
                    .label(format!("Resize {label} column"))
                    .accessibility_value(move || {
                        let (min, max) = column.limits();
                        Some(AccessibilityValue {
                            min: Some(min.min(width())),
                            max: Some(max.max(width())),
                            now: Some(width()),
                            text: Some(
                                format!(
                                    "{} points; Left/Right to resize, Home to reset",
                                    width().round()
                                )
                                .into(),
                            ),
                        })
                    })
                    .disabled(move || busy.get())
                    .focusable(true)
                    .accessibility_actions(vec![
                        AccessibilityActionDef {
                            name: "increment".into(),
                            label: None,
                        },
                        AccessibilityActionDef {
                            name: "decrement".into(),
                            label: None,
                        },
                    ])
                    .on_accessibility_action(move |name: &str| {
                        if busy.get_untracked() || !wide.get_untracked() {
                            return;
                        }
                        match name {
                            "increment" => widths.resize(column, width(), 10.0),
                            "decrement" => widths.resize(column, width(), -10.0),
                            _ => {}
                        }
                    })
                    .on_focus(move || focused.set(true))
                    .on_blur(move || focused.set(false))
                    .child(
                        view().flex_1().accessibility_elements_hidden(true).child(
                            resize_handle()
                                .edge(Some(HandleEdge::Trailing))
                                .disabled(move || busy.get())
                                .on_drag_start(move || {
                                    if !busy.get_untracked() && wide.get_untracked() {
                                        drag_start.replace(Some(width()));
                                        handle.focus();
                                    }
                                })
                                .on_drag_move(move |offset: &Points| {
                                    if busy.get_untracked() || !wide.get_untracked() {
                                        drag_start.replace(None);
                                    } else if let Some(start) = drag_start.get() {
                                        widths.resize(column, start, offset.0);
                                    }
                                })
                                .on_drag_end(move |offset: &Points| {
                                    if !busy.get_untracked()
                                        && wide.get_untracked()
                                        && let Some(start) = drag_start.get()
                                    {
                                        widths.resize(column, start, offset.0);
                                    }
                                    drag_start.replace(None);
                                })
                                .child(view().w(px(1.0)).h_full().bg(move || {
                                    if focused.get() {
                                        theme().foreground
                                    } else {
                                        theme().border
                                    }
                                })),
                        ),
                    )
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_uses_cumulative_offset_and_column_bounds() {
        assert_eq!(resized(Column::Name, 320.0, 80.0), Some(400.0));
        assert_eq!(resized(Column::Name, 320.0, -1000.0), Some(160.0));
        assert_eq!(resized(Column::Name, 320.0, 5000.0), Some(2048.0));
        assert_eq!(resized(Column::Type, 85.0, -100.0), Some(56.0));
        assert_eq!(resized(Column::Size, 110.0, -100.0), Some(56.0));
        assert_eq!(resized(Column::Packed, 110.0, 1000.0), Some(800.0));
        for offset in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(resized(Column::Name, 320.0, offset), None);
        }
        assert_eq!(resized(Column::Name, f32::NAN, 10.0), None);
        assert_eq!(resized(Column::Name, f32::MAX, f32::MAX), None);
    }
}
