// Windows bits, as in the web app (src-tauri/src/main.rs): the media keys, the tray icon and its menu,
// and the buttons on the taskbar thumbnail (shuffle, previous, play/pause, next, like). Each hands
// an action ("play", "next", "prev", "shuffle", "like", "show", "quit") to the app through poll().

use global_hotkey::hotkey::{Code, HotKey};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::Mutex;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// What the taskbar buttons clicked, waiting for the app
static CLICKED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub struct Desktop {
    _keys: Option<GlobalHotKeyManager>,
    keys: Vec<(u32, &'static str)>,
    _tray: Option<TrayIcon>,
    menu: Vec<(tray_icon::menu::MenuId, &'static str)>,
}

impl Desktop {
    /// Sets them up; whatever can't be (a media key another app holds, say) is left out quietly
    pub fn start() -> Desktop {
        let mut keys = Vec::new();
        let manager = GlobalHotKeyManager::new().ok().inspect(|m| {
            for (code, action) in [(Code::MediaPlayPause, "play"), (Code::MediaTrackNext, "next"), (Code::MediaTrackPrevious, "prev")] {
                let hk = HotKey::new(None, code);
                if m.register(hk).is_ok() {
                    keys.push((hk.id(), action));
                }
            }
        });
        let mut menu_ids = Vec::new();
        let tray = (|| {
            let menu = Menu::new();
            let items = [("Show", "show"), ("Play / Pause", "play"), ("Next", "next"), ("Previous", "prev"), ("Quit", "quit")];
            for (label, action) in items {
                let it = MenuItem::new(label, true, None);
                menu_ids.push((it.id().clone(), action));
                menu.append(&it).ok()?;
            }
            let png = image::load_from_memory(include_bytes!("../../../src-tauri/icons/32x32.png")).ok()?.to_rgba8();
            let icon = tray_icon::Icon::from_rgba(png.as_raw().clone(), png.width(), png.height()).ok()?;
            TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("OmniPulse").with_icon(icon).build().ok()
        })();
        Desktop { _keys: manager, keys, _tray: tray, menu: menu_ids }
    }

    /// What's been pressed since last time
    pub fn poll(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        while let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
            if e.state == HotKeyState::Pressed {
                if let Some((_, a)) = self.keys.iter().find(|(id, _)| *id == e.id) {
                    out.push(*a);
                }
            }
        }
        while let Ok(e) = MenuEvent::receiver().try_recv() {
            if let Some((_, a)) = self.menu.iter().find(|(id, _)| *id == e.id) {
                out.push(*a);
            }
        }
        while let Ok(e) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                out.push("show");
            }
        }
        if let Ok(mut c) = CLICKED.lock() {
            out.append(&mut c);
        }
        out
    }
}

// ---------- the taskbar thumbnail's buttons ----------
#[cfg(windows)]
pub mod taskbar {
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_DISABLED, THBF_ENABLED, THBN_CLICKED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{LoadImageW, PostMessageW, RegisterWindowMessageW, HICON, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, WM_COMMAND, WM_USER};

    const SUBCLASS_ID: usize = 7312;
    const BTN: [(u32, &str); 5] = [(2001, "shuffle"), (2002, "prev"), (2003, "play"), (2004, "next"), (2005, "like")];
    const WM_UPDATE_STATE: u32 = WM_USER + 21;
    static CREATED_MSG: AtomicU32 = AtomicU32::new(0);
    static HWND_SEEN: AtomicIsize = AtomicIsize::new(0);

    // the icons, the same as the web app's (written to the data folder once, for Windows to load)
    const ICONS: [(&str, &[u8]); 8] = [
        ("SHUFFLE_OFF", include_bytes!("../../../src-tauri/icons/taskbar/SHUFFLE_OFF.ico")),
        ("SHUFFLE_ON", include_bytes!("../../../src-tauri/icons/taskbar/SHUFFLE_ON.ico")),
        ("PREV_THUMB", include_bytes!("../../../src-tauri/icons/taskbar/PREV_THUMB.ico")),
        ("PLAY_THUMB", include_bytes!("../../../src-tauri/icons/taskbar/PLAY_THUMB.ico")),
        ("PAUSE_THUMB", include_bytes!("../../../src-tauri/icons/taskbar/PAUSE_THUMB.ico")),
        ("NEXT_THUMB", include_bytes!("../../../src-tauri/icons/taskbar/NEXT_THUMB.ico")),
        ("LIKE_OFF", include_bytes!("../../../src-tauri/icons/taskbar/LIKE_OFF.ico")),
        ("LIKE_ON", include_bytes!("../../../src-tauri/icons/taskbar/LIKE_ON.ico")),
    ];

    struct Ctx {
        taskbar: Option<ITaskbarList3>,
        icons: Vec<HICON>,
        state: usize,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn load_icons() -> Option<Vec<HICON>> {
        let dir = omnipulse_core::data_dir().ok()?.join("state").join("taskbar");
        std::fs::create_dir_all(&dir).ok()?;
        ICONS
            .iter()
            .map(|(name, bytes)| {
                let path = dir.join(format!("{name}.ico"));
                if std::fs::metadata(&path).map(|m| m.len() as usize) .ok() != Some(bytes.len()) {
                    std::fs::write(&path, bytes).ok()?;
                }
                let w = wide(&path.to_string_lossy());
                unsafe { LoadImageW(None, PCWSTR(w.as_ptr()), IMAGE_ICON, 0, 0, LR_LOADFROMFILE | LR_DEFAULTSIZE).ok().map(|h| HICON(h.0)) }
            })
            .collect()
    }

    fn button(id: u32, icon: HICON, tip: &str, enabled: bool) -> THUMBBUTTON {
        let mut b = THUMBBUTTON { dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS, iId: id, hIcon: icon, szTip: [0; 260], dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED }, ..Default::default() };
        for (i, c) in tip.encode_utf16().take(259).enumerate() {
            b.szTip[i] = c;
        }
        b
    }

    // state bits: 1 playing, 2 shuffle, 4 liked, 8 can like
    fn buttons(i: &[HICON], bits: usize) -> [THUMBBUTTON; 5] {
        let (playing, shuffle, liked, can_like) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
        [
            button(2001, if shuffle { i[1] } else { i[0] }, if shuffle { "Shuffle: on" } else { "Shuffle" }, true),
            button(2002, i[2], "Previous", true),
            button(2003, if playing { i[4] } else { i[3] }, if playing { "Pause" } else { "Play" }, true),
            button(2004, i[5], "Next", true),
            button(2005, if liked { i[7] } else { i[6] }, if liked { "Remove from Likes" } else { "Like" }, can_like),
        ]
    }

    unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, data: usize) -> LRESULT {
        let ctx = &mut *(data as *mut Ctx);
        let created = CREATED_MSG.load(Ordering::Relaxed);
        // the taskbar button exists (or Explorer restarted): add ours
        if created != 0 && msg == created {
            if let Ok(tb) = CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
                if tb.HrInit().is_ok() {
                    let _ = tb.ThumbBarAddButtons(hwnd, &buttons(&ctx.icons, ctx.state));
                    ctx.taskbar = Some(tb);
                }
            }
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
        if msg == WM_UPDATE_STATE {
            ctx.state = wparam.0;
            if let Some(tb) = &ctx.taskbar {
                let _ = tb.ThumbBarUpdateButtons(hwnd, &buttons(&ctx.icons, ctx.state));
            }
            return LRESULT(0);
        }
        if msg == WM_COMMAND && ((wparam.0 >> 16) & 0xFFFF) as u32 == THBN_CLICKED {
            let id = (wparam.0 & 0xFFFF) as u32;
            if let Some((_, a)) = BTN.iter().find(|(b, _)| *b == id) {
                if let Ok(mut c) = super::CLICKED.lock() {
                    c.push(a);
                }
            }
            return LRESULT(0);
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    /// Adds the buttons to this window's taskbar thumbnail
    pub fn attach(hwnd: isize) {
        let Some(icons) = load_icons() else { return };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if CREATED_MSG.load(Ordering::Relaxed) == 0 {
                let name = wide("TaskbarButtonCreated");
                CREATED_MSG.store(RegisterWindowMessageW(PCWSTR(name.as_ptr())), Ordering::Relaxed);
            }
            let ctx = Box::into_raw(Box::new(Ctx { taskbar: None, icons, state: 0 }));
            let h = HWND(hwnd as _);
            let _ = SetWindowSubclass(h, Some(subclass_proc), SUBCLASS_ID, ctx as usize);
            HWND_SEEN.store(hwnd, Ordering::Relaxed);
            // in case the taskbar button was made before we were listening
            let _ = PostMessageW(Some(h), CREATED_MSG.load(Ordering::Relaxed), WPARAM(0), LPARAM(0));
        }
    }

    /// Keeps the buttons in step with the player
    pub fn state(playing: bool, shuffle: bool, liked: bool, can_like: bool) {
        let h = HWND_SEEN.load(Ordering::Relaxed);
        if h == 0 {
            return;
        }
        let bits = playing as usize | (shuffle as usize) << 1 | (liked as usize) << 2 | (can_like as usize) << 3;
        unsafe {
            let _ = PostMessageW(Some(HWND(h as _)), WM_UPDATE_STATE, WPARAM(bits), LPARAM(0));
        }
    }
}

#[cfg(not(windows))]
pub mod taskbar {
    pub fn attach(_hwnd: isize) {}
    pub fn state(_playing: bool, _shuffle: bool, _liked: bool, _can_like: bool) {}
}
