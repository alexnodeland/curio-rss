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
//! handled natively by the OS; only our custom ids need routing. Every
//! label, predefined ones included, comes from the [`crate::l10n`] table in
//! the user's `ui.locale`: the menu is first built in English (before the
//! profile opens), then rebuilt in the chosen language once the core is up
//! and again whenever the setting changes ([`apply_locale`]).

use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Wry};

use crate::events::{MenuAction, emit_or_log};
use crate::l10n::Locale;

/// Builds the English menu bar. Called from the Tauri builder's
/// `.menu(...)`, before any profile (and so any locale) is known.
///
/// # Errors
///
/// Propagates any menu-construction failure from `muda`.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    build_for(app, Locale::En)
}

/// Rebuilds the menu bar in `locale` and installs it. Best-effort: a
/// failure is logged and the current menu stays.
pub fn apply_locale(app: &AppHandle, locale: Locale) {
    if let Err(error) = build_for(app, locale).and_then(|menu| app.set_menu(menu)) {
        tracing::warn!(%error, ?locale, "could not rebuild the menu in the chosen language");
    }
}

/// Builds the full menu bar with `locale`'s labels.
///
/// # Errors
///
/// Propagates any menu-construction failure from `muda`.
pub fn build_for(app: &AppHandle, locale: Locale) -> tauri::Result<Menu<Wry>> {
    let text = &locale.strings().menu;
    let settings = MenuItemBuilder::with_id("app.settings", text.settings)
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let app_menu = SubmenuBuilder::new(app, "Curio")
        .about_with_text(text.about, None)
        .separator()
        .item(&settings)
        .separator()
        .hide_with_text(text.hide)
        .hide_others_with_text(text.hide_others)
        .show_all_with_text(text.show_all)
        .separator()
        .quit_with_text(text.quit)
        .build()?;

    let add_feed = MenuItemBuilder::with_id("app.addFeed", text.add_feed)
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let file = SubmenuBuilder::new(app, text.file)
        .item(&add_feed)
        .build()?;

    let edit = SubmenuBuilder::new(app, text.edit)
        .undo_with_text(text.undo)
        .redo_with_text(text.redo)
        .separator()
        .cut_with_text(text.cut)
        .copy_with_text(text.copy)
        .paste_with_text(text.paste)
        .select_all_with_text(text.select_all)
        .build()?;

    let refresh = MenuItemBuilder::with_id("app.refreshAll", text.refresh_all)
        .accelerator("CmdOrCtrl+R")
        .build(app)?;
    let view = SubmenuBuilder::new(app, text.view).item(&refresh).build()?;

    let all = MenuItemBuilder::with_id("view.all", text.all_articles)
        .accelerator("CmdOrCtrl+1")
        .build(app)?;
    let starred = MenuItemBuilder::with_id("view.starred", text.starred)
        .accelerator("CmdOrCtrl+2")
        .build(app)?;
    let read_later = MenuItemBuilder::with_id("view.readLater", text.read_later)
        .accelerator("CmdOrCtrl+3")
        .build(app)?;
    let archived = MenuItemBuilder::with_id("view.archived", text.archived)
        .accelerator("CmdOrCtrl+4")
        .build(app)?;
    let next_unread = MenuItemBuilder::with_id("nav.nextUnread", text.next_unread).build(app)?;
    let go = SubmenuBuilder::new(app, text.go)
        .item(&all)
        .item(&starred)
        .item(&read_later)
        .item(&archived)
        .separator()
        .item(&next_unread)
        .build()?;

    let shortcuts = MenuItemBuilder::with_id("help.toggle", text.shortcuts)
        .accelerator("?")
        .build(app)?;
    let docs = MenuItemBuilder::with_id("menu.docs", text.docs).build(app)?;
    let report = MenuItemBuilder::with_id("menu.reportIssue", text.report_issue).build(app)?;
    let help = SubmenuBuilder::new(app, text.help)
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
