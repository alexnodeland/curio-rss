//! The native application menu (Tauri 2 `muda` — no cargo feature needed).
//!
//! The menu is a *second surface over the same actions* as the keyboard
//! registry and toolbars: each custom item carries an id that is either a
//! frontend `ShortcutId` (e.g. `app.addFeed`, `view.all`) or a menu-only id
//! (`menu.docs`, `menu.reportIssue`). A click emits a [`MenuAction`] event and
//! the frontend routes the id through the same `handleShortcut` path a key
//! press would — so the menu can never drift from what the shortcuts do.
//!
//! Predefined items (About, Quit, and the Edit roles cut/copy/paste/…) are
//! handled natively by the OS; only our custom ids need routing. Labels are
//! English — the native menu bar is not part of the in-app i18n surface.

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Wry};

use crate::events::{MenuAction, emit_or_log};

/// Builds the full menu bar. Called from the Tauri builder's `.menu(...)`.
///
/// # Errors
///
/// Propagates any menu-construction failure from `muda`.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let settings = MenuItemBuilder::with_id("app.settings", "Settings…")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let app_menu = SubmenuBuilder::new(app, "Curio")
        .about(None)
        .separator()
        .item(&settings)
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let add_feed = MenuItemBuilder::with_id("app.addFeed", "Add Feed…")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let file = SubmenuBuilder::new(app, "File").item(&add_feed).build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let refresh = MenuItemBuilder::with_id("app.refreshAll", "Refresh All")
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let view = SubmenuBuilder::new(app, "View").item(&refresh).build()?;

    let all = MenuItemBuilder::with_id("view.all", "All Articles")
        .accelerator("CmdOrCtrl+1")
        .build(app)?;
    let starred = MenuItemBuilder::with_id("view.starred", "Starred")
        .accelerator("CmdOrCtrl+2")
        .build(app)?;
    let read_later = MenuItemBuilder::with_id("view.readLater", "Read Later")
        .accelerator("CmdOrCtrl+3")
        .build(app)?;
    let archived = MenuItemBuilder::with_id("view.archived", "Archived")
        .accelerator("CmdOrCtrl+4")
        .build(app)?;
    let next_unread = MenuItemBuilder::with_id("nav.nextUnread", "Next Unread").build(app)?;
    let go = SubmenuBuilder::new(app, "Go")
        .item(&all)
        .item(&starred)
        .item(&read_later)
        .item(&archived)
        .separator()
        .item(&next_unread)
        .build()?;

    let shortcuts = MenuItemBuilder::with_id("help.toggle", "Keyboard Shortcuts")
        .accelerator("?")
        .build(app)?;
    let docs = MenuItemBuilder::with_id("menu.docs", "Curio Docs").build(app)?;
    let report = MenuItemBuilder::with_id("menu.reportIssue", "Report an Issue").build(app)?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&shortcuts)
        .separator()
        .item(&docs)
        .item(&report)
        .build()?;

    MenuBuilder::new(app)
        .items(&[&app_menu, &file, &edit, &view, &go, &help])
        .build()
}

/// Routes a menu click: emit the item id so the frontend runs it through the
/// same action layer as a shortcut. Unknown/predefined ids are harmless — the
/// frontend router ignores anything it does not recognize.
// `MenuEvent` is taken by value because that is the signature Tauri's
// `Builder::on_menu_event` requires; we only need its id.
#[allow(clippy::needless_pass_by_value)]
pub fn on_event(app: &AppHandle, event: MenuEvent) {
    emit_or_log(
        app,
        &MenuAction {
            id: event.id().as_ref().to_owned(),
        },
    );
}
