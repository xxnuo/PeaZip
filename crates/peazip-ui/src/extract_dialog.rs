use nsh_base::{button, dialog, radio, radio_group, theme};
use nsh_core::prelude::*;
use peazip_core::{ExtractOptions, ExtractPaths};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractScope {
    Selected,
    Displayed,
    All,
}

fn choice<T: Copy + PartialEq + 'static>(
    state: Signal<T>,
    value: T,
    label: &'static str,
    disabled: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let checked = move || state.get() == value;
    radio()
        .accessibility_label(label)
        .checked(checked)
        .disabled(disabled)
        .disabled_style(|s| s.opacity(0.4))
        .on_change(move |selected: &bool| {
            if *selected && !disabled() {
                state.set(value);
            }
        })
        .flex_row()
        .items_center()
        .gap_2()
        .py_1()
        .min_h(px(if is_desktop() { 32.0 } else { 44.0 }))
        .child(
            view()
                .size_4()
                .rounded_full()
                .border_1()
                .border_color(move || theme().border)
                .items_center()
                .justify_center()
                .child(when(checked, || {
                    view()
                        .size_2()
                        .rounded_full()
                        .bg(move || theme().foreground)
                })),
        )
        .child(text(label))
}

pub(crate) struct Summary {
    pub name: String,
    pub selected_count: usize,
    pub total_count: usize,
    pub displayed_count: usize,
    pub displayed_location: String,
}

pub(crate) fn show(
    summary: Summary,
    initial: ExtractOptions,
    confirm: impl Fn(ExtractScope, ExtractOptions) + Copy + 'static,
    cancel: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let Summary {
        name,
        selected_count,
        total_count,
        displayed_count,
        displayed_location,
    } = summary;
    let scope = signal(if selected_count > 0 {
        ExtractScope::Selected
    } else {
        ExtractScope::Displayed
    });
    let paths = signal(initial.paths);
    let finished = signal(false);
    let valid = move || match scope.get() {
        ExtractScope::Selected => selected_count > 0,
        ExtractScope::Displayed => displayed_count > 0,
        ExtractScope::All => total_count > 0,
    };
    let dismiss = move || {
        if !finished.get_untracked() {
            finished.set(true);
            cancel();
        }
    };
    let submit = move || {
        if !finished.get_untracked() && valid() {
            let scope = scope.get_untracked();
            let options = ExtractOptions {
                paths: paths.get_untracked(),
            };
            finished.set(true);
            confirm(scope, options);
        }
    };
    dialog().animation(ModalAnimation::None)
        .role(AccessibilityRole::Group).label("Extract archive")
        .close_on_escape(false).close_on_backdrop_press(false).close_on_swipe(false)
        .on_cancel(dismiss)
        .flex_1().p_safe_area().p_3().pb_keyboard_plus(px(12.0)).items_center().justify_center().bg(rgba(0x00000066))
        .child(view().w_full().max_w(px(460.0)).max_h(relative(1.0)).min_h(px(0.0))
            .bg(move || theme().background).text_color(move || theme().foreground)
            .border_1().border_color(move || theme().border)
            .child(text("Extract archive").font_weight(FontWeight::SEMIBOLD).px_3().py_2().border_b_1().border_color(move || theme().border))
            .child(scroll().min_h(px(0.0)).flex_shrink(1.0).child(view().p_3().gap_3()
                .child(view().gap_1().child(text("Archive").font_weight(FontWeight::SEMIBOLD)).child(text(name)))
                .child(text(displayed_location))
                .child(text(format!("{selected_count} selected; {displayed_count} in displayed scope; {total_count} archive entries")))
                .child(view().gap_1().role(AccessibilityRole::Group).label("Scope")
                    .child(text("Scope").font_weight(FontWeight::SEMIBOLD))
                    .child(radio_group()
                        .child(choice(scope, ExtractScope::Selected, "Selected entries", move || finished.get() || selected_count == 0))
                        .child(choice(scope, ExtractScope::Displayed, "Displayed entries", move || finished.get() || displayed_count == 0))
                        .child(choice(scope, ExtractScope::All, "All entries", move || finished.get() || total_count == 0))))
                .child(view().gap_1().role(AccessibilityRole::Group).label("Paths")
                    .child(text("Paths").font_weight(FontWeight::SEMIBOLD))
                    .child(radio_group()
                        .child(choice(paths, ExtractPaths::Full, "Full paths", move || finished.get()))
                        .child(choice(paths, ExtractPaths::FilesOnly, "Files only (no folders)", move || finished.get()))))
                .child(text(move || if paths.get() == ExtractPaths::FilesOnly {
                    "Files only does not preserve empty directories. Files with the same output name cause an error. Extracts into a new output folder."
                } else {
                    "Preserves archive paths and empty directories. Extracts into a new output folder."
                }).text_color(move || theme().muted_foreground))))
            .child(view().flex_row().flex_wrap().justify_end().gap_2().p_3().border_t_1().border_color(move || theme().border)
                .child(button().accessibility_label("Cancel").disabled(move || finished.get()).on_press(dismiss)
                    .px_3().py_2().border_1().border_color(move || theme().border).child(text("Cancel")))
                .child(button().accessibility_label("Continue").disabled(move || finished.get() || !valid()).on_press(submit)
                    .px_3().py_2().border_1().border_color(move || theme().border).bg(move || theme().muted)
                    .disabled_style(|s| s.opacity(0.4)).child(text("Continue")))))
}
