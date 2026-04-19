//! Shared libSDL2 loader + subsystem reference counting.
//!
//! `--vo sdl2` and `--ao sdl2` both want an initialised libSDL2 but
//! they're independent engines — either can be used alone, together,
//! or paired with a non-SDL counterpart. `Sdl2Root::acquire` loads the
//! library once per process (via `OnceLock`) and ref-counts subsystem
//! init so each engine brings up exactly what it needs and the last
//! holder tears everything down cleanly.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use oxideav_core::{Error, Result};

use crate::drivers::sdl2_loader::{self as ldr, Sdl2Lib};

/// Global process-level libSDL2 handle. Loaded lazily on first engine
/// creation, never torn down — SDL has no graceful "unload" and the
/// library keeps a bunch of thread-locals alive that we'd leak
/// anyway.
static LIB: OnceLock<Arc<Sdl2Lib>> = OnceLock::new();
/// Total subsystem-init holders. When the last engine drops, the
/// final `SDL_QuitSubSystem` happens and we also call `SDL_Quit` to
/// release SDL's own globals.
static SUBSYSTEM_HOLDERS: AtomicUsize = AtomicUsize::new(0);

fn lib() -> Result<Arc<Sdl2Lib>> {
    if let Some(l) = LIB.get() {
        return Ok(l.clone());
    }
    let loaded = Sdl2Lib::try_load().map_err(|e| {
        Error::other(format!(
            "SDL2 library not found at runtime — install libSDL2 ({e})"
        ))
    })?;
    let arc = Arc::new(loaded);
    // Race: two threads calling `lib()` simultaneously both load the
    // library and then try to publish it. `get_or_init` closes the race
    // by keeping only one.
    let winner = LIB.get_or_init(|| arc.clone());
    Ok(winner.clone())
}

/// RAII guard over `SDL_InitSubSystem(mask)`. Drop calls
/// `SDL_QuitSubSystem(mask)`. When the last outstanding holder drops,
/// `SDL_Quit` runs to release SDL's own globals.
pub struct SubsystemGuard {
    lib: Arc<Sdl2Lib>,
    mask: u32,
}

impl SubsystemGuard {
    pub fn lib(&self) -> &Arc<Sdl2Lib> {
        &self.lib
    }
}

impl Drop for SubsystemGuard {
    fn drop(&mut self) {
        unsafe { (self.lib.SDL_QuitSubSystem)(self.mask) };
        // If this was the last engine, SDL_Quit to drain SDL's
        // remaining globals. SDL itself refcounts subsystems; we track
        // engines so we know when to call the top-level Quit.
        if SUBSYSTEM_HOLDERS.fetch_sub(1, Ordering::AcqRel) == 1 {
            unsafe { (self.lib.SDL_Quit)() };
        }
    }
}

/// Bring up libSDL2 plus the requested subsystems. `mask` is an OR of
/// `SDL_INIT_*` flags from [`sdl2_loader`].
pub fn acquire(mask: u32) -> Result<SubsystemGuard> {
    let l = lib()?;
    let prev = SUBSYSTEM_HOLDERS.fetch_add(1, Ordering::AcqRel);
    let rc = unsafe {
        if prev == 0 {
            // First engine — do a full SDL_Init so the core is up. SDL
            // itself permits nested SDL_Init calls, but SDL_Quit only
            // fully shuts down once we've matched them all.
            (l.SDL_Init)(mask)
        } else {
            (l.SDL_InitSubSystem)(mask)
        }
    };
    if rc != 0 {
        // Undo the refcount bump so the next attempt starts clean.
        SUBSYSTEM_HOLDERS.fetch_sub(1, Ordering::AcqRel);
        let err = l.last_error();
        return Err(Error::other(format!(
            "SDL subsystem init (mask={mask:#x}) failed: {err}"
        )));
    }
    Ok(SubsystemGuard { lib: l, mask })
}

/// Events subsystem alone — SDL spins events on video or audio
/// subsystem init, but the engines may want to opt in explicitly when
/// neither is used. Not currently called, kept for symmetry.
#[allow(dead_code)]
pub const EVENTS_MASK: u32 = ldr::SDL_INIT_EVENTS;
pub const VIDEO_MASK: u32 = ldr::SDL_INIT_VIDEO | ldr::SDL_INIT_EVENTS;
pub const AUDIO_MASK: u32 = ldr::SDL_INIT_AUDIO | ldr::SDL_INIT_EVENTS;
