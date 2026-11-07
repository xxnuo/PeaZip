use nsh_base::{button, dialog, input, radio, radio_group, theme};
use nsh_core::prelude::*;
use peazip_core::{ZipCompression, ZipOptions};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compression {
    Automatic,
    Store,
    Deflate,
}

fn options(compression: Compression, level: &str) -> Option<ZipOptions> {
    let compression = match compression {
        Compression::Automatic => ZipCompression::Automatic,
        Compression::Store => ZipCompression::Stored,
        Compression::Deflate => ZipCompression::Deflate {
            level: level.parse().ok()?,
        },
    };
    let options = ZipOptions { compression };
    options.validate().ok()?;
    Some(options)
}

pub(crate) fn show(
    input_description: String,
    initial: ZipOptions,
    confirm: impl Fn(ZipOptions) + Copy + 'static,
    cancel: impl Fn() + Copy + 'static,
) -> impl IntoElement {
    let (initial_compression, initial_level) = match initial.compression {
        ZipCompression::Automatic => (Compression::Automatic, 6),
        ZipCompression::Stored => (Compression::Store, 6),
        ZipCompression::Deflate { level } => (Compression::Deflate, level),
    };
    let compression = signal(initial_compression);
    let level = signal(initial_level.to_string());
    let selected = computed(move || options(compression.get(), &level.get()));
    let finished = signal(false);
    let dismiss = move || {
        if !finished.get_untracked() {
            finished.set(true);
            cancel();
        }
    };
    let submit = move || {
        if !finished.get_untracked()
            && let Some(options) = selected.get_untracked()
        {
            finished.set(true);
            confirm(options);
        }
    };
    let choice = move |value, label: &'static str| {
        let checked = move || compression.get() == value;
        radio()
            .accessibility_label(label)
            .checked(checked)
            .on_change(move |checked: &bool| {
                if *checked && !finished.get_untracked() {
                    compression.set(value);
                }
            })
            .disabled(move || finished.get())
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
    };
    dialog().animation(ModalAnimation::None)
        .role(AccessibilityRole::Group).label("Create archive")
        .close_on_escape(false).close_on_backdrop_press(false).close_on_swipe(false)
        .on_cancel(dismiss)
        .flex_1().p_safe_area().p_3().pb_keyboard_plus(px(12.0)).items_center().justify_center().bg(rgba(0x00000066))
        .child(view().w_full().max_w(px(460.0)).max_h(relative(1.0)).min_h(px(0.0))
            .bg(move || theme().background).text_color(move || theme().foreground)
            .border_1().border_color(move || theme().border)
            .child(text("Create archive").font_weight(FontWeight::SEMIBOLD).px_3().py_2().border_b_1().border_color(move || theme().border))
            .child(scroll().min_h(px(0.0)).flex_shrink(1.0).child(view().p_3().gap_3()
                .child(view().flex_row().gap_3().child(text("Format")).child(text("ZIP")))
                .child(view().gap_1().child(text("Input").font_weight(FontWeight::SEMIBOLD)).child(text(input_description)))
                .child(view().gap_1().role(AccessibilityRole::Group).label("Compression").child(text("Compression").font_weight(FontWeight::SEMIBOLD))
                    .child(radio_group()
                        .child(choice(Compression::Automatic, "Automatic"))
                        .child(choice(Compression::Store, "Store (no compression)"))
                        .child(choice(Compression::Deflate, "Deflate"))))
                .child(when(move || compression.get() == Compression::Deflate, move || view().gap_1()
                    .child(view().flex_row().flex_wrap().items_center().gap_2()
                        .child(text("Deflate level (1–9)"))
                        .child(input().label("Compression level").placeholder("Compression level")
                            .value(move || level.get()).on_change(move |value: &str| level.set(value.to_owned()))
                            .keyboard_type(KeyboardType::Number).auto_capitalize(AutoCapitalize::None).auto_correct(false)
                            .submit_behavior(SubmitBehavior::Submit).on_submit(move |_: &str| submit())
                            .max_length(3).w(px(64.0)).border_1().border_color(move || theme().border).px_2().py_1()
                            .disabled(move || finished.get())))
                    .child(when(move || selected.get().is_none(), || text("Enter a Deflate level from 1 to 9.").text_color(move || theme().destructive)))))
                .child(text(move || match compression.get() {
                    Compression::Automatic => "Automatic may use Store to meet compression-ratio limits.",
                    Compression::Store => "Stores file data without compression.",
                    Compression::Deflate => "Uses the selected compression level. Exceeding compression-ratio limits reports an error; it does not switch to Store.",
                }).text_color(move || theme().muted_foreground))))
            .child(view().flex_row().flex_wrap().justify_end().gap_2().p_3().border_t_1().border_color(move || theme().border)
                .child(button().accessibility_label("Cancel").disabled(move || finished.get()).on_press(dismiss)
                    .px_3().py_2().border_1().border_color(move || theme().border).child(text("Cancel")))
                .child(button().accessibility_label("Continue").disabled(move || finished.get() || selected.get().is_none()).on_press(submit)
                    .px_3().py_2().border_1().border_color(move || theme().border).bg(move || theme().muted)
                    .disabled_style(|s| s.opacity(0.4)).child(text("Continue")))))
}
