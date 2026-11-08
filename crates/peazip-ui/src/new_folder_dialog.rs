use nsh_base::{button, dialog, input, theme};
use nsh_core::prelude::*;
use peazip_native::{DirectoryLimits, validate_folder_name};

pub(crate) fn show(
    parent: String,
    error: Signal<String>,
    working: Signal<bool>,
    submit: impl Fn(String) + Copy + 'static,
    cancel: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let name = signal("New folder".to_owned());
    let field = element_ref();
    let dismissed = signal(false);
    let validation = computed(move || {
        validate_folder_name(&name.get(), DirectoryLimits::default())
            .map_err(|error| error.to_string())
    });
    after_answer(move || {
        field.focus();
        field.set_selection(0, "New folder".encode_utf16().count() as u32);
    });
    let dismiss = move || {
        if !working.get_untracked() && !dismissed.get_untracked() {
            dismissed.set(true);
            cancel();
        }
    };
    let create = move || {
        if !working.get_untracked()
            && !dismissed.get_untracked()
            && validation.get_untracked().is_ok()
        {
            submit(name.get_untracked());
        }
    };
    dialog()
        .animation(ModalAnimation::None)
        .role(AccessibilityRole::Group)
        .label("New folder")
        .close_on_escape(false)
        .close_on_backdrop_press(false)
        .close_on_swipe(false)
        .on_cancel(dismiss)
        .flex_1()
        .p_safe_area()
        .p_3()
        .pb_keyboard_plus(px(12.0))
        .items_center()
        .justify_center()
        .bg(rgba(0x00000066))
        .child(
            view()
                .w_full()
                .max_w(px(460.0))
                .max_h(relative(1.0))
                .min_h(px(0.0))
                .bg(move || theme().background)
                .text_color(move || theme().foreground)
                .border_1()
                .border_color(move || theme().border)
                .child(
                    text("New folder")
                        .font_weight(FontWeight::SEMIBOLD)
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(move || theme().border),
                )
                .child(
                    scroll().min_h(px(0.0)).flex_shrink(1.0).child(
                        view()
                            .p_3()
                            .gap_3()
                            .child(
                                view()
                                    .gap_1()
                                    .child(text("Parent folder").font_weight(FontWeight::SEMIBOLD))
                                    .child(text(parent)),
                            )
                            .child(
                                view().gap_1().child(text("Folder name")).child(
                                    input()
                                        .element_ref(&field)
                                        .label("Folder name")
                                        .placeholder("Folder name")
                                        .value(move || name.get())
                                        .on_change(move |value: &str| {
                                            if !working.get_untracked()
                                                && !dismissed.get_untracked()
                                            {
                                                name.set(value.to_owned());
                                                error.set(String::new());
                                            }
                                        })
                                        .max_length(1024)
                                        .auto_capitalize(AutoCapitalize::None)
                                        .auto_correct(false)
                                        .submit_behavior(SubmitBehavior::Submit)
                                        .on_submit(move |_: &str| create())
                                        .on_key_press(move |key: &str| {
                                            if key == "Escape" {
                                                dismiss();
                                            }
                                        })
                                        .disabled(move || working.get() || dismissed.get())
                                        .border_1()
                                        .border_color(move || theme().border)
                                        .px_2()
                                        .py_1(),
                                ),
                            )
                            .child(when(
                                move || validation.get().is_err(),
                                move || {
                                    text(move || validation.get().err().unwrap_or_default())
                                        .text_color(move || theme().destructive)
                                },
                            ))
                            .child(when(
                                move || !error.with(String::is_empty),
                                move || {
                                    text(move || error.get())
                                        .text_color(move || theme().destructive)
                                },
                            ))
                            .child(
                                text("Creates one new folder. Existing items are not replaced.")
                                    .text_color(move || theme().muted_foreground),
                            ),
                    ),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .justify_end()
                        .gap_2()
                        .p_3()
                        .border_t_1()
                        .border_color(move || theme().border)
                        .child(
                            button()
                                .accessibility_label("Cancel")
                                .disabled(move || working.get() || dismissed.get())
                                .on_press(dismiss)
                                .px_3()
                                .py_2()
                                .border_1()
                                .border_color(move || theme().border)
                                .disabled_style(|s| s.opacity(0.4))
                                .child(text("Cancel")),
                        )
                        .child(
                            button()
                                .accessibility_label("Create")
                                .disabled(move || {
                                    working.get() || dismissed.get() || validation.get().is_err()
                                })
                                .on_press(create)
                                .px_3()
                                .py_2()
                                .border_1()
                                .border_color(move || theme().border)
                                .bg(move || theme().muted)
                                .disabled_style(|s| s.opacity(0.4))
                                .child(text("Create")),
                        ),
                ),
        )
}
