use crate::commands::{Command, CommandId, Commands};
use nsh_core::prelude::*;

const GROUPS: &[(&str, &[CommandId])] = &[
    (
        "File",
        &[
            CommandId::Open,
            CommandId::OpenFolder,
            CommandId::NewFolder,
            CommandId::OpenSelected,
            CommandId::CreateZip,
            CommandId::CreateFolderZip,
            CommandId::SaveArchive,
            CommandId::CloseArchive,
        ],
    ),
    (
        "Edit",
        &[
            CommandId::CopySelected,
            CommandId::SelectAll,
            CommandId::InvertSelection,
            CommandId::ClearSelection,
        ],
    ),
    (
        "Browser",
        &[
            CommandId::Back,
            CommandId::Forward,
            CommandId::Up,
            CommandId::Root,
            CommandId::FilesystemRoot,
            CommandId::FocusPath,
            CommandId::RefreshFolder,
        ],
    ),
    (
        "Layout",
        &[
            CommandId::ToggleFlat,
            CommandId::SortName,
            CommandId::SortSize,
            CommandId::SortType,
            CommandId::SortPacked,
            CommandId::SmartNameSorting,
            CommandId::AdjustColumns,
            CommandId::ToggleTypeColumn,
            CommandId::ToggleSizeColumn,
            CommandId::TogglePackedColumn,
        ],
    ),
    (
        "Tools",
        &[CommandId::Test, CommandId::Extract, CommandId::Cancel],
    ),
    ("Help", &[CommandId::About]),
];

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub use native::install_native;

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod native {
    use super::*;
    use nsh_core::{
        desktop::MenuRole,
        element::desktop::{MenuItem, set_application_menu},
    };

    fn item(command: Command, owner: Owner, focus_content: impl Fn() + Copy + 'static) -> MenuItem {
        let mut item = MenuItem::new(command.label).disabled(!command.enabled());
        if let Some(checked) = command.is_checked() {
            item = item.checked(checked);
        }
        if let Some(shortcut) = native_shortcut(&command) {
            item = item.shortcut(shortcut);
        }
        // Native accelerators cannot enforce NotEditing on GTK/AppKit/Win32.
        item.on_select(move || {
            owner.run(|| {
                if !command.enabled() {
                    return;
                }
                command.invoke();
                if matches!(
                    command.id,
                    CommandId::SelectAll
                        | CommandId::InvertSelection
                        | CommandId::ClearSelection
                        | CommandId::ToggleFlat
                        | CommandId::SortName
                        | CommandId::SortSize
                        | CommandId::SortType
                        | CommandId::SortPacked
                        | CommandId::AdjustColumns
                        | CommandId::SmartNameSorting
                        | CommandId::ToggleTypeColumn
                        | CommandId::ToggleSizeColumn
                        | CommandId::TogglePackedColumn
                        | CommandId::Test
                        | CommandId::CloseArchive
                        | CommandId::Back
                        | CommandId::Forward
                        | CommandId::Up
                        | CommandId::Root
                        | CommandId::OpenSelected
                        | CommandId::RefreshFolder
                        | CommandId::FilesystemRoot
                ) {
                    after_answer(focus_content);
                }
            })
        })
    }

    pub fn install_native(commands: Commands, focus_content: impl Fn() + Copy + 'static) {
        let owner = Owner::current();
        effect(move || {
            let mut menus = Vec::new();
            if cfg!(target_os = "macos") {
                let mut app = Vec::new();
                if let Some(about) = commands.get(CommandId::About) {
                    app.push(item(about, owner, focus_content));
                    app.push(MenuItem::separator());
                }
                app.extend([
                    MenuItem::role(MenuRole::Hide),
                    MenuItem::role(MenuRole::HideOthers),
                    MenuItem::role(MenuRole::ShowAll),
                    MenuItem::separator(),
                    MenuItem::role(MenuRole::Quit),
                ]);
                menus.push(MenuItem::submenu("PeaZip", app));
            }
            for &(label, ids) in GROUPS {
                let mut items: Vec<_> = ids
                    .iter()
                    .filter_map(|&id| commands.get(id))
                    .map(|command| item(command, owner, focus_content))
                    .collect();
                if label == "File" && !cfg!(target_os = "macos") {
                    if !items.is_empty() {
                        items.push(MenuItem::separator());
                    }
                    items.push(MenuItem::role(MenuRole::Quit));
                }
                if !items.is_empty() {
                    menus.push(MenuItem::submenu(label, items));
                }
            }
            set_application_menu(menus);
        });
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn native_shortcut(command: &Command) -> Option<&'static str> {
    match command.id {
        CommandId::Open | CommandId::OpenFolder | CommandId::FocusPath => command.shortcut,
        _ => None,
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios"))]
pub use fallback::menu_bar;

#[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios"))]
mod fallback {
    use super::*;
    use nsh_base::{MenuEvent, menu, menu_item, theme};

    fn item(
        command: Command,
        active: Signal<Option<&'static str>>,
        owner: Owner,
        trigger: ElementRef,
    ) -> impl IntoElement {
        let enabled = command.clone();
        let checked = command.clone();
        let mark = command.clone();
        let mut content = view().flex_row().items_center().gap_2();
        if let Some(icon) = command.icon {
            content = content.child(crate::icons::icon(icon));
        }
        content = content.child(text(command.label).flex_1().number_of_lines(1));
        if let Some(shortcut) = command.shortcut {
            content = content.child(
                text(move || shortcut_label(shortcut, apple_keys()))
                    .ml_4()
                    .text_color(move || theme().muted_foreground),
            );
        }
        menu_item()
            .accessibility_label(command.label)
            .disabled(move || !enabled.enabled())
            .checked(move || checked.is_checked())
            .close_on_select(false)
            .flex_row()
            .items_center()
            .gap_2()
            .px_2()
            .py_2()
            .min_h(px(if is_desktop() { 32.0 } else { 44.0 }))
            .hover(|s| s.bg(theme().muted))
            .focus(|s| s.bg(theme().muted))
            .disabled_style(|s| s.opacity(0.4))
            .indicator(view().size_3().items_center().justify_center().child(when(
                move || mark.is_checked() == Some(true),
                || view().size(px(6.0)).bg(move || theme().foreground),
            )))
            .on_select(move || {
                active.set(None);
                if command.id == CommandId::About {
                    trigger.focus();
                }
                // The popup's owner ends on close; archive tasks belong to the app.
                owner.run(|| command.invoke());
            })
            .child(content)
    }

    pub fn menu_bar(commands: Commands, active: Signal<Option<&'static str>>) -> impl IntoElement {
        let owner = Owner::current();
        let mut bar = view()
            .role(AccessibilityRole::Group)
            .flex_row()
            .flex_wrap()
            .gap_1()
            .px_2()
            .py_1()
            .label("Application menus")
            .border_b_1()
            .border_color(move || theme().border);
        for &(label, ids) in GROUPS {
            let items: Vec<_> = ids.iter().filter_map(|&id| commands.get(id)).collect();
            if items.is_empty() {
                continue;
            }
            let trigger = element_ref();
            bar = bar.child(
                menu()
                    .open(move || Some(active.get() == Some(label)))
                    .on_open_change(move |event: &MenuEvent| {
                        let MenuEvent::OpenChange(open, _) = event else {
                            return;
                        };
                        if *open {
                            active.set(Some(label));
                        } else if active.get_untracked() == Some(label) {
                            active.set(None);
                        }
                    })
                    .offset(px(2.0))
                    .popup_style(|s| {
                        s.min_w(px(210.0))
                            .p_1()
                            .border_1()
                            .border_color(theme().border)
                            .bg(theme().background)
                    })
                    .trigger(
                        view()
                            .role(AccessibilityRole::Button)
                            .focusable(true)
                            .element_ref(&trigger)
                            .label(label)
                            .on_mouse_enter(move |_: &MouseEvent| {
                                if active.get_untracked().is_some() {
                                    active.set(Some(label));
                                }
                            })
                            .px_2()
                            .py_1()
                            .bg(move || {
                                if active.get() == Some(label) {
                                    theme().muted
                                } else {
                                    theme().background
                                }
                            })
                            .child(text(label)),
                    )
                    .children(
                        items
                            .into_iter()
                            .map(|command| item(command, active, owner, trigger)),
                    ),
            );
        }
        bar
    }
}

#[cfg(any(target_arch = "wasm32", target_os = "android", target_os = "ios", test))]
fn shortcut_label(shortcut: &str, apple: bool) -> String {
    shortcut
        .split('+')
        .map(|key| match key {
            "mod" => if apple { "Cmd" } else { "Ctrl" }.to_owned(),
            "alt" => if apple { "Option" } else { "Alt" }.to_owned(),
            "shift" => "Shift".to_owned(),
            "left" => "Left".to_owned(),
            "right" => "Right".to_owned(),
            "up" => "Up".to_owned(),
            "down" => "Down".to_owned(),
            "enter" => "Enter".to_owned(),
            _ => key.to_ascii_uppercase(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::shortcut_label;

    #[test]
    fn shortcut_hints_use_platform_names() {
        for (shortcut, mac, other) in [
            ("mod+a", "Cmd+A", "Ctrl+A"),
            ("mod+shift+a", "Cmd+Shift+A", "Ctrl+Shift+A"),
            ("mod+enter", "Cmd+Enter", "Ctrl+Enter"),
            ("alt+left", "Option+Left", "Alt+Left"),
            ("alt+right", "Option+Right", "Alt+Right"),
            ("alt+up", "Option+Up", "Alt+Up"),
        ] {
            assert_eq!(shortcut_label(shortcut, true), mac);
            assert_eq!(shortcut_label(shortcut, false), other);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    #[test]
    fn only_always_commands_have_native_accelerators() {
        use super::{Command, CommandId, GROUPS, native_shortcut};
        for &(_, ids) in GROUPS {
            for &id in ids {
                let command = Command::new(id, "Command", || true, || {}).shortcut("mod+a");
                let expected = matches!(
                    id,
                    CommandId::Open | CommandId::OpenFolder | CommandId::FocusPath
                )
                .then_some("mod+a");
                assert_eq!(native_shortcut(&command), expected, "{id:?}");
            }
        }
        let without_shortcut = Command::new(CommandId::Open, "Open", || true, || {});
        assert_eq!(native_shortcut(&without_shortcut), None);
    }
}
