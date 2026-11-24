//! X11: one window, keys, the pointer, full screen and the theme property.
use crate::canvas::Canvas;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub struct Atoms {
    pub wm_protocols: Atom,
    pub wm_delete: Atom,
    pub net_wm_name: Atom,
    pub net_wm_pid: Atom,
    pub net_wm_state: Atom,
    pub net_wm_state_fullscreen: Atom,
    pub utf8: Atom,
    pub niruwm_theme: Atom,
}

pub struct X {
    pub c: RustConnection,
    pub root: Window,
    pub depth: u8,
    gc: Gcontext,
    pub atoms: Atoms,
    pub win: Window,
    pub fullscreen: bool,
    min_keycode: u8,
    per: u8,
    keysyms: Vec<u32>,
}

fn atom(c: &RustConnection, name: &str) -> Res<Atom> {
    Ok(c.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

impl X {
    pub fn open(title: &str) -> Res<Self> {
        let (c, screen) = x11rb::connect(None)?;
        let s = &c.setup().roots[screen];
        let (root, depth, visual) = (s.root, s.root_depth, s.root_visual);
        let (sw, sh) = (s.width_in_pixels as u32, s.height_in_pixels as u32);
        let atoms = Atoms {
            wm_protocols: atom(&c, "WM_PROTOCOLS")?,
            wm_delete: atom(&c, "WM_DELETE_WINDOW")?,
            net_wm_name: atom(&c, "_NET_WM_NAME")?,
            net_wm_pid: atom(&c, "_NET_WM_PID")?,
            net_wm_state: atom(&c, "_NET_WM_STATE")?,
            net_wm_state_fullscreen: atom(&c, "_NET_WM_STATE_FULLSCREEN")?,
            utf8: atom(&c, "UTF8_STRING")?,
            niruwm_theme: atom(&c, "_NIRUWM_THEME")?,
        };
        let events = EventMask::EXPOSURE
            | EventMask::KEY_PRESS
            | EventMask::BUTTON_PRESS
            | EventMask::POINTER_MOTION
            | EventMask::LEAVE_WINDOW
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::VISIBILITY_CHANGE;
        let win = c.generate_id()?;
        let (w, h) = ((sw * 3 / 4).clamp(480, 1600), (sh * 3 / 4).clamp(360, 1000));
        c.create_window(
            depth,
            win,
            root,
            0,
            0,
            w.min(sw) as u16,
            h.min(sh) as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(s.black_pixel)
                .bit_gravity(Gravity::NORTH_WEST)
                .event_mask(events),
        )?;
        c.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        c.change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"nirutoday\0NIRUTODAY\0",
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.net_wm_pid,
            AtomEnum::CARDINAL,
            &[std::process::id()],
        )?;
        c.change_property32(
            PropMode::REPLACE,
            win,
            atoms.wm_protocols,
            AtomEnum::ATOM,
            &[atoms.wm_delete],
        )?;
        let gc = c.generate_id()?;
        c.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))?;
        let setup = c.setup();
        let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);
        let map = c
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;
        let x = X {
            c,
            root,
            depth,
            gc,
            atoms,
            win,
            fullscreen: false,
            min_keycode,
            per: map.keysyms_per_keycode,
            keysyms: map.keysyms,
        };
        x.set_title(title);
        x.c.map_window(win)?;
        x.c.flush()?;
        Ok(x)
    }

    pub fn set_title(&self, t: &str) {
        for (prop, kind) in [
            (self.atoms.net_wm_name, self.atoms.utf8),
            (AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()),
        ] {
            let _ = self
                .c
                .change_property8(PropMode::REPLACE, self.win, prop, kind, t.as_bytes());
        }
        self.flush();
    }

    pub fn fd(&self) -> i32 {
        use std::os::fd::AsRawFd;
        self.c.stream().as_raw_fd()
    }

    pub fn poll(&self) -> Option<Event> {
        self.c.poll_for_event().ok().flatten()
    }

    pub fn flush(&self) {
        let _ = self.c.flush();
    }

    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        let ev = ClientMessageEvent::new(
            32,
            self.win,
            self.atoms.net_wm_state,
            [
                u32::from(self.fullscreen),
                self.atoms.net_wm_state_fullscreen,
                0,
                1,
                0,
            ],
        );
        let _ = self.c.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        );
        self.flush();
    }

    /// Copy rows `y0..y1` of the canvas to the window, in slices that fit
    /// the request size.
    pub fn present_rows(&self, cv: &Canvas, y0: i32, y1: i32) {
        let bytes = cv.as_bytes();
        let max = self.c.maximum_request_bytes().saturating_sub(64).max(4096);
        let row_bytes = cv.w as usize * 4;
        let rows = (max / row_bytes.max(1)).max(1);
        let (mut y, end) = (y0.max(0) as usize, (y1.min(cv.h)).max(0) as usize);
        while y < end {
            let n = rows.min(end - y);
            let _ = self.c.put_image(
                ImageFormat::Z_PIXMAP,
                self.win,
                self.gc,
                cv.w as u16,
                n as u16,
                0,
                y as i16,
                0,
                self.depth,
                &bytes[y * row_bytes..(y + n) * row_bytes],
            );
            y += n;
        }
        self.flush();
    }

    pub fn present(&self, cv: &Canvas) {
        self.present_rows(cv, 0, cv.h);
    }

    pub fn root_theme(&self) -> Option<String> {
        let r = self
            .c
            .get_property(
                false,
                self.root,
                self.atoms.niruwm_theme,
                AtomEnum::ANY,
                0,
                64,
            )
            .ok()?
            .reply()
            .ok()?;
        let s = String::from_utf8(r.value).ok()?;
        let s = s.trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn keysym(&self, keycode: u8, state: KeyButMask) -> u32 {
        let i = (keycode.saturating_sub(self.min_keycode)) as usize * self.per as usize;
        let shift = state.contains(KeyButMask::SHIFT);
        let get = |k: usize| self.keysyms.get(i + k).copied().unwrap_or(0);
        let (low, high) = (get(0), get(1));
        if shift && high != 0 {
            high
        } else if shift && (0x61..=0x7a).contains(&low) {
            low - 0x20
        } else {
            low
        }
    }
}
