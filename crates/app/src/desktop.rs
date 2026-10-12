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

// ---------- Windows' media controls (by the volume, in quick settings, on the lock screen) ----------
/// What's playing, as Windows' media controls and the taskbar widget show it
#[derive(Clone, Default, PartialEq)]
pub struct Now {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: String,
    pub playing: bool,
    pub has_track: bool,
    pub pos_ms: u64,
    pub dur_ms: u64,
}

/// A place in the track asked for from Windows' controls (ms), waiting for the app
static SEEK: Mutex<Option<u64>> = Mutex::new(None);

pub fn take_seek() -> Option<u64> {
    SEEK.lock().ok()?.take()
}

#[cfg(windows)]
pub mod media {
    use super::Now;
    use std::cell::RefCell;
    use windows::core::HSTRING;
    use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs, SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::RandomAccessStreamReference;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

    struct Smtc {
        c: SystemMediaTransportControls,
        shown: Option<Now>,
    }

    thread_local! {
        static SMTC: RefCell<Option<Smtc>> = const { RefCell::new(None) };
    }

    fn ts(ms: u64) -> TimeSpan {
        TimeSpan { Duration: ms as i64 * 10_000 }
    }

    fn make(hwnd: isize) -> windows::core::Result<SystemMediaTransportControls> {
        let interop = windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        let c: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd as _))? };
        c.SetIsPlayEnabled(true)?;
        c.SetIsPauseEnabled(true)?;
        c.SetIsNextEnabled(true)?;
        c.SetIsPreviousEnabled(true)?;
        c.ButtonPressed(&TypedEventHandler::<SystemMediaTransportControls, SystemMediaTransportControlsButtonPressedEventArgs>::new(|_, args| {
            if let Some(a) = args.as_ref() {
                let b = a.Button()?;
                let act = if b == SystemMediaTransportControlsButton::Play || b == SystemMediaTransportControlsButton::Pause {
                    "play"
                } else if b == SystemMediaTransportControlsButton::Next {
                    "next"
                } else if b == SystemMediaTransportControlsButton::Previous {
                    "prev"
                } else {
                    return Ok(());
                };
                if let Ok(mut c) = super::CLICKED.lock() {
                    c.push(act);
                }
            }
            Ok(())
        }))?;
        c.PlaybackPositionChangeRequested(&TypedEventHandler::<SystemMediaTransportControls, PlaybackPositionChangeRequestedEventArgs>::new(|_, args| {
            if let Some(a) = args.as_ref() {
                let ms = (a.RequestedPlaybackPosition()?.Duration / 10_000).max(0) as u64;
                if let Ok(mut s) = super::SEEK.lock() {
                    *s = Some(ms);
                }
            }
            Ok(())
        }))?;
        Ok(c)
    }

    /// On or off (Settings > App > Windows Media Controls); `hwnd` the app's window
    pub fn enable(hwnd: isize, on: bool) {
        SMTC.with(|s| {
            let mut s = s.borrow_mut();
            if on && s.is_none() {
                if let Ok(c) = make(hwnd) {
                    let _ = c.SetIsEnabled(true);
                    *s = Some(Smtc { c, shown: None });
                }
            }
            if !on {
                if let Some(m) = s.take() {
                    let _ = m.c.DisplayUpdater().and_then(|d| d.ClearAll());
                    let _ = m.c.SetIsEnabled(false);
                }
            }
        });
    }

    /// Keeps them in step with the player (only what's changed is sent)
    pub fn update(now: &Now) {
        SMTC.with(|s| {
            let mut s = s.borrow_mut();
            let Some(m) = s.as_mut() else { return };
            if m.shown.as_ref() == Some(now) {
                return;
            }
            let was = m.shown.take().unwrap_or_default();
            let first = !was.has_track;
            if !now.has_track {
                let _ = m.c.SetPlaybackStatus(MediaPlaybackStatus::Closed);
                let _ = m.c.DisplayUpdater().and_then(|d| d.ClearAll());
                m.shown = Some(now.clone());
                return;
            }
            if first || was.title != now.title || was.artist != now.artist || was.art != now.art || was.album != now.album {
                let _ = (|| -> windows::core::Result<()> {
                    let d = m.c.DisplayUpdater()?;
                    d.SetType(MediaPlaybackType::Music)?;
                    let p = d.MusicProperties()?;
                    p.SetTitle(&HSTRING::from(now.title.as_str()))?;
                    p.SetArtist(&HSTRING::from(now.artist.as_str()))?;
                    p.SetAlbumTitle(&HSTRING::from(now.album.as_str()))?;
                    if now.art.starts_with("http") {
                        d.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&HSTRING::from(now.art.as_str()))?)?)?;
                    } else {
                        d.SetThumbnail(None)?;
                    }
                    d.Update()
                })();
            }
            if first || was.playing != now.playing {
                let _ = m.c.SetPlaybackStatus(if now.playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused });
            }
            // where it is: each second, when it jumps (a seek), or with a new length
            if first || now.pos_ms / 1000 != was.pos_ms / 1000 || now.dur_ms != was.dur_ms {
                let _ = (|| -> windows::core::Result<()> {
                    let t = SystemMediaTransportControlsTimelineProperties::new()?;
                    t.SetStartTime(ts(0))?;
                    t.SetMinSeekTime(ts(0))?;
                    t.SetEndTime(ts(now.dur_ms))?;
                    t.SetMaxSeekTime(ts(now.dur_ms))?;
                    t.SetPosition(ts(now.pos_ms.min(now.dur_ms)))?;
                    m.c.UpdateTimelineProperties(&t)
                })();
            }
            m.shown = Some(now.clone());
        });
    }
}

#[cfg(not(windows))]
pub mod media {
    pub fn enable(_hwnd: isize, _on: bool) {}
    pub fn update(_now: &super::Now) {}
}

// ---------- the taskbar widget's place: over the taskbar's empty room, left of the clock ----------
#[cfg(windows)]
pub mod dock {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL, DWM_WINDOW_CORNER_PREFERENCE};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, FindWindowW, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    /// Where it goes, in pixels (x, y, width, height): the taskbar's height less a little, `width` wide,
    /// just left of the clock and the tray. None: no room (the taskbar hidden, or down a side of the screen)
    pub fn spot(width: i32, scale: f32) -> Option<(i32, i32, i32, i32)> {
        unsafe {
            let bar = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
            let mut r = RECT::default();
            GetWindowRect(bar, &mut r).ok()?;
            let (bw, bh) = (r.right - r.left, r.bottom - r.top);
            if bw < bh || bh < 20 {
                return None;
            }
            // (auto-hidden: only a sliver of it on screen)
            let mon = MonitorFromWindow(bar, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                let m = mi.rcMonitor;
                if r.top >= m.bottom - 4 || r.bottom <= m.top + 4 {
                    return None;
                }
            }
            let mut t = RECT::default();
            let right = match FindWindowExW(Some(bar), None, w!("TrayNotifyWnd"), PCWSTR::null()) {
                Ok(tray) if GetWindowRect(tray, &mut t).is_ok() && t.left > r.left + bw / 2 => t.left,
                _ => r.right - (220.0 * scale) as i32,
            };
            let gap = (6.0 * scale) as i32;
            Some((right - width - gap, r.top + gap, width, bh - 2 * gap))
        }
    }

    /// Something full screen (a game, a video, a presentation): it keeps out of the way
    pub fn busy() -> bool {
        unsafe { SHQueryUserNotificationState().map(|s| s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE).unwrap_or(false) }
    }

    /// Made a widget: no taskbar button of its own, never takes the focus, small round corners
    pub fn style(hwnd: isize) {
        unsafe {
            let h = HWND(hwnd as _);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
            SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW.0 as isize | WS_EX_NOACTIVATE.0 as isize) & !(WS_EX_APPWINDOW.0 as isize));
            let pref = DWMWCP_ROUNDSMALL;
            let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const DWM_WINDOW_CORNER_PREFERENCE as _, std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32);
        }
    }

    /// Put (back) over the taskbar: it comes up over us whenever it's clicked
    pub fn keep(hwnd: isize, (x, y, w, h): (i32, i32, i32, i32)) {
        unsafe {
            let _ = SetWindowPos(HWND(hwnd as _), Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }
}

#[cfg(not(windows))]
pub mod dock {
    pub fn spot(_w: i32, _s: f32) -> Option<(i32, i32, i32, i32)> {
        None
    }
    pub fn busy() -> bool {
        false
    }
    pub fn style(_hwnd: isize) {}
    pub fn keep(_hwnd: isize, _r: (i32, i32, i32, i32)) {}
}
