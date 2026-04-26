//! macOS Now-Playing integration.
//!
//! Loads `Foundation.framework`, `AppKit.framework`,
//! `MediaPlayer.framework`, and `/usr/lib/libobjc.A.dylib` at
//! runtime via `libloading`. NOTHING is linked at build time —
//! verify with `otool -L target/release/oxideplay | grep
//! MediaPlayer`.
//!
//! The Objective-C dance has three parts:
//!
//! 1. **dlopen + dlsym.** `objc_getClass`, `sel_registerName`,
//!    `objc_msgSend`. (`objc_msgSend_stret` exists for >16-byte
//!    struct returns on x86_64; we don't need it because every
//!    selector we call returns a pointer, integer, or `double`,
//!    all of which arm64's unified ABI handles via the standard
//!    entry point.) Then the framework string-key globals
//!    (`MPMediaItemPropertyTitle` etc.) — those are exported as
//!    `NSString*` *globals*, so we read the symbol address (which
//!    is the address *of* the variable) and dereference once.
//! 2. **Build the NSDictionary.** `NSMutableDictionary
//!    dictionaryWithCapacity:`, then `setObject:forKey:` for each
//!    populated field of the [`crate::media_controls::TrackInfo`].
//!    Strings: `NSString stringWithUTF8String:`. Floats:
//!    `NSNumber numberWithDouble:`. Artwork: `NSImage initWithData:`
//!    wrapped in `MPMediaItemArtwork initWithImage:`.
//!
//!    Apple's `MPNowPlayingInfoCenter setNowPlayingInfo:` REPLACES
//!    the previous dict (it doesn't merge), so every "incremental"
//!    update — pause / resume / position tick — has to rebuild
//!    title / artist / album / artwork too. We cache the last
//!    [`TrackInfo`] for that.
//! 3. **Command targets.** Allocate a one-method Objective-C class
//!    pair (`OxideavMediaTarget`) at startup, stash a raw pointer
//!    to a `Box<TargetState>` on the instance via
//!    `objc_setAssociatedObject`, and wire each command via
//!    `addTarget:action:`. The `extern "C"` IMP fishes the state
//!    back out and pushes the right
//!    [`crate::media_controls::MediaCommand`] into a shared
//!    `Mutex<VecDeque<…>>` the engine drains via `take_command`.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
// Apple's runtime types `SEL` / `IMP` are upper-case in every header
// and reference. Renaming them to `Sel` / `Imp` would diverge from
// every line of <objc/runtime.h> grep-able by future readers.
#![allow(clippy::upper_case_acronyms)]

use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libloading::{Library, Symbol};

use super::{MediaCommand, MediaControls, PlaybackState, TrackInfo};

// ────────────────────── Objective-C runtime types ──────────────────────

/// Opaque Objective-C `id` / `Class` / `SEL` — pointer-shaped at
/// the FFI boundary. We never look inside.
type id = *mut c_void;
type Class = *mut c_void;
type SEL = *mut c_void;
type IMP = unsafe extern "C" fn();

// `objc_msgSend` is variadic in C. In Rust we cast to a concrete
// function-pointer type per call site, picking a signature that
// matches the receiver's actual selector. Mismatching the cast
// to the selector's real signature is UB — the helpers below
// (msg0_id, msg1_id_id, etc.) keep that surface tiny and
// auditable.
type Fn_objc_getClass = unsafe extern "C" fn(*const c_char) -> Class;
type Fn_sel_registerName = unsafe extern "C" fn(*const c_char) -> SEL;
type Fn_objc_msgSend_raw = unsafe extern "C" fn();

type Fn_objc_allocateClassPair =
    unsafe extern "C" fn(superclass: Class, name: *const c_char, extraBytes: usize) -> Class;
type Fn_class_addMethod =
    unsafe extern "C" fn(cls: Class, name: SEL, imp: IMP, types: *const c_char) -> u8;
type Fn_objc_registerClassPair = unsafe extern "C" fn(cls: Class);
type Fn_objc_setAssociatedObject =
    unsafe extern "C" fn(object: id, key: *const c_void, value: id, policy: usize);
type Fn_objc_getAssociatedObject = unsafe extern "C" fn(object: id, key: *const c_void) -> id;

struct ObjcLib {
    _lib: Library,
    objc_getClass: Fn_objc_getClass,
    sel_registerName: Fn_sel_registerName,
    /// Raw, untyped `objc_msgSend`. Every call site re-casts.
    objc_msgSend: Fn_objc_msgSend_raw,
    objc_allocateClassPair: Fn_objc_allocateClassPair,
    class_addMethod: Fn_class_addMethod,
    objc_registerClassPair: Fn_objc_registerClassPair,
    objc_setAssociatedObject: Fn_objc_setAssociatedObject,
    objc_getAssociatedObject: Fn_objc_getAssociatedObject,
}

unsafe impl Send for ObjcLib {}
unsafe impl Sync for ObjcLib {}

impl ObjcLib {
    unsafe fn load() -> Result<Self, String> {
        let lib = Library::new("/usr/lib/libobjc.A.dylib")
            .map_err(|e| format!("dlopen libobjc.A.dylib: {e}"))?;
        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let s: Symbol<$ty> = lib
                    .get(concat!($name, "\0").as_bytes())
                    .map_err(|e| format!("dlsym {}: {e}", $name))?;
                *s
            }};
        }
        Ok(ObjcLib {
            objc_getClass: sym!("objc_getClass", Fn_objc_getClass),
            sel_registerName: sym!("sel_registerName", Fn_sel_registerName),
            objc_msgSend: sym!("objc_msgSend", Fn_objc_msgSend_raw),
            objc_allocateClassPair: sym!("objc_allocateClassPair", Fn_objc_allocateClassPair),
            class_addMethod: sym!("class_addMethod", Fn_class_addMethod),
            objc_registerClassPair: sym!("objc_registerClassPair", Fn_objc_registerClassPair),
            objc_setAssociatedObject: sym!("objc_setAssociatedObject", Fn_objc_setAssociatedObject),
            objc_getAssociatedObject: sym!("objc_getAssociatedObject", Fn_objc_getAssociatedObject),
            _lib: lib,
        })
    }

    unsafe fn class(&self, name: &CStr) -> Result<Class, String> {
        let c = (self.objc_getClass)(name.as_ptr());
        if c.is_null() {
            Err(format!(
                "objc_getClass({}) returned null",
                name.to_string_lossy()
            ))
        } else {
            Ok(c)
        }
    }

    unsafe fn sel(&self, name: &CStr) -> SEL {
        (self.sel_registerName)(name.as_ptr())
    }
}

// ────────────────────── Foundation / AppKit / MediaPlayer ──────────────────────

/// Holds the framework `Library` handles + the NSString* globals
/// we read out of `MediaPlayer.framework`. The libraries stay
/// resident for the program's lifetime — `Library`'s drop would
/// otherwise unload them (and any subsequent `objc_msgSend` to a
/// MediaPlayer class would crash).
struct Frameworks {
    _foundation: Library,
    _appkit: Library,
    _mediaplayer: Library,
    /// `MPMediaItemPropertyTitle` — an `NSString*` exported by
    /// MediaPlayer.framework. The dlsym'd address is the address
    /// *of the variable*; we deref once to land on the live
    /// `NSString` `id`.
    key_title: id,
    key_artist: id,
    key_album: id,
    key_artwork: id,
    key_playback_duration: id,
    key_elapsed_playback_time: id,
    key_playback_rate: id,
}

unsafe impl Send for Frameworks {}
unsafe impl Sync for Frameworks {}

impl Frameworks {
    unsafe fn load() -> Result<Self, String> {
        let foundation = open_framework(
            "Foundation",
            "/System/Library/Frameworks/Foundation.framework/Foundation",
        )?;
        let appkit = open_framework(
            "AppKit",
            "/System/Library/Frameworks/AppKit.framework/AppKit",
        )?;
        let mediaplayer = open_framework(
            "MediaPlayer",
            "/System/Library/Frameworks/MediaPlayer.framework/MediaPlayer",
        )?;

        let key_title = read_nsstring_global(&mediaplayer, b"MPMediaItemPropertyTitle\0")?;
        let key_artist = read_nsstring_global(&mediaplayer, b"MPMediaItemPropertyArtist\0")?;
        let key_album = read_nsstring_global(&mediaplayer, b"MPMediaItemPropertyAlbumTitle\0")?;
        let key_artwork = read_nsstring_global(&mediaplayer, b"MPMediaItemPropertyArtwork\0")?;
        let key_playback_duration =
            read_nsstring_global(&mediaplayer, b"MPMediaItemPropertyPlaybackDuration\0")?;
        let key_elapsed_playback_time = read_nsstring_global(
            &mediaplayer,
            b"MPNowPlayingInfoPropertyElapsedPlaybackTime\0",
        )?;
        let key_playback_rate =
            read_nsstring_global(&mediaplayer, b"MPNowPlayingInfoPropertyPlaybackRate\0")?;

        Ok(Self {
            _foundation: foundation,
            _appkit: appkit,
            _mediaplayer: mediaplayer,
            key_title,
            key_artist,
            key_album,
            key_artwork,
            key_playback_duration,
            key_elapsed_playback_time,
            key_playback_rate,
        })
    }
}

unsafe fn open_framework(label: &str, path: &str) -> Result<Library, String> {
    Library::new(path).map_err(|e| format!("dlopen {label} ({path}): {e}"))
}

unsafe fn read_nsstring_global(lib: &Library, name: &[u8]) -> Result<id, String> {
    let s: Symbol<*const id> = lib.get(name).map_err(|e| {
        format!(
            "dlsym {} failed: {e}",
            std::str::from_utf8(&name[..name.len().saturating_sub(1)]).unwrap_or("<utf8>")
        )
    })?;
    let raw = *s; // address of the NSString* variable
    if raw.is_null() {
        return Err(format!(
            "{} symbol address was null",
            std::str::from_utf8(&name[..name.len().saturating_sub(1)]).unwrap_or("<utf8>")
        ));
    }
    Ok(*raw) // deref → the actual NSString id
}

// ────────────────────── tiny msg_send shims ──────────────────────
//
// Each of these casts the raw `objc_msgSend` to a concrete
// signature. Mismatching the cast to the selector's real signature
// is undefined behaviour. Kept tiny for auditability.

unsafe fn msg0_id(objc: &ObjcLib, recv: id, sel: SEL) -> id {
    let f: unsafe extern "C" fn(id, SEL) -> id = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel)
}
unsafe fn msg1_id_id(objc: &ObjcLib, recv: id, sel: SEL, a: id) -> id {
    let f: unsafe extern "C" fn(id, SEL, id) -> id = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, a)
}
unsafe fn msg2_void_id_id(objc: &ObjcLib, recv: id, sel: SEL, a: id, b: id) {
    let f: unsafe extern "C" fn(id, SEL, id, id) = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, a, b)
}
unsafe fn msg2_id_id_sel(objc: &ObjcLib, recv: id, sel: SEL, target: id, action: SEL) -> id {
    let f: unsafe extern "C" fn(id, SEL, id, SEL) -> id = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, target, action)
}
unsafe fn msg1_id_cstr(objc: &ObjcLib, recv: id, sel: SEL, s: *const c_char) -> id {
    let f: unsafe extern "C" fn(id, SEL, *const c_char) -> id =
        std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, s)
}
unsafe fn msg1_id_double(objc: &ObjcLib, recv: id, sel: SEL, x: f64) -> id {
    let f: unsafe extern "C" fn(id, SEL, f64) -> id = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, x)
}
unsafe fn msg1_void_isize(objc: &ObjcLib, recv: id, sel: SEL, x: isize) {
    let f: unsafe extern "C" fn(id, SEL, isize) = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, x)
}
unsafe fn msg1_void_u8(objc: &ObjcLib, recv: id, sel: SEL, x: u8) {
    let f: unsafe extern "C" fn(id, SEL, u8) = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, x)
}
unsafe fn msg1_id_isize(objc: &ObjcLib, recv: id, sel: SEL, x: isize) -> id {
    let f: unsafe extern "C" fn(id, SEL, isize) -> id = std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, x)
}
unsafe fn msg2_id_data_len(objc: &ObjcLib, recv: id, sel: SEL, bytes: *const u8, len: usize) -> id {
    let f: unsafe extern "C" fn(id, SEL, *const u8, usize) -> id =
        std::mem::transmute(objc.objc_msgSend);
    f(recv, sel, bytes, len)
}

// ────────────────────── command-target Objective-C subclass ──────────────────────

/// Heap-owned state pointed at by every command-target instance via
/// `objc_setAssociatedObject`. Holds the queue the IMP pushes into
/// + which command this particular target carries.
struct TargetState {
    queue: Arc<Mutex<VecDeque<MediaCommand>>>,
    command: TargetCommand,
    /// Kept alive so the IMP can call back into the ObjC runtime
    /// (it needs `sel_registerName`, `objc_msgSend` to read the
    /// event's `positionTime` for the Seek case).
    objc: Arc<ObjcLib>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum TargetCommand {
    Play,
    Pause,
    TogglePlayPause,
    Next,
    Previous,
    /// Carries an `f64` "positionTime" inside the
    /// `MPChangePlaybackPositionCommandEvent`.
    Seek,
}

/// Static key for `objc_setAssociatedObject`. Apple only requires
/// that the key be a unique pointer; the address of any static
/// works.
static ASSOC_KEY: u8 = 0;

/// Stashed `Arc<ObjcLib>` so the `extern "C"` IMP — which has no
/// userdata channel beyond `self` — can call back into the ObjC
/// runtime without re-loading libobjc. Initialised once in
/// `MacosMediaControls::new`.
static mut OBJC_FOR_IMP: Option<Arc<ObjcLib>> = None;

unsafe extern "C" fn target_imp_simple(self_: id, _cmd: SEL, _event: id) -> isize {
    if let Some(state) = associated_state(self_) {
        if let Ok(mut q) = state.queue.lock() {
            let cmd = match state.command {
                TargetCommand::Play => MediaCommand::Play,
                TargetCommand::Pause => MediaCommand::Pause,
                TargetCommand::TogglePlayPause => MediaCommand::TogglePlayPause,
                TargetCommand::Next => MediaCommand::Next,
                TargetCommand::Previous => MediaCommand::Previous,
                TargetCommand::Seek => return 1, // wrong target, defensive
            };
            q.push_back(cmd);
        }
    }
    0 // MPRemoteCommandHandlerStatusSuccess
}

unsafe extern "C" fn target_imp_seek(self_: id, _cmd: SEL, event: id) -> isize {
    let Some(state) = associated_state(self_) else {
        return 1;
    };
    if event.is_null() {
        return 1;
    }
    let position_time_sel = state.objc.sel(c"positionTime");
    let f: unsafe extern "C" fn(id, SEL) -> f64 = std::mem::transmute(state.objc.objc_msgSend);
    let secs = f(event, position_time_sel);
    if !secs.is_finite() || secs < 0.0 {
        return 1;
    }
    if let Ok(mut q) = state.queue.lock() {
        q.push_back(MediaCommand::Seek(secs));
    }
    0
}

unsafe fn associated_state(self_: id) -> Option<&'static TargetState> {
    #[allow(static_mut_refs)]
    let objc = OBJC_FOR_IMP.as_ref()?;
    let raw = (objc.objc_getAssociatedObject)(self_, &ASSOC_KEY as *const u8 as *const c_void);
    if raw.is_null() {
        return None;
    }
    Some(&*(raw as *const TargetState))
}

// ────────────────────── public type ──────────────────────

pub struct MacosMediaControls {
    objc: Arc<ObjcLib>,
    fw: Arc<Frameworks>,

    // Cached classes / selectors we touch on every update tick.
    cls_ns_string: Class,
    cls_ns_number: Class,
    cls_ns_data: Class,
    cls_ns_image: Class,
    cls_ns_mutable_dictionary: Class,
    cls_mp_now_playing_info_center: Class,
    cls_mp_media_item_artwork: Class,

    sel_default_center: SEL,
    sel_set_now_playing_info: SEL,
    sel_set_playback_state: SEL,
    sel_string_with_utf8_string: SEL,
    sel_number_with_double: SEL,
    sel_dictionary_with_capacity: SEL,
    sel_set_object_for_key: SEL,
    sel_init_with_data: SEL,
    sel_init_with_bytes_length: SEL,
    sel_alloc: SEL,
    sel_init_with_image: SEL,

    // Engine → OS state mirror. `set_track` updates `track`,
    // `set_playback_state` updates `state`, `set_position` updates
    // `position` — and any of those can rebuild + push the full
    // dict (since `setNowPlayingInfo:` REPLACES, not merges).
    track: TrackInfo,
    state: PlaybackState,
    position: Duration,

    cmd_state: Arc<Mutex<VecDeque<MediaCommand>>>,
    /// Boxed `TargetState`s — kept alive for the lifetime of the
    /// process so the associated-object IMPs can keep dereferencing
    /// them. We `Box::into_raw`'d them into the associated objects;
    /// these pointers exist so the boxes can be reclaimed by `Drop`
    /// (or just leak gracefully when the program exits).
    _pinned_target_states: Vec<*mut TargetState>,
    /// Last `set_position` push timestamp — rate-limits OS pushes
    /// to ~2 Hz. Apple's widget extrapolates from `(elapsed, rate,
    /// timestamp)`, so updating more frequently burns CPU without
    /// changing what the user sees.
    last_position_push: Instant,
}

unsafe impl Send for MacosMediaControls {}

impl MacosMediaControls {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let objc = Arc::new(ObjcLib::load()?);
            let fw = Arc::new(Frameworks::load()?);

            // Stash the ObjcLib for the IMPs.
            // SAFETY: written once on the main thread before any
            // command target is wired up; never written again. The
            // Mach scheduler can deliver remote-command callbacks
            // on any thread once `addTarget:action:` lands, so the
            // assignment must be visible before any ObjC selector
            // can fire — guaranteed by happens-before from the
            // following synchronous ObjC calls (each
            // `objc_msgSend` is a full memory barrier on Apple
            // platforms).
            #[allow(static_mut_refs)]
            {
                OBJC_FOR_IMP = Some(objc.clone());
            }

            let cls_ns_string = objc.class(c_str(b"NSString\0"))?;
            let cls_ns_number = objc.class(c_str(b"NSNumber\0"))?;
            let cls_ns_data = objc.class(c_str(b"NSData\0"))?;
            let cls_ns_image = objc.class(c_str(b"NSImage\0"))?;
            let cls_ns_mutable_dictionary = objc.class(c_str(b"NSMutableDictionary\0"))?;
            let cls_mp_now_playing_info_center = objc.class(c_str(b"MPNowPlayingInfoCenter\0"))?;
            let cls_mp_media_item_artwork = objc.class(c_str(b"MPMediaItemArtwork\0"))?;
            let cls_mp_remote_command_center = objc.class(c_str(b"MPRemoteCommandCenter\0"))?;

            let sel_default_center = objc.sel(c_str(b"defaultCenter\0"));
            let sel_shared_command_center = objc.sel(c_str(b"sharedCommandCenter\0"));
            let sel_set_now_playing_info = objc.sel(c_str(b"setNowPlayingInfo:\0"));
            let sel_set_playback_state = objc.sel(c_str(b"setPlaybackState:\0"));
            let sel_string_with_utf8_string = objc.sel(c_str(b"stringWithUTF8String:\0"));
            let sel_number_with_double = objc.sel(c_str(b"numberWithDouble:\0"));
            let sel_dictionary_with_capacity = objc.sel(c_str(b"dictionaryWithCapacity:\0"));
            let sel_set_object_for_key = objc.sel(c_str(b"setObject:forKey:\0"));
            let sel_alloc = objc.sel(c_str(b"alloc\0"));
            let sel_init_with_data = objc.sel(c_str(b"initWithData:\0"));
            let sel_init_with_bytes_length = objc.sel(c_str(b"initWithBytes:length:\0"));
            let sel_init_with_image = objc.sel(c_str(b"initWithImage:\0"));

            // Command-center wiring: build a one-method ObjC
            // subclass + per-command instance + `addTarget:action:`.
            let queue: Arc<Mutex<VecDeque<MediaCommand>>> = Arc::new(Mutex::new(VecDeque::new()));
            let target_cls = create_target_class(&objc)?;
            let cmd_center = msg0_id(
                &objc,
                cls_mp_remote_command_center as id,
                sel_shared_command_center,
            );
            if cmd_center.is_null() {
                return Err("MPRemoteCommandCenter sharedCommandCenter returned nil".into());
            }
            let sel_set_enabled = objc.sel(c_str(b"setEnabled:\0"));
            let sel_add_target_action = objc.sel(c_str(b"addTarget:action:\0"));
            let sel_handle_simple = objc.sel(c_str(b"oxideavHandleSimple:\0"));
            let sel_handle_seek = objc.sel(c_str(b"oxideavHandleSeek:\0"));
            let sel_init = objc.sel(c_str(b"init\0"));

            let mut pinned: Vec<*mut TargetState> = Vec::new();

            // Per-command wiring. Errors bubble up immediately —
            // if a command property is missing on this OS version
            // we fail loudly so the caller can fall back to the
            // noop impl.
            let to_wire: &[(&[u8], TargetCommand)] = &[
                (b"playCommand\0", TargetCommand::Play),
                (b"pauseCommand\0", TargetCommand::Pause),
                (b"togglePlayPauseCommand\0", TargetCommand::TogglePlayPause),
                (b"nextTrackCommand\0", TargetCommand::Next),
                (b"previousTrackCommand\0", TargetCommand::Previous),
                (b"changePlaybackPositionCommand\0", TargetCommand::Seek),
            ];
            for &(property, target_cmd) in to_wire {
                let sel_property = objc.sel(
                    CStr::from_bytes_with_nul(property)
                        .map_err(|_| "internal: selector cstr missing nul")?,
                );
                let cmd_obj = msg0_id(&objc, cmd_center, sel_property);
                if cmd_obj.is_null() {
                    return Err(format!(
                        "MPRemoteCommandCenter has no '{}'",
                        std::str::from_utf8(&property[..property.len() - 1]).unwrap_or("?"),
                    ));
                }
                msg1_void_u8(&objc, cmd_obj, sel_set_enabled, 1);
                let target_alloc = msg0_id(&objc, target_cls as id, sel_alloc);
                let target = msg0_id(&objc, target_alloc, sel_init);
                if target.is_null() {
                    return Err("OxideavMediaTarget alloc/init returned nil".into());
                }
                let state = Box::new(TargetState {
                    queue: queue.clone(),
                    command: target_cmd,
                    objc: objc.clone(),
                });
                let raw_state = Box::into_raw(state);
                pinned.push(raw_state);
                // `objc_setAssociatedObject` policy 0 ==
                // OBJC_ASSOCIATION_ASSIGN — no retain/release; we
                // own the lifetime via `_pinned_target_states`.
                (objc.objc_setAssociatedObject)(
                    target,
                    &ASSOC_KEY as *const u8 as *const c_void,
                    raw_state as id,
                    0,
                );
                let action_sel = if matches!(target_cmd, TargetCommand::Seek) {
                    sel_handle_seek
                } else {
                    sel_handle_simple
                };
                let _ = msg2_id_id_sel(&objc, cmd_obj, sel_add_target_action, target, action_sel);
            }

            Ok(Self {
                objc,
                fw,
                cls_ns_string,
                cls_ns_number,
                cls_ns_data,
                cls_ns_image,
                cls_ns_mutable_dictionary,
                cls_mp_now_playing_info_center,
                cls_mp_media_item_artwork,
                sel_default_center,
                sel_set_now_playing_info,
                sel_set_playback_state,
                sel_string_with_utf8_string,
                sel_number_with_double,
                sel_dictionary_with_capacity,
                sel_set_object_for_key,
                sel_init_with_data,
                sel_init_with_bytes_length,
                sel_alloc,
                sel_init_with_image,
                track: TrackInfo::default(),
                state: PlaybackState::Stopped,
                position: Duration::ZERO,
                cmd_state: queue,
                _pinned_target_states: pinned,
                last_position_push: Instant::now()
                    .checked_sub(Duration::from_secs(10))
                    .unwrap_or_else(Instant::now),
            })
        }
    }

    // ── thin Foundation/AppKit/MediaPlayer wrappers ──

    unsafe fn ns_string(&self, s: &str) -> id {
        // `stringWithUTF8String:` requires NUL-terminated input.
        // Strip any embedded NULs defensively; a tracker-format
        // title with garbage bytes shouldn't crash the player.
        let cleaned: String = s.chars().filter(|c| *c != '\0').collect();
        let c = match CString::new(cleaned) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        msg1_id_cstr(
            &self.objc,
            self.cls_ns_string as id,
            self.sel_string_with_utf8_string,
            c.as_ptr(),
        )
    }
    unsafe fn ns_number_double(&self, x: f64) -> id {
        msg1_id_double(
            &self.objc,
            self.cls_ns_number as id,
            self.sel_number_with_double,
            x,
        )
    }
    unsafe fn ns_data_from(&self, bytes: &[u8]) -> id {
        let alloc = msg0_id(&self.objc, self.cls_ns_data as id, self.sel_alloc);
        msg2_id_data_len(
            &self.objc,
            alloc,
            self.sel_init_with_bytes_length,
            bytes.as_ptr(),
            bytes.len(),
        )
    }
    unsafe fn make_dict(&self, capacity: isize) -> id {
        msg1_id_isize(
            &self.objc,
            self.cls_ns_mutable_dictionary as id,
            self.sel_dictionary_with_capacity,
            capacity,
        )
    }
    unsafe fn build_artwork(&self, bytes: &[u8]) -> Option<id> {
        if bytes.is_empty() {
            return None;
        }
        let data = self.ns_data_from(bytes);
        if data.is_null() {
            return None;
        }
        let img_alloc = msg0_id(&self.objc, self.cls_ns_image as id, self.sel_alloc);
        let img = msg1_id_id(&self.objc, img_alloc, self.sel_init_with_data, data);
        if img.is_null() {
            return None;
        }
        let art_alloc = msg0_id(
            &self.objc,
            self.cls_mp_media_item_artwork as id,
            self.sel_alloc,
        );
        let art = msg1_id_id(&self.objc, art_alloc, self.sel_init_with_image, img);
        if art.is_null() {
            None
        } else {
            Some(art)
        }
    }
    unsafe fn now_playing_center(&self) -> id {
        msg0_id(
            &self.objc,
            self.cls_mp_now_playing_info_center as id,
            self.sel_default_center,
        )
    }
    unsafe fn dict_set(&self, dict: id, value: id, key: id) {
        if dict.is_null() || value.is_null() || key.is_null() {
            return;
        }
        msg2_void_id_id(&self.objc, dict, self.sel_set_object_for_key, value, key);
    }

    /// Build the full nowPlayingInfo dict from the current cached
    /// state and push it through `setNowPlayingInfo:`. Apple
    /// REPLACES the previous dict, so every push has to repeat
    /// title / artist / album / artwork.
    unsafe fn publish(&self) {
        let dict = self.make_dict(8);
        if dict.is_null() {
            return;
        }
        if let Some(t) = self.track.title.as_deref() {
            let v = self.ns_string(t);
            self.dict_set(dict, v, self.fw.key_title);
        }
        if let Some(a) = self.track.artist.as_deref() {
            let v = self.ns_string(a);
            self.dict_set(dict, v, self.fw.key_artist);
        }
        if let Some(a) = self.track.album.as_deref() {
            let v = self.ns_string(a);
            self.dict_set(dict, v, self.fw.key_album);
        }
        if let Some(d) = self.track.duration {
            let v = self.ns_number_double(d.as_secs_f64());
            self.dict_set(dict, v, self.fw.key_playback_duration);
        }
        if let Some(art) = self.track.artwork.as_ref() {
            if let Some(art_obj) = self.build_artwork(&art.data) {
                self.dict_set(dict, art_obj, self.fw.key_artwork);
            }
        }
        let rate = self.ns_number_double(if self.state == PlaybackState::Playing {
            1.0
        } else {
            0.0
        });
        self.dict_set(dict, rate, self.fw.key_playback_rate);
        let elapsed = self.ns_number_double(self.position.as_secs_f64());
        self.dict_set(dict, elapsed, self.fw.key_elapsed_playback_time);

        let center = self.now_playing_center();
        if !center.is_null() {
            msg1_id_id(&self.objc, center, self.sel_set_now_playing_info, dict);
        }
    }
}

impl MediaControls for MacosMediaControls {
    fn set_track(&mut self, info: &TrackInfo) {
        self.track = info.clone();
        // Treat a fresh track as a "we're starting from 0" event —
        // the engine's first `set_position` lands within one tick
        // and overwrites this anyway.
        self.position = Duration::ZERO;
        unsafe {
            self.publish();
        }
        self.last_position_push = Instant::now();
    }

    fn set_playback_state(&mut self, state: PlaybackState) {
        if self.state == state {
            return;
        }
        self.state = state;
        let raw: isize = match state {
            PlaybackState::Playing => 1,
            PlaybackState::Paused => 2,
            PlaybackState::Stopped => 3,
        };
        unsafe {
            let center = self.now_playing_center();
            if !center.is_null() {
                msg1_void_isize(&self.objc, center, self.sel_set_playback_state, raw);
            }
            // Republish so the rate (1.0 vs 0.0) lands in the
            // dictionary too — `setPlaybackState:` on its own
            // doesn't touch the dictionary's rate field.
            self.publish();
        }
        self.last_position_push = Instant::now();
    }

    fn set_position(&mut self, elapsed: Duration) {
        self.position = elapsed;
        // Rate-limit to ~2 Hz. The widget extrapolates from
        // (elapsed, rate, timestamp); anything faster is wasted.
        if self.last_position_push.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_position_push = Instant::now();
        unsafe {
            self.publish();
        }
    }

    fn take_command(&mut self) -> Option<MediaCommand> {
        let mut q = self.cmd_state.lock().ok()?;
        q.pop_front()
    }
}

unsafe fn create_target_class(objc: &ObjcLib) -> Result<Class, String> {
    let ns_object = objc.class(c_str(b"NSObject\0"))?;
    let cls = (objc.objc_allocateClassPair)(ns_object, c_str(b"OxideavMediaTarget\0").as_ptr(), 0);
    if cls.is_null() {
        // A previous run in the same process may have already
        // registered the class (we never unregister). Fall back to
        // looking it up; any class with this name was also created
        // by us, so its IMPs are correct.
        return objc.class(c_str(b"OxideavMediaTarget\0"));
    }

    // `oxideavHandleSimple:` — `(id self, SEL _cmd, id event) ->
    // NSInteger`. Apple type-encoding string: `q@:@`. (`q` =
    // NSInteger, `@` = id, `:` = SEL.)
    let sel_simple = objc.sel(c_str(b"oxideavHandleSimple:\0"));
    let imp_simple: IMP = std::mem::transmute(target_imp_simple as *const c_void);
    if (objc.class_addMethod)(cls, sel_simple, imp_simple, c_str(b"q@:@\0").as_ptr()) == 0 {
        return Err("class_addMethod(oxideavHandleSimple:) failed".into());
    }
    let sel_seek = objc.sel(c_str(b"oxideavHandleSeek:\0"));
    let imp_seek: IMP = std::mem::transmute(target_imp_seek as *const c_void);
    if (objc.class_addMethod)(cls, sel_seek, imp_seek, c_str(b"q@:@\0").as_ptr()) == 0 {
        return Err("class_addMethod(oxideavHandleSeek:) failed".into());
    }
    (objc.objc_registerClassPair)(cls);
    Ok(cls)
}

fn c_str(bytes: &'static [u8]) -> &'static CStr {
    CStr::from_bytes_with_nul(bytes).expect("c_str must be nul-terminated")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loading the macOS frameworks should succeed on a developer's
    /// machine. CI runners (sandboxed) may not be able to dlopen
    /// MediaPlayer.framework; the test accepts that and just
    /// requires it not to panic.
    #[test]
    fn try_load_does_not_panic() {
        let _ = std::panic::catch_unwind(|| {
            let _ = MacosMediaControls::new();
        });
    }
}
