//! The status item on Linux: the little of `tray-icon` the app uses, as a StatusNotifierItem and its
//! D-Bus menu (`ksni`), which is what panels show: KDE's, Ubuntu's and Cinnamon's, and GNOME's with
//! the AppIndicator extension. `tray-icon` would bring GTK and libappindicator. A desktop without one
//! has no status item: the shortcuts, the command and the asides window still do everything.

use std::sync::{Arc, Mutex, OnceLock};

use ksni::blocking::{Handle, TrayMethods};

/// The status item's picture, as StatusNotifierItem wants it.
#[derive(Clone)]
pub struct Icon(ksni::Icon);

impl Icon {
    /// From straight-alpha RGBA pixels.
    pub fn from_rgba(rgba: Vec<u8>, width: u32, height: u32) -> Result<Icon, String> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err("the icon's size doesn't match its pixels".into());
        }
        let data = rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [a, r, g, b]).collect();
        Ok(Icon(ksni::Icon { width: width as i32, height: height as i32, data }))
    }
}

/// `tray-icon`'s events about the icon itself. Panels here only open its menu: none come.
pub struct TrayIconEvent;

impl TrayIconEvent {
    pub fn set_event_handler<F: Fn(TrayIconEvent) + Send + Sync + 'static>(_: Option<F>) {}
}

pub mod menu {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MenuId(usize);

    fn next_id() -> MenuId {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        MenuId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    /// An item of the menu was chosen.
    #[derive(Debug)]
    pub struct MenuEvent {
        pub id: MenuId,
    }

    type Handler = Box<dyn Fn(MenuEvent) + Send + Sync>;
    static HANDLER: OnceLock<Handler> = OnceLock::new();

    impl MenuEvent {
        pub fn set_event_handler<F: Fn(MenuEvent) + Send + Sync + 'static>(handler: Option<F>) {
            if let Some(handler) = handler {
                let _ = HANDLER.set(Box::new(handler));
            }
        }
    }

    pub(super) fn chosen(id: MenuId) {
        if let Some(handler) = HANDLER.get() {
            handler(MenuEvent { id });
        }
    }

    pub mod accelerator {
        /// `tray-icon`'s shortcuts beside an item: only a Mac's menus show them.
        pub struct Accelerator;

        impl std::str::FromStr for Accelerator {
            type Err = ();

            fn from_str(_: &str) -> Result<Accelerator, ()> {
                Ok(Accelerator)
            }
        }
    }
    use accelerator::Accelerator;

    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Kind {
        Standard,
        Check,
        Separator,
    }

    #[derive(Clone, PartialEq)]
    pub(super) struct Item {
        pub id: MenuId,
        pub kind: Kind,
        pub text: String,
        pub enabled: bool,
        pub checked: bool,
    }

    /// An item, shared with the menu that has it: changing it updates the panel's menu.
    #[derive(Clone)]
    pub struct Entry {
        id: MenuId,
        item: Arc<Mutex<Item>>,
    }

    impl Entry {
        fn new(kind: Kind, text: &str, enabled: bool, checked: bool) -> Entry {
            let id = next_id();
            let item = Item { id: id.clone(), kind, text: label(text), enabled, checked };
            Entry { id, item: Arc::new(Mutex::new(item)) }
        }

        pub fn id(&self) -> &MenuId {
            &self.id
        }

        fn change(&self, change: impl FnOnce(&mut Item)) {
            let mut item = self.item.lock().unwrap();
            let before = item.clone();
            change(&mut item);
            let changed = *item != before;
            drop(item);
            if changed {
                refresh();
            }
        }

        pub fn set_text(&self, text: impl AsRef<str>) {
            let text = label(text.as_ref());
            self.change(|item| item.text = text);
        }

        pub fn set_enabled(&self, enabled: bool) {
            self.change(|item| item.enabled = enabled);
        }

        pub fn set_checked(&self, checked: bool) {
            self.change(|item| item.checked = checked);
        }

        pub fn set_accelerator(&self, _: Option<Accelerator>) -> Result<(), String> {
            Ok(())
        }
    }

    /// `tray.rs` writes a menu title as muda wants it (`&&` for `&`). D-Bus menus take `__` for `_`.
    fn label(text: &str) -> String {
        text.replace("&&", "&").replace('_', "__")
    }

    /// Anything the menu can show.
    pub trait IsMenuItem {
        fn entry(&self) -> &Entry;
    }

    macro_rules! item {
        ($name:ident) => {
            pub struct $name(Entry);

            impl std::ops::Deref for $name {
                type Target = Entry;

                fn deref(&self) -> &Entry {
                    &self.0
                }
            }

            impl IsMenuItem for $name {
                fn entry(&self) -> &Entry {
                    &self.0
                }
            }
        };
    }
    item!(MenuItem);
    item!(CheckMenuItem);
    item!(PredefinedMenuItem);

    impl MenuItem {
        pub fn new(text: &str, enabled: bool, _: Option<Accelerator>) -> MenuItem {
            MenuItem(Entry::new(Kind::Standard, text, enabled, false))
        }
    }

    impl CheckMenuItem {
        pub fn new(text: &str, enabled: bool, checked: bool, _: Option<Accelerator>) -> CheckMenuItem {
            CheckMenuItem(Entry::new(Kind::Check, text, enabled, checked))
        }
    }

    impl PredefinedMenuItem {
        pub fn separator() -> PredefinedMenuItem {
            PredefinedMenuItem(Entry::new(Kind::Separator, "", true, false))
        }
    }

    /// The menu, shared with what shows it.
    #[derive(Clone, Default)]
    pub struct Menu(Arc<Mutex<Vec<Entry>>>);

    impl Menu {
        pub fn new() -> Menu {
            Menu::default()
        }

        pub fn append_items(&self, items: &[&dyn IsMenuItem]) -> anyhow::Result<()> {
            self.0.lock().unwrap().extend(items.iter().map(|i| i.entry().clone()));
            refresh();
            Ok(())
        }

        pub fn insert(&self, item: &dyn IsMenuItem, at: usize) -> anyhow::Result<()> {
            let mut items = self.0.lock().unwrap();
            let at = at.min(items.len());
            items.insert(at, item.entry().clone());
            drop(items);
            refresh();
            Ok(())
        }

        pub fn remove(&self, item: &dyn IsMenuItem) -> anyhow::Result<()> {
            self.0.lock().unwrap().retain(|i| i.id != item.entry().id);
            refresh();
            Ok(())
        }

        pub(super) fn items(&self) -> Vec<Item> {
            self.0.lock().unwrap().iter().map(|e| e.item.lock().unwrap().clone()).collect()
        }
    }
}

use menu::{Kind, Menu};

/// The item on the panel, as the D-Bus service has it.
struct Service {
    menu: Menu,
    icon: Option<Icon>,
    title: String,
}

impl ksni::Tray for Service {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "asides".into()
    }

    fn title(&self) -> String {
        "asides".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.iter().map(|i| i.0.clone()).collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.title.clone(), ..Default::default() }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.menu
            .items()
            .into_iter()
            .map(|item| {
                let id = item.id.clone();
                let activate = move |_: &mut Service| menu::chosen(id.clone());
                match item.kind {
                    Kind::Separator => ksni::MenuItem::Separator,
                    Kind::Standard => ksni::menu::StandardItem {
                        label: item.text,
                        enabled: item.enabled,
                        activate: Box::new(activate),
                        ..Default::default()
                    }
                    .into(),
                    Kind::Check => ksni::menu::CheckmarkItem {
                        label: item.text,
                        enabled: item.enabled,
                        checked: item.checked,
                        activate: Box::new(activate),
                        ..Default::default()
                    }
                    .into(),
                }
            })
            .collect()
    }
}

/// The service, once it runs: what changes in the menu goes to the panel through it.
static SERVICE: OnceLock<Handle<Service>> = OnceLock::new();

/// Tells the panel the menu changed.
fn refresh() {
    if let Some(service) = SERVICE.get() {
        service.update(|_| {});
    }
}

pub struct TrayIconBuilder {
    menu: Option<Menu>,
    icon: Option<Icon>,
    tooltip: String,
}

impl TrayIconBuilder {
    pub fn new() -> TrayIconBuilder {
        TrayIconBuilder { menu: None, icon: None, tooltip: String::new() }
    }

    #[allow(clippy::boxed_local, reason = "tray-icon's signature")]
    pub fn with_menu(mut self, menu: Box<Menu>) -> Self {
        self.menu = Some(*menu);
        self
    }

    pub fn with_menu_on_left_click(self, _: bool) -> Self {
        self
    }

    pub fn with_tooltip(mut self, tooltip: &str) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Fails on a desktop that shows no status items (GNOME without the AppIndicator extension):
    /// the app goes on without one.
    pub fn build(self) -> anyhow::Result<TrayIcon> {
        let service = Service { menu: self.menu.unwrap_or_default(), icon: self.icon, title: self.tooltip };
        let handle = service.spawn().map_err(|err| anyhow::anyhow!("{err}"))?;
        let _ = SERVICE.set(handle.clone());
        Ok(TrayIcon(handle))
    }
}

pub struct TrayIcon(Handle<Service>);

impl TrayIcon {
    pub fn set_icon(&self, icon: Option<Icon>) -> Result<(), String> {
        self.0.update(|service| service.icon = icon);
        Ok(())
    }

    /// Panels show no text beside an item: what the Mac's shows there goes in its tooltip.
    pub fn set_title(&self, title: Option<&str>) {
        let title = title.map_or_else(|| "asides".to_string(), |t| format!("asides {t}"));
        self.0.update(|service| service.title = title);
    }
}
