//! The native menu bar, built from the frontend's own command list.
//!
//! Discipline (PLAN.md, C5): the menu **mirrors the command palette** rather
//! than restating it. `app.ts` owns one list of commands — which entries exist
//! at all depends on the selected change (immutable changes have no Abandon,
//! the working copy has no Mark Reviewed) — and pushes that list here whenever
//! it changes. A menu item is just a command id; clicking one emits
//! `menu-command`, and the frontend runs the command it already had. There is
//! no second definition of the command surface to drift out of step.
//!
//! Menus the palette does not own — the app menu, Edit, Window — are built
//! from Tauri's predefined items. Edit is not decoration: without it a WebView
//! on macOS loses Cmd+C/Cmd+V entirely.

use serde::Deserialize;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Runtime};

/// One palette group, verbatim — the submenu title is the group title.
#[derive(Debug, Clone, Deserialize)]
pub struct MenuGroup {
    pub title: String,
    pub items: Vec<MenuEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MenuEntry {
    /// The palette command id, echoed back in the `menu-command` event.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Ids are namespaced so a menu click cannot be confused with anything else
/// Tauri routes through the same event, and so the frontend can tell a mirrored
/// command from a structural item at a glance.
pub const PREFIX: &str = "cmd:";

pub fn build<R: Runtime>(app: &AppHandle<R>, groups: &[MenuGroup]) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;

    // App menu. macOS puts About/Services/Hide/Quit here by convention; on
    // other platforms it reads as a plain "jjdiff" menu, which is harmless.
    menu.append(&Submenu::with_items(
        app,
        "jjdiff",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About jjdiff"), Some(AboutMetadata::default()))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?)?;

    menu.append(&Submenu::with_items(
        app,
        "File",
        true,
        &[&PredefinedMenuItem::close_window(app, Some("Close Window"))?],
    )?)?;

    // Required, not cosmetic: a WebView with a custom menu and no Edit menu has
    // no Cmd+C/Cmd+V on macOS.
    menu.append(&Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?)?;

    // The mirrored half. No accelerators here on purpose: the frontend already
    // dispatches every keyboard shortcut itself, and a menu accelerator for the
    // same key would either shadow it or fire alongside it.
    for group in groups {
        if group.items.is_empty() {
            continue;
        }
        let items: Vec<MenuItem<R>> = group
            .items
            .iter()
            .map(|entry| {
                MenuItem::with_id(
                    app,
                    format!("{PREFIX}{}", entry.id),
                    &entry.label,
                    entry.enabled.unwrap_or(true),
                    None::<&str>,
                )
            })
            .collect::<tauri::Result<_>>()?;
        let refs: Vec<&dyn tauri::menu::IsMenuItem<R>> =
            items.iter().map(|item| item as &dyn tauri::menu::IsMenuItem<R>).collect();
        menu.append(&Submenu::with_items(app, &group.title, true, &refs)?)?;
    }

    menu.append(&Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?)?;

    Ok(menu)
}
