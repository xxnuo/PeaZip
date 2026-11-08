use crate::commands::{self, CommandId, Commands};
use crate::navigation::{self, BrowserRow, BrowserState, Sort};
use nsh_base::{button, checkbox, input, theme, toolbar};
use nsh_core::{
    element::{AutoCapitalize, Position, SubmitBehavior},
    prelude::*,
};
use peazip_core::Entry;
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub(crate) struct Controls {
    pub flat: Signal<bool>,
    pub sort: Signal<Sort>,
    pub draft: Signal<String>,
    pub input: ElementRef,
    pub content: ElementRef,
    pub visible: Memo<Vec<BrowserRow>>,
}

impl Controls {
    pub fn new(entries: Signal<Vec<Entry>>, navigation: Signal<BrowserState>) -> Self {
        let flat = signal(false);
        let sort = signal(Sort::NameAsc);
        let visible = computed(move || {
            entries.with(|items| {
                navigation
                    .with(|state| navigation::rows(items, state.current(), flat.get(), sort.get()))
            })
        });
        Self {
            flat,
            sort,
            draft: signal("/".into()),
            input: element_ref(),
            content: element_ref(),
            visible,
        }
    }
}

pub(crate) struct Explorer {
    pub entries: Signal<Vec<Entry>>,
    pub selected: Signal<BTreeSet<String>>,
    pub busy: Signal<bool>,
    pub name: Signal<String>,
    pub navigation: Signal<BrowserState>,
    pub archive_version: Signal<u64>,
    pub controls: Controls,
    pub commands: Commands,
}

pub(crate) fn explorer(context: Explorer) -> impl IntoElement {
    let Explorer {
        entries,
        selected,
        busy,
        name,
        navigation,
        archive_version,
        controls,
        commands,
    } = context;
    let Controls {
        flat,
        sort,
        draft,
        input: path_input,
        content,
        visible,
    } = controls;
    let back = commands.get(CommandId::Back).unwrap();
    let forward = commands.get(CommandId::Forward).unwrap();
    let up = commands.get(CommandId::Up).unwrap();
    let root = commands.get(CommandId::Root).unwrap();
    let select_all = commands.get(CommandId::SelectAll).unwrap();
    let clear_selection = commands.get(CommandId::ClearSelection).unwrap();
    let invert = commands.get(CommandId::InvertSelection).unwrap();
    let toggle_flat = commands.get(CommandId::ToggleFlat).unwrap();
    let sort_name = commands.get(CommandId::SortName).unwrap();
    let sort_size = commands.get(CommandId::SortSize).unwrap();
    let sort_type = commands.get(CommandId::SortType).unwrap();
    let sort_packed = commands.get(CommandId::SortPacked).unwrap();
    let error = signal(String::new());
    let wide = signal(false);
    let packed_order = move || matches!(sort.get(), Sort::PackedAsc | Sort::PackedDesc);
    let show_size = move || wide.get() || !packed_order();
    let show_packed = move || wide.get() || packed_order();
    effect(move || {
        let _ = archive_version.get();
        draft.set(navigation.with(|state| {
            if state.current().is_empty() {
                "/".into()
            } else {
                state.current().into()
            }
        }));
        error.set(String::new());
        path_input.blur();
        content.focus();
    });
    let folders = computed(move || {
        entries.with(|items| {
            navigation.with(|state| {
                navigation::rows(items, state.current(), false, Sort::NameAsc)
                    .into_iter()
                    .filter(|row| row.is_dir)
                    .collect::<Vec<_>>()
            })
        })
    });
    let enter = move |path: String| {
        if busy.get_untracked() {
            return;
        }
        let mut next = navigation.get_untracked();
        match entries.with(|items| next.enter(&path, items)) {
            Ok(()) => {
                draft.set(if next.current().is_empty() {
                    "/".into()
                } else {
                    next.current().into()
                });
                navigation.set(next);
                path_input.blur();
                content.focus();
                error.set(String::new());
            }
            Err(message) => error.set(message),
        }
    };
    view().flex_1().min_h(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| wide.set(bounds.size.width.0 >= 760.0))
        .child(text(move || name.get()).number_of_lines(1).px_3().py_1().border_b_1().border_color(move || theme().border))
        .child(toolbar().flex_row().flex_wrap().gap_2().px_2().py_1().border_b_1().border_color(move || theme().border)
            .child(commands::button(&back).px_2().py_1().flex_row().items_center().gap_1().child(crate::icons::icon("back")).child(text("Back")))
            .child(commands::button(&forward).px_2().py_1().flex_row().items_center().gap_1().child(crate::icons::icon("forward")).child(text("Forward")))
            .child(commands::button(&up).px_2().py_1().flex_row().items_center().gap_1().child(crate::icons::icon("up")).child(text("Up")))
            .child(commands::button(&root).px_2().py_1().child(text("Archive root")))
            .child(input().element_ref(&path_input).submit_behavior(SubmitBehavior::Submit).auto_capitalize(AutoCapitalize::None).auto_correct(false).value(move || draft.get()).on_change(move |value: &str| draft.set(value.to_owned())).on_submit(move |_: &str| enter(draft.get_untracked())).label("Archive path").placeholder("Archive path").disabled(move || busy.get()).flex_1().min_w(px(110.0)).border_1().border_color(move || theme().border).px_2().py_1())
            .child(button().disabled(move || busy.get()).on_press(move || enter(draft.get_untracked())).px_2().py_1().child(text("Go"))))
        .child(when(move || !error.with(String::is_empty), move || text(move || error.get()).px_3().py_1().text_color(move || theme().destructive)))
        .child(toolbar().flex_row().flex_wrap().gap_3().px_3().py_1().border_b_1().border_color(move || theme().border)
            .child(commands::button(&toggle_flat).child(text(move || if flat.get() { "Folder view" } else { "All entries" })))
            .child(commands::button(&select_all).child(text("Select all")))
            .child(commands::button(&invert).child(text("Invert selection")))
            .child(commands::button(&clear_selection).child(text("Clear selection")))
            .child(text(move || format!("{} selected",selected.with(BTreeSet::len)))))
        .child(view().flex_row().flex_1().min_h(px(0.0))
            .child(when(move || wide.get() && !flat.get(), move || view().w(px(180.0)).min_h(px(0.0)).border_r_1().border_color(move || theme().border)
                .child(text("Archive folders").px_3().py_2().font_weight(FontWeight::SEMIBOLD))
                .child(button().accessibility_label("Browse folder /").disabled(move || busy.get()).on_press(move || enter(String::new())).px_3().py_1().child(text("/")))
                .child(text(move || navigation.with(|state| state.current().to_owned())).px_3().py_1().number_of_lines(1))
                .child(virtual_list(move || folders.with(Vec::len), move |at: Position| {
                    let row = move || folders.with(|rows| rows.get(at.index()).cloned());
                    button().accessibility_label(move || row().map(|row| format!("Browse folder /{}", row.path).into())).disabled(move || busy.get())
                        .on_press(move || {if let Some(row)=row(){ enter(row.path); }}).px_3().py_2()
                        .child(text(move || row().map(|row|row.name).unwrap_or_default()).number_of_lines(1))
                }).estimated_item_size(px(32.0)).flex_1())))
            .child(view().element_ref(&content).focusable(true).role(AccessibilityRole::Group).label("Archive contents").flex_1().min_w(px(0.0)).min_h(px(0.0))
                .child(when(move || show_packed() && entries.with(|items| items.iter().any(|entry| !entry.is_dir && entry.packed_size.is_none())),
                    || text("— means the file's packed size is unavailable.").px_3().py_1().text_color(move || theme().muted_foreground)))
                .child(view().flex_row().px_3().py_1().bg(move || theme().muted).border_b_1().border_color(move || theme().border)
                    .child(view().flex_row().flex_1().gap_2()
                        .child(commands::button(&sort_name).flex_1().child(text("Name")))
                        .child(when(move || wide.get(), move || commands::button(&sort_type).child(text("Type")).w(px(85.0))))
                        .child(when(show_size, move || commands::button(&sort_size).w(px(110.0)).child(text("Size (bytes)"))))
                        .child(when(show_packed, move || commands::button(&sort_packed).w(px(110.0)).child(text("Packed (bytes)")))))
                    .child(view().w(px(36.0))))
                .child(virtual_list(move || visible.with(Vec::len), move |at: Position| {
                    let row=move || visible.with(|rows| rows.get(at.index()).cloned());
                    let selection = move || row().map_or((false, false), |row| entries.with(|items| selected.with(|chosen| navigation::selection_state(&row, items, chosen))));
                    let checked = move || selection().0;
                    let mixed = move || selection().1;
                    view().flex_row().items_center().border_b_1().border_color(move || theme().border)
                        .bg(move || if checked() {theme().muted}else{theme().background})
                        .child(checkbox().accessibility_label(move || row().map(|row| row.path.into())).checked(checked).indeterminate(mixed).disabled(move || busy.get())
                            .on_change(move |value: &bool| {
                                if busy.get_untracked() {return;}
                                if let Some(row)=row() {
                                    entries.with(|items| selected.update(|chosen| navigation::set_selected(&row, *value, items, chosen)));
                                }
                            }).flex_row().items_center().flex_1().min_w(px(0.0)).gap_2().px_3().py_2()
                            .indicator_class("size-4 border border-border bg-background")
                            .indicator(view().size_4().items_center().justify_center().child(when(checked, move || view().w(px(8.0)).h(px(8.0)).bg(move || theme().foreground))).child(when(mixed, move || view().w(px(8.0)).h(px(2.0)).bg(move || theme().foreground))))
                            .child(when(move || row().is_some_and(|row|row.is_dir), || crate::icons::icon("folder")))
                            .child(when(move || row().is_some_and(|row|!row.is_dir), || crate::icons::icon("file")))
                            .child(text(move || row().map(|row|row.name).unwrap_or_default()).number_of_lines(1).flex_1().min_w(px(0.0)).pr_2())
                            .child(when(move || wide.get(), move || text(move || row().map(|row| row.kind_label()).unwrap_or_default()).number_of_lines(1).w(px(85.0))))
                            .child(when(show_size, move || text(move || row().map(|row| if row.is_dir {"—".into()}else{row.size.to_string()}).unwrap_or_default())
                                .label(move || row().map(|row| if row.is_dir {"Size in bytes: not applicable".into()} else {format!("Size in bytes: {}",row.size)}).unwrap_or_default()).w(px(110.0))))
                            .child(when(show_packed, move || text(move || row().and_then(|row|row.packed_size).map(|size|size.to_string()).unwrap_or_else(||"—".into()))
                                .label(move || row().map(|row| if row.is_dir {"Packed bytes: not applicable".into()} else {row.packed_size.map(|size|format!("Packed bytes: {size}")).unwrap_or_else(||"Packed bytes: unavailable".into())}).unwrap_or_default()).w(px(110.0)))))
                        .child(view().w(px(36.0)).child(when(move || row().is_some_and(|row|row.is_dir), move || button().accessibility_label(move ||row().map(|row|format!("Open folder {}",row.path).into())).disabled(move ||busy.get()).on_press(move || {if let Some(row)=row(){flat.set(false);enter(row.path);}}).px_2().py_2().child(text("›")))))
                }).estimated_item_size(px(36.0)).flex_1())
            ))
}
