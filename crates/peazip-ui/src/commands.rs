use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    Open,
    OpenFolder,
    NewFolder,
    CopySelected,
    OpenSelected,
    RefreshFolder,
    FilesystemRoot,
    CreateZip,
    CreateFolderZip,
    SaveArchive,
    CloseArchive,
    Extract,
    Test,
    Cancel,
    SelectAll,
    InvertSelection,
    ClearSelection,
    Back,
    Forward,
    Up,
    Root,
    FocusPath,
    ToggleFlat,
    SortName,
    SortSize,
    SortType,
    SortPacked,
    About,
}

#[derive(Clone)]
pub struct Command {
    pub id: CommandId,
    pub label: &'static str,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    enabled: Rc<dyn Fn() -> bool>,
    run: Rc<dyn Fn()>,
    checked: Option<Rc<dyn Fn() -> bool>>,
    owner: Option<nsh_core::prelude::Owner>,
}

impl Command {
    pub fn new(
        id: CommandId,
        label: &'static str,
        enabled: impl Fn() -> bool + 'static,
        run: impl Fn() + 'static,
    ) -> Self {
        Self {
            id,
            label,
            icon: None,
            shortcut: None,
            enabled: Rc::new(enabled),
            run: Rc::new(run),
            checked: None,
            owner: None,
        }
    }
    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn shortcut(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
    pub fn enabled(&self) -> bool {
        (self.enabled)()
    }
    pub fn checked(mut self, checked: impl Fn() -> bool + 'static) -> Self {
        self.checked = Some(Rc::new(checked));
        self
    }
    pub fn is_checked(&self) -> Option<bool> {
        self.checked.as_ref().map(|checked| checked())
    }
    pub fn in_owner(mut self, owner: nsh_core::prelude::Owner) -> Self {
        self.owner = Some(owner);
        self
    }
    pub fn unless(mut self, blocked: impl Fn() -> bool + 'static) -> Self {
        let enabled = self.enabled;
        self.enabled = Rc::new(move || !blocked() && enabled());
        self
    }
    pub fn invoke(&self) {
        let run = || {
            if self.enabled() {
                (self.run)();
            }
        };
        if let Some(owner) = self.owner {
            owner.run(run);
        } else {
            run();
        }
    }
}

#[derive(Clone)]
pub struct Commands(Rc<Vec<Command>>);

impl Commands {
    pub fn new(commands: Vec<Command>) -> Self {
        Self(Rc::new(commands))
    }
    pub fn get(&self, id: CommandId) -> Option<Command> {
        self.0.iter().find(|command| command.id == id).cloned()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Command> {
        self.0.iter()
    }
}

pub(crate) fn button(command: &Command) -> nsh_base::ButtonElement {
    let eligibility = command.clone();
    let action = command.clone();
    nsh_base::button()
        .disabled(move || !eligibility.enabled())
        .on_press(move || action.invoke())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn modal_gate_blocks_existing_command_clones_until_dismissed() {
        let modal = Rc::new(Cell::new(false));
        let calls = Rc::new(Cell::new(0));
        let command = Command::new(CommandId::CreateZip, "Create", || true, {
            let calls = calls.clone();
            move || calls.set(calls.get() + 1)
        })
        .unless({
            let modal = modal.clone();
            move || modal.get()
        });
        let native_menu_item = command.clone();
        modal.set(true);
        assert!(!native_menu_item.enabled());
        native_menu_item.invoke();
        assert_eq!(calls.get(), 0);
        modal.set(false);
        native_menu_item.invoke();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn clones_share_live_eligibility_and_actions_without_stale_invocations() {
        let eligible = Rc::new(Cell::new(true));
        let calls = Rc::new(Cell::new(0));
        let command = Command::new(
            CommandId::Open,
            "Open archive…",
            {
                let eligible = eligible.clone();
                move || eligible.get()
            },
            {
                let calls = calls.clone();
                move || calls.set(calls.get() + 1)
            },
        );
        assert_eq!(command.icon, None);
        assert_eq!(command.shortcut, None);
        let command = command.icon("folder-open").shortcut("Ctrl+O");
        let cloned = command.clone();
        assert!(Rc::ptr_eq(&command.enabled, &cloned.enabled));
        assert!(Rc::ptr_eq(&command.run, &cloned.run));
        assert_eq!(cloned.id, CommandId::Open);
        assert_eq!(cloned.label, "Open archive…");
        assert_eq!(cloned.icon, Some("folder-open"));
        assert_eq!(cloned.shortcut, Some("Ctrl+O"));
        assert!(cloned.enabled());
        eligible.set(false);
        command.invoke();
        cloned.invoke();
        assert_eq!(calls.get(), 0);
        assert!(!cloned.enabled());
        eligible.set(true);
        command.invoke();
        cloned.invoke();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn checked_state_is_optional_live_and_shared_by_clones() {
        let command = Command::new(CommandId::ToggleFlat, "Flat view", || true, || {});
        assert_eq!(command.is_checked(), None);
        let checked = Rc::new(Cell::new(false));
        let command = command.checked({
            let checked = checked.clone();
            move || checked.get()
        });
        let cloned = command.clone();
        assert_eq!(command.is_checked(), Some(false));
        checked.set(true);
        assert_eq!(cloned.is_checked(), Some(true));
        checked.set(false);
        assert_eq!(command.is_checked(), Some(false));
    }

    #[test]
    fn command_collection_preserves_order_and_shares_commands_and_storage() {
        let eligible = Rc::new(Cell::new(true));
        let command = Command::new(
            CommandId::Open,
            "Open",
            {
                let eligible = eligible.clone();
                move || eligible.get()
            },
            || {},
        );
        let commands = Commands::new(vec![
            command,
            Command::new(CommandId::About, "About", || true, || {}),
        ]);
        let cloned = commands.clone();
        assert!(Rc::ptr_eq(&commands.0, &cloned.0));
        assert_eq!(
            cloned.iter().map(|command| command.id).collect::<Vec<_>>(),
            [CommandId::Open, CommandId::About]
        );
        let open = cloned.get(CommandId::Open).unwrap();
        assert!(Rc::ptr_eq(&commands.0[0].enabled, &open.enabled));
        eligible.set(false);
        assert!(!open.enabled());
        assert!(commands.get(CommandId::Extract).is_none());
        assert!(Commands::new(vec![]).iter().next().is_none());
    }
}
