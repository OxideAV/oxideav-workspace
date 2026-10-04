//! SDL2 viewer window (runtime-loaded libSDL2). Software frames only:
//! each [`RgbaImage`] is streamed into an RGBA32 texture and stretched
//! over the window (frames are rendered at the window's aspect, so the
//! stretch is uniform). SDL has no text overlay here, so the HUD's
//! one-line summary goes to the window title.

use std::ffi::{c_int, c_void, CString};
use std::ptr;
use std::sync::Arc;
use std::time::Duration;

use oxideav_core::{Error, Result};
use oxideav_render::RgbaImage;

use crate::drivers::sdl2_loader::{self as ldr, SDL_Event, Sdl2Lib};
use crate::drivers::sdl2_root::{self, SubsystemGuard};

use super::state::{InputEvent, Key, MouseButton};
use super::{Frontend, Hud};

const INITIAL_SIZE: (u32, u32) = (1024, 768);

pub struct SdlFrontend {
    lib: Arc<Sdl2Lib>,
    window: *mut c_void,
    renderer: *mut c_void,
    texture: Option<(*mut c_void, u32, u32)>,
    last_title: String,
    // Dropped last so SDL_QuitSubSystem runs after the window is gone.
    _guard: SubsystemGuard,
}

impl SdlFrontend {
    pub fn new(title: &str) -> Result<Self> {
        let guard = sdl2_root::acquire(sdl2_root::VIDEO_MASK)?;
        let lib = guard.lib().clone();
        let ctitle = CString::new(title.replace('\0', "")).unwrap_or_default();
        // SAFETY: title is NUL-terminated and outlives the call.
        let window = unsafe {
            (lib.SDL_CreateWindow)(
                ctitle.as_ptr(),
                ldr::SDL_WINDOWPOS_CENTERED,
                ldr::SDL_WINDOWPOS_CENTERED,
                INITIAL_SIZE.0 as c_int,
                INITIAL_SIZE.1 as c_int,
                ldr::SDL_WINDOW_RESIZABLE,
            )
        };
        if window.is_null() {
            return Err(Error::other(format!(
                "SDL_CreateWindow failed: {}",
                lib.last_error()
            )));
        }
        // SAFETY: window is valid; -1 = first capable driver.
        let renderer = unsafe { (lib.SDL_CreateRenderer)(window, -1, 0) };
        if renderer.is_null() {
            let err = lib.last_error();
            unsafe { (lib.SDL_DestroyWindow)(window) };
            return Err(Error::other(format!("SDL_CreateRenderer failed: {err}")));
        }
        Ok(Self {
            lib,
            window,
            renderer,
            texture: None,
            last_title: title.to_string(),
            _guard: guard,
        })
    }

    fn translate(&mut self, ev: &SDL_Event, out: &mut Vec<InputEvent>) {
        match ev.r#type {
            ldr::SDL_QUIT => out.push(InputEvent::CloseRequested),
            ldr::SDL_WINDOWEVENT => {
                // SDL_WindowEvent: event (u8) @12, data1 @16, data2 @20.
                if ev.u8_at(12) == ldr::SDL_WINDOWEVENT_SIZE_CHANGED {
                    let (w, h) = self.output_size();
                    out.push(InputEvent::Resized(w, h));
                }
            }
            ldr::SDL_KEYDOWN | ldr::SDL_KEYUP => {
                // SAFETY: type checked above.
                let key = unsafe { ev.as_key() };
                let sym = key.keysym.sym;
                let down = ev.r#type == ldr::SDL_KEYDOWN;
                if sym == ldr::SDLK_LSHIFT || sym == ldr::SDLK_RSHIFT {
                    out.push(InputEvent::Shift(down));
                    return;
                }
                if !down {
                    return;
                }
                let k = match sym {
                    s if s == ldr::SDLK_ESCAPE => Some(Key::Escape),
                    s if s == ldr::SDLK_HOME => Some(Key::Home),
                    s if s == ldr::SDLK_F1 => Some(Key::F1),
                    s if s == ldr::SDLK_KP_PLUS => Some(Key::Char('+')),
                    s if s == ldr::SDLK_KP_MINUS => Some(Key::Char('-')),
                    s if (0x20..0x7f).contains(&s) => {
                        Some(Key::Char((s as u8 as char).to_ascii_lowercase()))
                    }
                    _ => None,
                };
                if let Some(k) = k {
                    let zoom = matches!(k, Key::Char('+' | '=' | '-' | '_'));
                    if key.repeat == 0 || zoom {
                        out.push(InputEvent::Key(k));
                    }
                }
            }
            ldr::SDL_MOUSEMOTION => {
                // SDL_MouseMotionEvent: x @20, y @24.
                out.push(InputEvent::MouseMove {
                    x: ev.i32_at(20) as f64,
                    y: ev.i32_at(24) as f64,
                });
            }
            ldr::SDL_MOUSEBUTTONDOWN | ldr::SDL_MOUSEBUTTONUP => {
                // SDL_MouseButtonEvent: button (u8) @16, x @20, y @24.
                let button = match ev.u8_at(16) {
                    ldr::SDL_BUTTON_LEFT => MouseButton::Left,
                    ldr::SDL_BUTTON_MIDDLE => MouseButton::Middle,
                    ldr::SDL_BUTTON_RIGHT => MouseButton::Right,
                    _ => return,
                };
                out.push(if ev.r#type == ldr::SDL_MOUSEBUTTONDOWN {
                    InputEvent::MouseDown {
                        button,
                        x: ev.i32_at(20) as f64,
                        y: ev.i32_at(24) as f64,
                    }
                } else {
                    InputEvent::MouseUp { button }
                });
            }
            ldr::SDL_MOUSEWHEEL => {
                // SDL_MouseWheelEvent: y @20, direction @24.
                let mut y = ev.i32_at(20) as f32;
                if ev.u32_at(24) == ldr::SDL_MOUSEWHEEL_FLIPPED {
                    y = -y;
                }
                out.push(InputEvent::Wheel(y));
            }
            _ => {}
        }
    }

    fn output_size(&self) -> (u32, u32) {
        let (mut w, mut h): (c_int, c_int) = (0, 0);
        // SAFETY: renderer is valid for self's lifetime.
        unsafe { (self.lib.SDL_GetRendererOutputSize)(self.renderer, &mut w, &mut h) };
        if w <= 0 || h <= 0 {
            INITIAL_SIZE
        } else {
            (w as u32, h as u32)
        }
    }
}

impl Drop for SdlFrontend {
    fn drop(&mut self) {
        // SAFETY: each handle is destroyed once, children first.
        unsafe {
            if let Some((t, _, _)) = self.texture.take() {
                (self.lib.SDL_DestroyTexture)(t);
            }
            (self.lib.SDL_DestroyRenderer)(self.renderer);
            (self.lib.SDL_DestroyWindow)(self.window);
        }
    }
}

impl Frontend for SdlFrontend {
    fn pump(&mut self, timeout: Duration) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let mut ev = SDL_Event::zeroed();
        // Block for the first event (up to `timeout`), then drain.
        let ms = timeout.as_millis().min(i32::MAX as u128) as c_int;
        // SAFETY: ev is a properly sized + aligned SDL_Event.
        let mut got = unsafe { (self.lib.SDL_WaitEventTimeout)(&mut ev, ms) };
        while got != 0 {
            self.translate(&ev, &mut out);
            ev = SDL_Event::zeroed();
            got = unsafe { (self.lib.SDL_PollEvent)(&mut ev) };
        }
        out
    }

    fn size(&self) -> (u32, u32) {
        self.output_size()
    }

    fn set_image(&mut self, img: &RgbaImage) -> Result<()> {
        let (w, h) = (img.width, img.height);
        if w == 0 || h == 0 {
            return Ok(());
        }
        let stride = img.stride.max(w as usize * 4);
        if img.pixels.len() < stride * (h as usize - 1) + w as usize * 4 {
            return Err(Error::invalid("viewer: short RGBA image"));
        }
        if !matches!(self.texture, Some((_, tw, th)) if (tw, th) == (w, h)) {
            if let Some((t, _, _)) = self.texture.take() {
                unsafe { (self.lib.SDL_DestroyTexture)(t) };
            }
            // SAFETY: renderer is valid.
            let t = unsafe {
                (self.lib.SDL_CreateTexture)(
                    self.renderer,
                    ldr::SDL_PIXELFORMAT_RGBA32,
                    ldr::SDL_TEXTUREACCESS_STREAMING,
                    w as c_int,
                    h as c_int,
                )
            };
            if t.is_null() {
                return Err(Error::other(format!(
                    "SDL_CreateTexture failed: {}",
                    self.lib.last_error()
                )));
            }
            self.texture = Some((t, w, h));
        }
        if let Some((t, _, _)) = self.texture {
            // SAFETY: pixels cover h rows of `stride` bytes (checked).
            let rc = unsafe {
                (self.lib.SDL_UpdateTexture)(
                    t,
                    ptr::null(),
                    img.pixels.as_ptr() as *const c_void,
                    stride as c_int,
                )
            };
            if rc != 0 {
                return Err(Error::other(format!(
                    "SDL_UpdateTexture failed: {}",
                    self.lib.last_error()
                )));
            }
        }
        Ok(())
    }

    fn present(&mut self, hud: &Hud) -> Result<()> {
        // SAFETY: all handles valid; null rects = whole texture/target.
        unsafe {
            (self.lib.SDL_RenderClear)(self.renderer);
            if let Some((t, _, _)) = self.texture {
                (self.lib.SDL_RenderCopy)(self.renderer, t, ptr::null(), ptr::null());
            }
            (self.lib.SDL_RenderPresent)(self.renderer);
        }
        // No text overlay: the HUD goes to the window title (summary
        // only, or every status row when the HUD is toggled on).
        let title = if hud.visible {
            let rows: Vec<String> = hud
                .rows
                .iter()
                .filter(|(k, _)| k != "file")
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            format!("{} | {}", hud.summary, rows.join(" · "))
        } else {
            hud.summary.clone()
        };
        if title != self.last_title {
            if let Ok(c) = CString::new(title.replace('\0', "")) {
                // SAFETY: window valid, title NUL-terminated.
                unsafe { (self.lib.SDL_SetWindowTitle)(self.window, c.as_ptr()) };
            }
            self.last_title = title;
        }
        Ok(())
    }

    fn toggle_fullscreen(&mut self) {
        // SAFETY: window valid.
        unsafe {
            let flags = (self.lib.SDL_GetWindowFlags)(self.window);
            let next = if flags & ldr::SDL_WINDOW_FULLSCREEN_DESKTOP != 0 {
                0
            } else {
                ldr::SDL_WINDOW_FULLSCREEN_DESKTOP
            };
            (self.lib.SDL_SetWindowFullscreen)(self.window, next);
        }
    }

    fn describe(&self) -> String {
        let (w, h) = self.output_size();
        format!("sdl2  window {w}x{h}  (software render, RGBA32 upload)")
    }
}
