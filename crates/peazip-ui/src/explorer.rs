use crate::columns::{self, Column, ColumnVisibility, ColumnWidths};
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
    pub filter: Signal<String>,
    pub wildcards: Signal<bool>,
    pub pattern: Memo<crate::name_filter::Pattern>,
    pub input: ElementRef,
    pub content: ElementRef,
    pub visible: Memo<Vec<BrowserRow>>,
    pub widths: ColumnWidths,
    pub visibility: ColumnVisibility,
    pub smart_sort: Signal<bool>,
}

impl Controls {
    pub fn new(entries: Signal<Vec<Entry>>, navigation: Signal<BrowserState>) -> Self {
        let flat = signal(false);
        let sort = signal(Sort::NameAsc);
        let smart_sort = signal(true);
        let filter = signal(String::new());
        let wildcards = signal(false);
        let pattern =
            computed(move || crate::name_filter::Pattern::new(&filter.get(), wildcards.get()));
        let visible = computed(move || {
            entries.with(|items| {
                let mut rows = navigation.with(|state| {
                    navigation::rows_with_smart_sort(
                        items,
                        state.current(),
                        flat.get(),
                        sort.get(),
                        smart_sort.get(),
                    )
                });
                pattern.with(|pattern| rows.retain(|row| pattern.matches(&row.name)));
                rows
            })
        });
        Self {
            flat,
            sort,
            draft: signal("/".into()),
            filter,
            wildcards,
            pattern,
            input: element_ref(),
            content: element_ref(),
            visible,
            widths: ColumnWidths::new(),
            visibility: ColumnVisibility::new(),
            smart_sort,
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
        filter,
        wildcards,
        pattern,
        input: path_input,
        content,
        visible,
        widths,
        visibility,
        smart_sort,
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
    let filter_input = element_ref();
    let wide = signal(false);
    let packed_order = move || matches!(sort.get(), Sort::PackedAsc | Sort::PackedDesc);
    let show_type = move || wide.get() && visibility.shown(Column::Type);
    let show_size = move || {
        visibility.shown(Column::Size)
            && (wide.get() || !visibility.shown(Column::Packed) || !packed_order())
    };
    let show_packed = move || {
        visibility.shown(Column::Packed)
            && (wide.get() || !visibility.shown(Column::Size) || packed_order())
    };
    let table_viewport = signal((0.0_f32, 0.0_f32));
    let type_width = move || {
        if show_type() {
            widths.get(Column::Type, 85.0)
        } else {
            0.0
        }
    };
    let size_width = move || {
        if !show_size() {
            0.0
        } else if wide.get() {
            widths.get(Column::Size, 110.0)
        } else {
            110.0
        }
    };
    let packed_width = move || {
        if !show_packed() {
            0.0
        } else if wide.get() {
            widths.get(Column::Packed, 110.0)
        } else {
            110.0
        }
    };
    let name_width = move || {
        let remaining =
            table_viewport.get().0 - 36.0 - type_width() - size_width() - packed_width();
        if wide.get() {
            widths.get(Column::Name, remaining.max(160.0))
        } else {
            remaining.max(0.0)
        }
    };
    let table_width = move || {
        if wide.get() {
            name_width() + type_width() + size_width() + packed_width() + 36.0
        } else {
            table_viewport.get().0
        }
    };
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
                navigation::rows_with_smart_sort(
                    items,
                    state.current(),
                    false,
                    Sort::NameAsc,
                    smart_sort.get(),
                )
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
    view().flex_1().min_w(px(0.0)).min_h(px(0.0))
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
        .child(view().flex_row().flex_wrap().items_center().gap_2().px_3().py_1().border_b_1().border_color(move || theme().border)
            .child(input().element_ref(&filter_input).label("Filter names").placeholder("Filter names").value(move || filter.get())
                .on_change(move |value: &str| {if !busy.get_untracked(){filter.set(value.chars().take(256).collect());}})
                .max_length(256).auto_capitalize(AutoCapitalize::None).auto_correct(false).submit_behavior(SubmitBehavior::Submit)
                .on_submit(move |_: &str| {filter_input.blur(); content.focus();}).disabled(move || busy.get()).flex_1().min_w(px(120.0))
                .border_1().border_color(move || theme().border).px_2().py_1())
            .child(checkbox().role(Some(AccessibilityRole::Switch)).accessibility_label("Use wildcards").checked(move || wildcards.get()).disabled(move || busy.get())
                .on_change(move |value: &bool| { if !busy.get_untracked(){wildcards.set(*value);} }).flex_row().items_center().gap_2().px_2().py_1()
                .indicator(view().size_4().border_1().border_color(move || theme().border).items_center().justify_center().child(when(move || wildcards.get(), || view().size_2().bg(move || theme().foreground))))
                .child(text("Use wildcards")))
            .child(button().accessibility_label("Clear filter").disabled(move || busy.get() || filter.with(String::is_empty))
                .on_press(move || {if !busy.get_untracked(){filter.set(String::new());filter_input.blur();content.focus();}}).px_2().py_1().child(text("Clear filter")))
            .child(text(move || format!("{} of {} rows shown", visible.with(Vec::len), entries.with(|items| navigation.with(|state| navigation::rows(items,state.current(),flat.get(),Sort::NameAsc).len()))))))
        .child(when(move || pattern.with(|pattern| pattern.error().is_some()), move || text(move || pattern.with(|pattern| pattern.error().unwrap_or_default().to_owned())).px_3().py_1().text_color(move || theme().destructive)))
        .child(when(move || !filter.with(String::is_empty), move || text(move || if wildcards.get() {
            "Name masks use *, ?, [abc], [a-z] and [!abc] (case insensitive). Matching folders include their contents. Hidden selections are retained."
        } else { "Names contain the text (case insensitive). Matching folders include their contents. Hidden selections are retained." }).px_3().py_1().text_color(move || theme().muted_foreground)))
        .child(toolbar().flex_row().flex_wrap().gap_3().px_3().py_1().border_b_1().border_color(move || theme().border)
            .child(commands::button(&toggle_flat).child(text(move || if flat.get() { "Folder view" } else { "All entries" })))
            .child(commands::button(&select_all).child(text("Select all")))
            .child(commands::button(&invert).child(text("Invert selection")))
            .child(commands::button(&clear_selection).child(text("Clear selection")))
            .child(text(move || format!("{} selected",selected.with(BTreeSet::len)))))
        .child(view().flex_row().flex_1().min_w(px(0.0)).min_h(px(0.0))
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
                .child(view().relative().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden()
                    .on_layout(move |bounds: &Bounds<Points>| table_viewport.set((bounds.size.width.0, bounds.size.height.0)))
                    .child(scroll().horizontal().label("Archive table").absolute().inset_0().scrollbar(move || wide.get())
                        .child(view().w(move || px(table_width())).h(move || px(table_viewport.get().1)).min_h(px(0.0))
                .child(view().flex_row().flex_shrink(0.0).py_1().bg(move || theme().muted).border_b_1().border_color(move || theme().border)
                    .child(columns::header(widths, Column::Name, sort_name, "Name", name_width, busy, wide))
                    .child(when(show_type, move || columns::header(widths, Column::Type, sort_type.clone(), "Type", type_width, busy, wide)))
                    .child(when(show_size, move || columns::header(widths, Column::Size, sort_size.clone(), "Size (bytes)", size_width, busy, wide)))
                    .child(when(show_packed, move || columns::header(widths, Column::Packed, sort_packed.clone(), "Packed (bytes)", packed_width, busy, wide)))
                    .child(view().w(px(36.0)).flex_shrink(0.0)))
                .child(virtual_list(move || visible.with(Vec::len), move |at: Position| {
                    let row=move || visible.with(|rows| rows.get(at.index()).cloned());
                    let selection = move || row().map_or((false, false), |row| entries.with(|items| selected.with(|chosen| navigation::selection_state(&row, items, chosen))));
                    let checked = move || selection().0;
                    let mixed = move || selection().1;
                    view().flex_row().items_center().w(move || px(table_width())).border_b_1().border_color(move || theme().border)
                        .bg(move || if checked() {theme().muted}else{theme().background})
                        .child(checkbox().accessibility_label(move || row().map(|row| row.path.into())).checked(checked).indeterminate(mixed).disabled(move || busy.get())
                            .on_change(move |value: &bool| {
                                if busy.get_untracked() {return;}
                                if let Some(row)=row() {
                                    entries.with(|items| selected.update(|chosen| navigation::set_selected(&row, *value, items, chosen)));
                                }
                            }).flex_row().items_center().w(move || px((table_width() - 36.0).max(0.0))).flex_shrink(0.0).min_w(px(0.0))
                            .indicator_class("size-4 ml-3 shrink-0 border border-border bg-background")
                            .indicator(view().size_4().items_center().justify_center().child(when(checked, move || view().w(px(8.0)).h(px(8.0)).bg(move || theme().foreground))).child(when(mixed, move || view().w(px(8.0)).h(px(2.0)).bg(move || theme().foreground))))
                            .child(view().flex_row().items_center().gap_2().pl_2().pr_2().py_2().w(move || px((name_width() - 28.0).max(0.0))).flex_shrink(0.0).min_w(px(0.0))
                                .child(when(move || row().is_some_and(|row|row.is_dir), || crate::icons::icon("folder")))
                                .child(when(move || row().is_some_and(|row|!row.is_dir), || crate::icons::icon("file")))
                                .child(text(move || row().map(|row|row.name).unwrap_or_default()).number_of_lines(1).flex_1().min_w(px(0.0))))
                            .child(when(show_type, move || text(move || row().map(|row| row.kind_label()).unwrap_or_default()).number_of_lines(1).w(move || px(type_width())).flex_shrink(0.0).px_2()))
                            .child(when(show_size, move || text(move || row().map(|row| if row.is_dir {"—".into()}else{row.size.to_string()}).unwrap_or_default())
                                .label(move || row().map(|row| if row.is_dir {"Size in bytes: not applicable".into()} else {format!("Size in bytes: {}",row.size)}).unwrap_or_default()).number_of_lines(1).w(move || px(size_width())).flex_shrink(0.0).px_2()))
                            .child(when(show_packed, move || text(move || row().and_then(|row|row.packed_size).map(|size|size.to_string()).unwrap_or_else(||"—".into()))
                                .label(move || row().map(|row| if row.is_dir {"Packed bytes: not applicable".into()} else {row.packed_size.map(|size|format!("Packed bytes: {size}")).unwrap_or_else(||"Packed bytes: unavailable".into())}).unwrap_or_default()).number_of_lines(1).w(move || px(packed_width())).flex_shrink(0.0).px_2())))
                        .child(view().w(px(36.0)).flex_shrink(0.0).child(when(move || row().is_some_and(|row|row.is_dir), move || button().accessibility_label(move ||row().map(|row|format!("Open folder {}",row.path).into())).disabled(move ||busy.get()).on_press(move || {if let Some(row)=row(){flat.set(false);enter(row.path);}}).px_2().py_2().child(text("›")))))
                }).estimated_item_size(px(36.0)).flex_1().min_h(px(0.0)))
                    )))
            ))
}
