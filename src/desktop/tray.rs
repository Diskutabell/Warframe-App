//! Linux system tray icon via D-Bus StatusNotifierItem (`org.kde.StatusNotifierItem`) and
//! DBusMenu (`com.canonical.dbusmenu`).
//!
//! Uses GTK's own GLib/GDBus session connection (`gtk::gio`), so it runs directly on the GTK main
//! loop without extra threads or dependencies and integrates natively with Quickshell's
//! `SystemTray`, Waybar, KDE Plasma, and GNOME AppIndicator.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Result, anyhow};
use gtk::gio;
use gtk::glib::{self, ToVariant, Variant, VariantDict, VariantTy};

use crate::state::State;
use crate::tray_icon;

const SNI_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

const ID_TOGGLE: i32 = 1;
const ID_SEP_1: i32 = 2;
const ID_REWARDS: i32 = 3;
const ID_RIVEN: i32 = 4;
const ID_SEP_2: i32 = 5;
const ID_QUIT: i32 = 6;

const SNI_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierItem">
    <method name="ContextMenu">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="Activate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="SecondaryActivate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="Scroll">
      <arg name="delta" type="i" direction="in"/>
      <arg name="orientation" type="s" direction="in"/>
    </method>
    <signal name="NewTitle"/>
    <signal name="NewIcon"/>
    <signal name="NewAttentionIcon"/>
    <signal name="NewOverlayIcon"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus">
      <arg name="status" type="s"/>
    </signal>
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="u" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconPixmap" type="a(iiay)" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionMovieName" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
  </interface>
</node>
"#;

const MENU_XML: &str = r#"
<node>
  <interface name="com.canonical.dbusmenu">
    <method name="GetLayout">
      <arg name="parentId" type="i" direction="in"/>
      <arg name="recursionDepth" type="i" direction="in"/>
      <arg name="propertyNames" type="as" direction="in"/>
      <arg name="revision" type="u" direction="out"/>
      <arg name="layout" type="(ia{sv}av)" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg name="ids" type="ai" direction="in"/>
      <arg name="propertyNames" type="as" direction="in"/>
      <arg name="properties" type="a(ia{sv})" direction="out"/>
    </method>
    <method name="GetProperty">
      <arg name="id" type="i" direction="in"/>
      <arg name="name" type="s" direction="in"/>
      <arg name="value" type="v" direction="out"/>
    </method>
    <method name="Event">
      <arg name="id" type="i" direction="in"/>
      <arg name="eventId" type="s" direction="in"/>
      <arg name="data" type="v" direction="in"/>
      <arg name="timestamp" type="u" direction="in"/>
    </method>
    <method name="EventGroup">
      <arg name="events" type="a(isvu)" direction="in"/>
      <arg name="idErrors" type="ai" direction="out"/>
    </method>
    <method name="AboutToShow">
      <arg name="id" type="i" direction="in"/>
      <arg name="needUpdate" type="b" direction="out"/>
    </method>
    <method name="AboutToShowGroup">
      <arg name="ids" type="ai" direction="in"/>
      <arg name="updatesNeeded" type="ai" direction="out"/>
      <arg name="idErrors" type="ai" direction="out"/>
    </method>
    <signal name="ItemsPropertiesUpdated">
      <arg name="updatedProps" type="a(ia{sv})"/>
      <arg name="removedProps" type="a(ias)"/>
    </signal>
    <signal name="LayoutUpdated">
      <arg name="revision" type="u"/>
      <arg name="parent" type="i"/>
    </signal>
    <signal name="ItemActivationRequested">
      <arg name="id" type="i"/>
      <arg name="timestamp" type="u"/>
    </signal>
    <property name="Version" type="u" access="read"/>
    <property name="TextDirection" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconThemePath" type="as" access="read"/>
  </interface>
</node>
"#;

/// Live D-Bus registrations for one active tray icon. Dropping unowns our well-known bus name so
/// `StatusNotifierWatcher` (Quickshell, Waybar, KDE) removes the icon immediately.
struct ActiveTray {
    conn: gio::DBusConnection,
    sni_reg: Option<gio::RegistrationId>,
    menu_reg: Option<gio::RegistrationId>,
    owner_id: Option<gio::OwnerId>,
    // `gio` 0.18's `bus_watch_name_on_connection` returns a `WatcherId` from a private module
    // shadowed by `dbus_connection::WatcherId`, so hold its unwatch action in a closure.
    unwatch: Option<Box<dyn FnOnce()>>,
    visible: Arc<AtomicBool>,
    rev: Arc<AtomicU32>,
}

impl ActiveTray {
    fn start(state: Arc<State>, window_visible: bool) -> Result<Self> {
        static SEQ: AtomicU32 = AtomicU32::new(1);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let bus_name = format!("org.kde.StatusNotifierItem-{}-{seq}", std::process::id());

        let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
            .map_err(|e| anyhow!("session D-Bus unavailable: {e}"))?;
        let sni_node = gio::DBusNodeInfo::for_xml(SNI_XML)?;
        let sni_iface = sni_node
            .lookup_interface("org.kde.StatusNotifierItem")
            .ok_or_else(|| anyhow!("missing SNI interface xml"))?;
        let menu_node = gio::DBusNodeInfo::for_xml(MENU_XML)?;
        let menu_iface = menu_node
            .lookup_interface("com.canonical.dbusmenu")
            .ok_or_else(|| anyhow!("missing dbusmenu interface xml"))?;

        let visible = Arc::new(AtomicBool::new(window_visible));
        let rev = Arc::new(AtomicU32::new(1));
        let icon_bytes = Arc::new(tray_icon::argb32_be());

        let sni_state = Arc::clone(&state);
        let sni_icon = Arc::clone(&icon_bytes);
        let sni_reg = conn.register_object(
            SNI_PATH,
            &sni_iface,
            move |_conn, _sender, _path, _iface, method, _params, inv| {
                match method {
                    "Activate" | "SecondaryActivate" => sni_state.toggle_window(),
                    "ContextMenu" | "Scroll" => {}
                    _ => {}
                }
                inv.return_value(None);
            },
            move |_conn, _sender, _path, _iface, prop| sni_property(prop, &sni_icon),
            |_conn, _sender, _path, _iface, _prop, _val| false,
        )?;

        let menu_state = Arc::clone(&state);
        let menu_vis = Arc::clone(&visible);
        let menu_rev = Arc::clone(&rev);
        let menu_reg = conn.register_object(
            MENU_PATH,
            &menu_iface,
            move |_conn, _sender, _path, _iface, method, params, inv| {
                handle_menu_call(&menu_state, &menu_vis, &menu_rev, method, &params, inv);
            },
            |_conn, _sender, _path, _iface, prop| menu_property(prop),
            |_conn, _sender, _path, _iface, _prop, _val| false,
        )?;

        // Register with StatusNotifierWatcher once BOTH our bus name is owned and the watcher is up,
        // and re-register automatically if Quickshell/Waybar restarts later.
        let name_owned = Arc::new(AtomicBool::new(false));
        let watcher_up = Arc::new(AtomicBool::new(false));
        let register = |c: &gio::DBusConnection, name: &str| {
            c.call(
                Some(WATCHER_NAME),
                WATCHER_PATH,
                WATCHER_NAME,
                "RegisterStatusNotifierItem",
                Some(&(name,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                gio::Cancellable::NONE,
                |_| {},
            );
        };

        let owned_flag = Arc::clone(&name_owned);
        let watcher_flag = Arc::clone(&watcher_up);
        let bus_for_owner = bus_name.clone();
        let owner_id = gio::bus_own_name_on_connection(
            &conn,
            &bus_name,
            gio::BusNameOwnerFlags::NONE,
            move |c, _| {
                owned_flag.store(true, Ordering::Release);
                if watcher_flag.load(Ordering::Acquire) {
                    register(&c, &bus_for_owner);
                }
            },
            move |_, _| {},
        );

        let owned_flag = Arc::clone(&name_owned);
        let watcher_flag = Arc::clone(&watcher_up);
        let watcher_flag_lost = Arc::clone(&watcher_up);
        let bus_for_watch = bus_name;
        let watcher_id = gio::bus_watch_name_on_connection(
            &conn,
            WATCHER_NAME,
            gio::BusNameWatcherFlags::NONE,
            move |c, _, _| {
                watcher_flag.store(true, Ordering::Release);
                if owned_flag.load(Ordering::Acquire) {
                    register(&c, &bus_for_watch);
                }
            },
            move |_, _| {
                watcher_flag_lost.store(false, Ordering::Release);
            },
        );

        Ok(Self {
            conn,
            sni_reg: Some(sni_reg),
            menu_reg: Some(menu_reg),
            owner_id: Some(owner_id),
            unwatch: Some(Box::new(move || gio::bus_unwatch_name(watcher_id))),
            visible,
            rev,
        })
    }

    fn set_window_visible(&self, is_visible: bool) {
        if self.visible.swap(is_visible, Ordering::AcqRel) == is_visible {
            return;
        }
        let rev = self.rev.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let _ = self.conn.emit_signal(
            None,
            MENU_PATH,
            "com.canonical.dbusmenu",
            "LayoutUpdated",
            Some(&(rev, 0i32).to_variant()),
        );
    }
}

impl Drop for ActiveTray {
    fn drop(&mut self) {
        if let Some(unwatch) = self.unwatch.take() {
            unwatch();
        }
        if let Some(id) = self.owner_id.take() {
            gio::bus_unown_name(id);
        }
        if let Some(id) = self.menu_reg.take() {
            let _ = self.conn.unregister_object(id);
        }
        if let Some(id) = self.sni_reg.take() {
            let _ = self.conn.unregister_object(id);
        }
    }
}

/// Manages the optional Linux system tray icon. Safe to toggle on/off at runtime from Settings.
pub struct Tray {
    state: Arc<State>,
    active: Option<ActiveTray>,
}

impl Tray {
    pub fn new(state: Arc<State>, enabled: bool, window_visible: bool) -> Self {
        let mut tray = Self { state, active: None };
        tray.set_enabled(enabled, window_visible);
        tray
    }

    pub fn set_enabled(&mut self, enabled: bool, window_visible: bool) {
        if !enabled {
            self.active = None;
            return;
        }
        if let Some(active) = &self.active {
            active.set_window_visible(window_visible);
            return;
        }
        match ActiveTray::start(Arc::clone(&self.state), window_visible) {
            Ok(active) => self.active = Some(active),
            Err(e) => eprintln!("system tray unavailable: {e:#}"),
        }
    }

    pub fn set_window_visible(&self, visible: bool) {
        if let Some(active) = &self.active {
            active.set_window_visible(visible);
        }
    }
}

fn empty_pixmaps() -> Variant {
    let ty = VariantTy::new("(iiay)").expect("valid variant type");
    Variant::array_from_iter_with_type(ty, std::iter::empty::<Variant>())
}

fn icon_pixmaps(argb: &[u8]) -> Variant {
    let size = tray_icon::SIZE as i32;
    let entry = Variant::tuple_from_iter([size.to_variant(), size.to_variant(), Variant::array_from_fixed_array(argb)]);
    let ty = VariantTy::new("(iiay)").expect("valid variant type");
    Variant::array_from_iter_with_type(ty, [entry])
}

fn sni_property(name: &str, icon_bytes: &[u8]) -> Variant {
    match name {
        "Category" => "ApplicationStatus".to_variant(),
        "Id" | "Title" => "warframeproject".to_variant(),
        "Status" => "Active".to_variant(),
        "WindowId" => 0u32.to_variant(),
        "IconPixmap" => icon_pixmaps(icon_bytes),
        "OverlayIconPixmap" | "AttentionIconPixmap" => empty_pixmaps(),
        "ToolTip" => Variant::tuple_from_iter([
            "".to_variant(),
            empty_pixmaps(),
            "warframeproject".to_variant(),
            "Click to show or hide".to_variant(),
        ]),
        "ItemIsMenu" => false.to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH).expect("valid object path").to_variant(),
        // Empty string for IconName, OverlayIconName, AttentionIconName, AttentionMovieName.
        _ => "".to_variant(),
    }
}

fn menu_property(name: &str) -> Variant {
    match name {
        "Version" => 3u32.to_variant(),
        "TextDirection" => "ltr".to_variant(),
        "Status" => "normal".to_variant(),
        "IconThemePath" => Variant::array_from_iter_with_type(VariantTy::STRING, std::iter::empty::<Variant>()),
        _ => "".to_variant(),
    }
}

fn item_props(id: i32, visible: bool) -> Variant {
    let dict = VariantDict::new(None);
    match id {
        0 => dict.insert("children-display", "submenu"),
        ID_TOGGLE => {
            dict.insert("label", if visible { "Hide window" } else { "Show window" });
            dict.insert("enabled", true);
            dict.insert("visible", true);
        }
        ID_SEP_1 | ID_SEP_2 => {
            dict.insert("type", "separator");
            dict.insert("visible", true);
        }
        ID_REWARDS => {
            dict.insert("label", "Scan reward screen");
            dict.insert("enabled", true);
            dict.insert("visible", true);
        }
        ID_RIVEN => {
            dict.insert("label", "Grade riven");
            dict.insert("enabled", true);
            dict.insert("visible", true);
        }
        ID_QUIT => {
            dict.insert("label", "Quit");
            dict.insert("enabled", true);
            dict.insert("visible", true);
        }
        _ => {}
    }
    dict.end()
}

fn menu_node(id: i32, visible: bool, depth: i32) -> Variant {
    let children: Vec<Variant> = if id == 0 && depth != 0 {
        let next_depth = if depth < 0 { -1 } else { depth - 1 };
        [ID_TOGGLE, ID_SEP_1, ID_REWARDS, ID_RIVEN, ID_SEP_2, ID_QUIT]
            .into_iter()
            .map(|cid| Variant::from_variant(&menu_node(cid, visible, next_depth)))
            .collect()
    } else {
        Vec::new()
    };
    let children_arr = Variant::array_from_iter_with_type(VariantTy::VARIANT, children);
    Variant::tuple_from_iter([id.to_variant(), item_props(id, visible), children_arr])
}

fn dispatch_menu_click(state: &Arc<State>, id: i32) {
    match id {
        ID_TOGGLE => state.toggle_window(),
        ID_REWARDS => {
            let s = Arc::clone(state);
            let _ = std::thread::Builder::new().name("tray-rewards".into()).spawn(move || {
                let _ = s.scan_rewards();
            });
        }
        ID_RIVEN => {
            let s = Arc::clone(state);
            let _ = std::thread::Builder::new().name("tray-riven".into()).spawn(move || {
                let _ = s.scan_riven();
            });
        }
        ID_QUIT => state.quit(),
        _ => {}
    }
}

fn handle_menu_call(
    state: &Arc<State>,
    visible: &AtomicBool,
    rev: &AtomicU32,
    method: &str,
    params: &Variant,
    inv: gio::DBusMethodInvocation,
) {
    let vis = visible.load(Ordering::Acquire);
    match method {
        "GetLayout" => {
            let parent_id = params.try_child_get::<i32>(0).ok().flatten().unwrap_or(0);
            let depth = params.try_child_get::<i32>(1).ok().flatten().unwrap_or(-1);
            let layout = menu_node(parent_id, vis, depth);
            let r = rev.load(Ordering::Acquire);
            inv.return_value(Some(&Variant::tuple_from_iter([r.to_variant(), layout])));
        }
        "GetGroupProperties" => {
            let ids = params
                .try_child_value(0)
                .and_then(|v| v.fixed_array::<i32>().ok().map(|s| s.to_vec()))
                .unwrap_or_default();
            let entries: Vec<Variant> =
                ids.into_iter().map(|id| Variant::tuple_from_iter([id.to_variant(), item_props(id, vis)])).collect();
            let ty = VariantTy::new("(ia{sv})").expect("valid variant type");
            let arr = Variant::array_from_iter_with_type(ty, entries);
            inv.return_value(Some(&Variant::tuple_from_iter([arr])));
        }
        "GetProperty" => {
            let id = params.try_child_get::<i32>(0).ok().flatten().unwrap_or(0);
            let name = params.try_child_get::<String>(1).ok().flatten().unwrap_or_default();
            let props = VariantDict::new(Some(&item_props(id, vis)));
            let val = props.lookup_value(&name, None).unwrap_or_else(|| "".to_variant());
            inv.return_value(Some(&Variant::tuple_from_iter([Variant::from_variant(&val)])));
        }
        "Event" => {
            let id = params.try_child_get::<i32>(0).ok().flatten().unwrap_or(0);
            let kind = params.try_child_get::<String>(1).ok().flatten().unwrap_or_default();
            if kind == "clicked" {
                dispatch_menu_click(state, id);
            }
            inv.return_value(None);
        }
        "EventGroup" => {
            if let Some(arr) = params.try_child_value(0) {
                for i in 0..arr.n_children() {
                    let ev = arr.child_value(i);
                    let id = ev.try_child_get::<i32>(0).ok().flatten().unwrap_or(0);
                    let kind = ev.try_child_get::<String>(1).ok().flatten().unwrap_or_default();
                    if kind == "clicked" {
                        dispatch_menu_click(state, id);
                    }
                }
            }
            let empty = Variant::array_from_fixed_array(&[] as &[i32]);
            inv.return_value(Some(&Variant::tuple_from_iter([empty])));
        }
        "AboutToShow" => {
            inv.return_value(Some(&Variant::tuple_from_iter([false.to_variant()])));
        }
        "AboutToShowGroup" => {
            let empty = Variant::array_from_fixed_array(&[] as &[i32]);
            inv.return_value(Some(&Variant::tuple_from_iter([empty.clone(), empty])));
        }
        _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "Unknown method"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbus_xml_and_menu_layout_variants_are_well_formed() {
        let sni = gio::DBusNodeInfo::for_xml(SNI_XML).unwrap();
        assert!(sni.lookup_interface("org.kde.StatusNotifierItem").is_some());
        let menu = gio::DBusNodeInfo::for_xml(MENU_XML).unwrap();
        assert!(menu.lookup_interface("com.canonical.dbusmenu").is_some());

        let icon = tray_icon::argb32_be();
        assert_eq!(sni_property("IconPixmap", &icon).type_().as_str(), "a(iiay)");
        assert_eq!(sni_property("ToolTip", &icon).type_().as_str(), "(sa(iiay)ss)");
        assert_eq!(sni_property("Menu", &icon).type_().as_str(), "o");
        assert_eq!(sni_property("ItemIsMenu", &icon).get::<bool>(), Some(false));

        let shown_root = menu_node(0, true, -1);
        assert_eq!(shown_root.type_().as_str(), "(ia{sv}av)");
        let children = shown_root.child_value(2);
        assert_eq!(children.n_children(), 6);
        let first = children.child_value(0).as_variant().unwrap();
        let props = VariantDict::new(Some(&first.child_value(1)));
        assert_eq!(props.lookup::<String>("label").unwrap().as_deref(), Some("Hide window"));

        let hidden_root = menu_node(0, false, -1);
        let first_hidden = hidden_root.child_value(2).child_value(0).as_variant().unwrap();
        let props_hidden = VariantDict::new(Some(&first_hidden.child_value(1)));
        assert_eq!(props_hidden.lookup::<String>("label").unwrap().as_deref(), Some("Show window"));
    }
}
