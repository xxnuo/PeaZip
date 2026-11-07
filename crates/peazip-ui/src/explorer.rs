use crate::navigation::{self, BrowserState, Sort};
use nsh_base::{button, checkbox, input, theme, toolbar};
use nsh_core::{
    element::{AutoCapitalize, Position, SubmitBehavior},
    prelude::*,
};
use peazip_core::Entry;
use std::collections::BTreeSet;

pub(crate) fn explorer(
    entries: Signal<Vec<Entry>>,
    selected: Signal<BTreeSet<String>>,
    busy: Signal<bool>,
    name: Signal<String>,
    navigation: Signal<BrowserState>,
    archive_version: Signal<u64>,
) -> impl IntoElement {
    let flat = signal(false);
    let sort = signal(Sort::NameAsc);
    let draft = signal("/".to_owned());
    let error = signal(String::new());
    let wide = signal(false);
    let path_input = element_ref();
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
    });
    let visible = computed(move || {
        entries.with(|items| {
            navigation
                .with(|state| navigation::rows(items, state.current(), flat.get(), sort.get()))
        })
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
                error.set(String::new());
            }
            Err(message) => error.set(message),
        }
    };
    view().flex_1().min_h(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| wide.set(bounds.size.width.0 >= 760.0))
        .child(text(move || name.get()).number_of_lines(1).px_3().py_1().border_b_1().border_color(move || theme().border))
        .child(toolbar().flex_row().flex_wrap().gap_2().px_2().py_1().border_b_1().border_color(move || theme().border)
            .child(button().disabled(move || busy.get() || !navigation.with(BrowserState::can_back)).on_press(move || { navigation.update(|state| { state.back(); }); }).px_2().py_1().flex_row().items_center().gap_1().child(crate::icons::icon("back")).child(text("Back")))
            .child(button().disabled(move || busy.get() || !navigation.with(BrowserState::can_forward)).on_press(move || { navigation.update(|state| { state.forward(); }); }).px_2().py_1().flex_row().items_center().gap_1().child(crate::icons::icon("forward")).child(text("Forward")))
            .child(button().disabled(move || busy.get() || navigation.with(|state| state.current().is_empty())).on_press(move || { navigation.update(|state| { state.up(); }); }).px_2().py_1().flex_row().items_center().gap_1().child(crate::icons::icon("up")).child(text("Up")))
            .child(button().disabled(move || busy.get() || navigation.with(|state| state.current().is_empty())).on_press(move || { navigation.update(|state| { state.root(); }); }).px_2().py_1().child(text("Archive root")))
            .child(input().element_ref(&path_input).submit_behavior(SubmitBehavior::Submit).auto_capitalize(AutoCapitalize::None).auto_correct(false).value(move || draft.get()).on_change(move |value: &str| draft.set(value.to_owned())).on_submit(move |_: &str| enter(draft.get_untracked())).label("Archive path").placeholder("Archive path").disabled(move || busy.get()).flex_1().min_w(px(110.0)).border_1().border_color(move || theme().border).px_2().py_1())
            .child(button().disabled(move || busy.get()).on_press(move || enter(draft.get_untracked())).px_2().py_1().child(text("Go"))))
        .child(when(move || !error.with(String::is_empty), move || text(move || error.get()).px_3().py_1().text_color(move || theme().destructive)))
        .child(toolbar().flex_row().flex_wrap().gap_3().px_3().py_1().border_b_1().border_color(move || theme().border)
            .child(button().disabled(move || busy.get()).on_press(move || flat.update(|value| *value = !*value)).child(text(move || if flat.get() { "Folder view" } else { "All entries" })))
            .child(button().disabled(move || busy.get() || visible.with(Vec::is_empty)).on_press(move || {
                if busy.get_untracked() {return;}
                let paths=entries.with(|items| visible.with(|rows| rows.iter().flat_map(|row| navigation::selection_paths(row,items)).collect()));
                selected.set(paths);
            }).child(text("Select all")))
            .child(button().disabled(move || busy.get() || selected.with(BTreeSet::is_empty)).on_press(move || { if !busy.get_untracked() { selected.set(BTreeSet::new()); } }).child(text("Clear selection")))
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
            .child(view().flex_1().min_w(px(0.0)).min_h(px(0.0))
                .child(view().flex_row().px_3().py_1().bg(move || theme().muted).border_b_1().border_color(move || theme().border)
                    .child(button().disabled(move || busy.get()).on_press(move || sort.update(|value| *value=if *value==Sort::NameAsc {Sort::NameDesc}else{Sort::NameAsc})).flex_1().child(text("Name")))
                    .child(when(move || wide.get(), move || text("Type").w(px(85.0))))
                    .child(button().disabled(move || busy.get()).on_press(move || sort.update(|value| *value=if *value==Sort::SizeAsc {Sort::SizeDesc}else{Sort::SizeAsc})).w(px(110.0)).child(text("Size (bytes)")))
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
                            .child(when(move || wide.get(), move || text(move || row().map(|row| if row.is_dir {"Folder".into()} else {row.name.rsplit_once('.').map(|(_,ext)|ext.to_ascii_uppercase()).filter(|ext|!ext.is_empty()).unwrap_or_else(||"File".into())}).unwrap_or_default()).number_of_lines(1).w(px(85.0))))
                            .child(text(move || row().map(|row| if row.is_dir {"—".into()}else{row.size.to_string()}).unwrap_or_default()).w(px(110.0))))
                        .child(view().w(px(36.0)).child(when(move || row().is_some_and(|row|row.is_dir), move || button().accessibility_label(move ||row().map(|row|format!("Open folder {}",row.path).into())).disabled(move ||busy.get()).on_press(move || {if let Some(row)=row(){flat.set(false);enter(row.path);}}).px_2().py_2().child(text("›")))))
                }).estimated_item_size(px(36.0)).flex_1())
            ))
}
