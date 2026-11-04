use std::cell::RefCell;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use color_eyre::eyre::{Result, eyre};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{MainThreadMarker, NSData, NSObject, NSSize, NSString};

use crate::app_settings::AppSettings;

const IDLE_ICON_PNG: &[u8] = include_bytes!("../../assets/icons/menubar-idle@2x.png");
const STATUS_ICON_POINTS: f64 = 18.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusItemAction {
    OpenSettings,
    OpenModels,
    PasteLast,
    /// Copy History entry `id` (Labs › Copy recent dictations).
    CopyRecentDictation(u64),
    /// The update item: check, download, or restart into an update.
    Update,
    Quit,
}

#[derive(Debug)]
struct StatusItemTargetIvars {
    actions: SyncSender<StatusItemAction>,
}

objc2::define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = StatusItemTargetIvars]
    struct StatusItemTarget;

    unsafe impl NSObjectProtocol for StatusItemTarget {}

    impl StatusItemTarget {
        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(StatusItemAction::OpenSettings);
        }

        #[unsafe(method(openModels:))]
        fn open_models(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(StatusItemAction::OpenModels);
        }

        #[unsafe(method(pasteLast:))]
        fn paste_last(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(StatusItemAction::PasteLast);
        }

        #[unsafe(method(copyRecentDictation:))]
        fn copy_recent_dictation(&self, sender: &NSMenuItem) {
            if let Ok(id) = u64::try_from(sender.tag()) {
                let _ = self
                    .ivars()
                    .actions
                    .try_send(StatusItemAction::CopyRecentDictation(id));
            }
        }

        /// The status menu's delegate: it is about to open.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, _menu: &NSMenu) {
            refresh_recent_dictations();
        }

        #[unsafe(method(checkForUpdates:))]
        fn check_for_updates(&self, _sender: &AnyObject) {
            let _ = self
                .ivars()
                .actions
                .try_send(StatusItemAction::Update);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            let _ = self.ivars().actions.try_send(StatusItemAction::Quit);
        }
    }
);

impl StatusItemTarget {
    fn new(actions: SyncSender<StatusItemAction>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(StatusItemTargetIvars { actions });
        unsafe { msg_send![super(this), init] }
    }
}

struct StatusItemController {
    item: Retained<NSStatusItem>,
    /// Labs › Copy recent dictations: hidden until the menu opens with it on.
    recent: Retained<NSMenuItem>,
    recent_menu: Retained<NSMenu>,
    update: Option<Retained<NSMenuItem>>,
    update_title: RefCell<String>,
    _menu: Retained<NSMenu>,
    target: Retained<StatusItemTarget>,
    transcription: Retained<NSMenuItem>,
}

thread_local! {
    static STATUS_ITEM: RefCell<Option<StatusItemController>> = const { RefCell::new(None) };
}

pub fn install() -> Result<Receiver<StatusItemAction>> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| eyre!("status item requires the main thread"))?;
    let (actions, receiver) = sync_channel(8);
    let target = StatusItemTarget::new(actions, mtm);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Mumble"));
    add_item(
        &menu,
        &target,
        "Paste Last Dictation",
        sel!(pasteLast:),
        mtm,
    );
    let recent_menu = NSMenu::initWithTitle(
        NSMenu::alloc(mtm),
        &NSString::from_str("Copy Recent Dictation"),
    );
    let recent = plain_item("Copy Recent Dictation", mtm);
    recent.setSubmenu(Some(&recent_menu));
    recent.setHidden(true);
    menu.addItem(&recent);
    add_item(&menu, &target, "Settings…", sel!(openSettings:), mtm);
    // Read-only line showing the transcription service and language; opens
    // Settings › General, where it is chosen.
    let transcription = add_item(&menu, &target, "Transcription…", sel!(openModels:), mtm);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let update = if crate::updater::managed_by_policy() {
        // An item without an action is shown disabled.
        menu.addItem(&plain_item(crate::updater::MANAGED_LABEL, mtm));
        None
    } else {
        Some(add_item(
            &menu,
            &target,
            &update_title(&crate::desktop::host::DesktopUpdateStatus::Checking),
            sel!(checkForUpdates:),
            mtm,
        ))
    };
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_item(
        &menu,
        &target,
        &format!("Quit {}", crate::app_paths::app_name()),
        sel!(quit:),
        mtm,
    );

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-2.0);
    let button = item
        .button(mtm)
        .ok_or_else(|| eyre!("status item button is unavailable"))?;
    let symbol = status_bar_icon(IDLE_ICON_PNG, "Mumble")?;
    button.setImage(Some(&symbol));
    // SAFETY: the target answers `menuNeedsUpdate:` and outlives the menu's
    // weak delegate reference (both live in STATUS_ITEM for the app's life).
    unsafe {
        let _: () = msg_send![&*menu, setDelegate: &*target];
    }
    item.setMenu(Some(&menu));
    // Labs › Hide menu bar icon: the item stays installed, only hidden.
    item.setVisible(!crate::labs::hide_menu_bar_icon());

    STATUS_ITEM.with(|status_item| {
        *status_item.borrow_mut() = Some(StatusItemController {
            item,
            recent,
            recent_menu,
            update,
            update_title: RefCell::new("Check for Updates…".into()),
            _menu: menu,
            target,
            transcription,
        });
    });
    Ok(receiver)
}

fn status_bar_icon(png: &[u8], accessibility_description: &str) -> Result<Retained<NSImage>> {
    let data = NSData::with_bytes(png);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| eyre!("could not decode Mumble status item icon"))?;
    image.setSize(NSSize::new(STATUS_ICON_POINTS, STATUS_ICON_POINTS));
    image.setTemplate(true);
    image.setAccessibilityDescription(Some(&NSString::from_str(accessibility_description)));
    Ok(image)
}

fn plain_item(title: &str, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            None,
            &NSString::new(),
        )
    }
}

fn add_item(
    menu: &NSMenu,
    target: &StatusItemTarget,
    title: &str,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::new(),
        )
    };
    unsafe { item.setTarget(Some(target)) };
    menu.addItem(&item);
    item
}

pub fn installed() -> bool {
    STATUS_ITEM.with(|controller| controller.borrow().is_some())
}

pub fn update_transcription(settings: &AppSettings) {
    STATUS_ITEM.with(|controller| {
        let controller = controller.borrow();
        let Some(controller) = controller.as_ref() else {
            return;
        };
        let selection = &settings.transcription;
        controller
            .transcription
            .setTitle(&NSString::from_str(&format!(
                "Transcription: {} ({})…",
                selection.provider.name(),
                selection.language,
            )));
    });
}

/// A title short enough for one menu line.
fn menu_title(title: &str) -> String {
    const MAX_CHARS: usize = 48;
    let title = title.trim();
    if title.chars().count() <= MAX_CHARS {
        return title.into();
    }
    let cut = title.chars().take(MAX_CHARS - 1).collect::<String>();
    format!("{}…", cut.trim_end())
}

/// How many dictations Copy Recent Dictation lists.
const RECENT_DICTATIONS: usize = 5;

/// A dictation on one menu line: whitespace and line breaks collapsed, then
/// cut to fit.
fn recent_title(text: &str) -> String {
    menu_title(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Rebuilds Copy Recent Dictation as the menu opens. History is read only
/// while Labs › Copy recent dictations is on; the item stays hidden while
/// it is off, History is off, or nothing is listed.
fn refresh_recent_dictations() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let entries = if crate::labs::recent_dictations() {
        crate::macos_rune::runtime()
            .and_then(|runtime| runtime.history())
            .map(|history| history.recent(RECENT_DICTATIONS))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    STATUS_ITEM.with(|controller| {
        let controller = controller.borrow();
        let Some(controller) = controller.as_ref() else {
            return;
        };
        controller.recent_menu.removeAllItems();
        let mut listed = false;
        for entry in &entries {
            let Ok(tag) = isize::try_from(entry.id) else {
                continue;
            };
            let item = add_item(
                &controller.recent_menu,
                &controller.target,
                &recent_title(&entry.final_text),
                sel!(copyRecentDictation:),
                mtm,
            );
            item.setTag(tag);
            listed = true;
        }
        controller.recent.setHidden(!listed);
    });
}

fn update_title(status: &crate::desktop::host::DesktopUpdateStatus) -> String {
    use crate::desktop::host::DesktopUpdateStatus;
    match status {
        DesktopUpdateStatus::ReadyToRestart => "Restart to Update".into(),
        DesktopUpdateStatus::Available => "Download Update…".into(),
        DesktopUpdateStatus::SwitchAvailable { channel, version } => {
            format!("{}…", crate::updater::switch_label(channel, version))
        }
        _ => "Check for Updates…".into(),
    }
}

/// Retitles the update item after the updater's status, so an update ready
/// or available stays reachable from the menu bar.
pub fn update_software(status: &crate::desktop::host::DesktopUpdateStatus) {
    let title = update_title(status);
    STATUS_ITEM.with(|controller| {
        let controller = controller.borrow();
        let Some(controller) = controller.as_ref() else {
            return;
        };
        if let Some(item) = &controller.update
            && *controller.update_title.borrow() != title
        {
            item.setTitle(&NSString::from_str(&title));
            controller.update_title.replace(title);
        }
    });
}

/// Labs › Hide menu bar icon: the item stays installed, only hidden.
pub fn update_visibility() {
    STATUS_ITEM.with(|controller| {
        let controller = controller.borrow();
        let Some(controller) = controller.as_ref() else {
            return;
        };
        let shown = !crate::labs::hide_menu_bar_icon();
        if controller.item.isVisible() != shown {
            controller.item.setVisible(shown);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_dictations_show_on_one_short_line() {
        assert_eq!(
            recent_title("  Hello\nthere,\n\n  world  "),
            "Hello there, world"
        );
        let long = recent_title(
            "A long dictation that goes on\nand on well past what fits on one menu line",
        );
        assert_eq!(long, "A long dictation that goes on and on well past…");
        assert_eq!(long.chars().count(), 47);
    }

    #[test]
    fn the_status_bar_icon_loads_as_an_18pt_retina_template() {
        let icon = status_bar_icon(IDLE_ICON_PNG, "Mumble").unwrap();
        assert!(icon.isTemplate());
        assert_eq!(
            icon.size(),
            NSSize::new(STATUS_ICON_POINTS, STATUS_ICON_POINTS)
        );
        assert_eq!(
            icon.accessibilityDescription().unwrap().to_string(),
            "Mumble"
        );
        let rep = icon.representations().firstObject().unwrap();
        assert_eq!(rep.pixelsWide(), 36);
        assert_eq!(rep.pixelsHigh(), 36);
    }
}
