//! macOS headphone-detection probe.
//!
//! Runtime-loads `CoreAudio.framework` through `libloading` (zero
//! build-time framework links) and queries:
//!
//! 1. `kAudioHardwarePropertyDefaultOutputDevice` — the system's
//!    current playback device.
//! 2. `kAudioDevicePropertyTransportType` on that device — bluetooth,
//!    USB, built-in, HDMI, etc.
//! 3. For built-in devices, `kAudioDevicePropertyDataSource` —
//!    distinguishes "Internal Speakers" from the headphone jack.
//! 4. As a fallback for unknown transports, the device's user-visible
//!    name (`kAudioObjectPropertyName`) is matched against a small
//!    list of well-known wireless-headphone model substrings (AirPods,
//!    Beats, …) — heuristic, but practical for the common case.
//!
//! ## Threading
//!
//! [`probe`] performs a small handful of synchronous HAL property
//! reads. Each call costs <1 ms on warm caches. The engine calls this
//! at stream-open and on a 1 Hz tick from its main loop — never from
//! the audio callback.
//!
//! ## Non-macOS
//!
//! On every other target_os this module is empty. The `cfg`-stub at the
//! bottom of this file returns [`HeadphoneStatus::Unknown`] so the
//! routing decision tree falls back to non-binaural downmix. Linux
//! (PulseAudio "port" introspection) and Windows (`MMDeviceEnumerator`
//! `IMMEndpoint::GetJackInfo`) are deferred.

use crate::drivers::audio_routing::HeadphoneStatus;

#[cfg(target_os = "macos")]
mod imp {
    #![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

    use std::ffi::c_void;
    use std::ptr;
    use std::sync::{Arc, Mutex, OnceLock};

    use libloading::{Library, Symbol};

    use super::HeadphoneStatus;

    type OSStatus = i32;
    type AudioObjectID = u32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AudioObjectPropertyAddress {
        mSelector: u32,
        mScope: u32,
        mElement: u32,
    }

    const fn four_cc(b: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*b)
    }

    const kAudioObjectSystemObject: AudioObjectID = 1;
    const kAudioObjectPropertyElementMain: u32 = 0;
    const kAudioObjectPropertyScopeGlobal: u32 = four_cc(b"glob");
    const kAudioObjectPropertyScopeOutput: u32 = four_cc(b"outp");

    const kAudioHardwarePropertyDefaultOutputDevice: u32 = four_cc(b"dOut");
    const kAudioDevicePropertyTransportType: u32 = four_cc(b"tran");
    const kAudioDevicePropertyDataSource: u32 = four_cc(b"ssrc");
    const kAudioObjectPropertyName: u32 = four_cc(b"lnam");

    // Transport type constants (CoreAudio.framework — AudioHardwareBase.h).
    const kAudioDeviceTransportTypeBuiltIn: u32 = four_cc(b"bltn");
    const kAudioDeviceTransportTypeBluetooth: u32 = four_cc(b"blue");
    const kAudioDeviceTransportTypeBluetoothLE: u32 = four_cc(b"blea");
    const kAudioDeviceTransportTypeUSB: u32 = four_cc(b"usb ");
    const kAudioDeviceTransportTypeHDMI: u32 = four_cc(b"hdmi");
    const kAudioDeviceTransportTypeDisplayPort: u32 = four_cc(b"dprt");
    const kAudioDeviceTransportTypeAirPlay: u32 = four_cc(b"airp");
    const kAudioDeviceTransportTypeAVB: u32 = four_cc(b"eavb");
    const kAudioDeviceTransportTypeThunderbolt: u32 = four_cc(b"thun");

    // Data-source FourCCs for the built-in jack on Mac hardware.
    // 'hdpn' = headphones, 'ispk' = internal speaker, 'imic' = internal mic.
    const kIOAudioOutputPortSubTypeHeadphones: u32 = four_cc(b"hdpn");
    const kIOAudioOutputPortSubTypeInternalSpeaker: u32 = four_cc(b"ispk");

    type Fn_AudioObjectGetPropertyData = unsafe extern "C" fn(
        inObjectID: AudioObjectID,
        inAddress: *const AudioObjectPropertyAddress,
        inQualifierDataSize: u32,
        inQualifierData: *const c_void,
        ioDataSize: *mut u32,
        outData: *mut c_void,
    ) -> OSStatus;

    type Fn_AudioObjectGetPropertyDataSize = unsafe extern "C" fn(
        inObjectID: AudioObjectID,
        inAddress: *const AudioObjectPropertyAddress,
        inQualifierDataSize: u32,
        inQualifierData: *const c_void,
        outDataSize: *mut u32,
    ) -> OSStatus;

    // CoreFoundation: CFStringRef → UTF8 conversion. Names come back
    // as a CFString and we need them as `String` for substring matching.
    type CFStringRef = *const c_void;
    type Boolean = u8;

    type Fn_CFStringGetLength = unsafe extern "C" fn(theString: CFStringRef) -> i64;
    type Fn_CFStringGetCString = unsafe extern "C" fn(
        theString: CFStringRef,
        buf: *mut u8,
        size: i64,
        encoding: u32,
    ) -> Boolean;
    type Fn_CFRelease = unsafe extern "C" fn(cf: *const c_void);

    const kCFStringEncodingUTF8: u32 = 0x0800_0100;

    #[allow(dead_code)]
    struct CaLib {
        _lib: Library,
        AudioObjectGetPropertyData: Fn_AudioObjectGetPropertyData,
        AudioObjectGetPropertyDataSize: Fn_AudioObjectGetPropertyDataSize,
    }

    unsafe impl Send for CaLib {}
    unsafe impl Sync for CaLib {}

    #[allow(dead_code)]
    struct CfLib {
        _lib: Library,
        CFStringGetLength: Fn_CFStringGetLength,
        CFStringGetCString: Fn_CFStringGetCString,
        CFRelease: Fn_CFRelease,
    }

    unsafe impl Send for CfLib {}
    unsafe impl Sync for CfLib {}

    fn ca_lib() -> Option<Arc<CaLib>> {
        static CACHED: OnceLock<Mutex<Option<Option<Arc<CaLib>>>>> = OnceLock::new();
        let slot = CACHED.get_or_init(|| Mutex::new(None));
        let mut g = slot.lock().unwrap();
        if let Some(loaded) = g.as_ref() {
            return loaded.clone();
        }
        const CANDIDATES: &[&str] = &[
            "/System/Library/Frameworks/CoreAudio.framework/CoreAudio",
            "CoreAudio.framework/CoreAudio",
            "CoreAudio",
        ];
        let lib = CANDIDATES
            .iter()
            .find_map(|p| unsafe { Library::new(*p) }.ok());
        let loaded = lib.and_then(|lib| unsafe {
            let get: Symbol<Fn_AudioObjectGetPropertyData> =
                lib.get(b"AudioObjectGetPropertyData\0").ok()?;
            let size: Symbol<Fn_AudioObjectGetPropertyDataSize> =
                lib.get(b"AudioObjectGetPropertyDataSize\0").ok()?;
            let g = *get;
            let s = *size;
            Some(Arc::new(CaLib {
                AudioObjectGetPropertyData: g,
                AudioObjectGetPropertyDataSize: s,
                _lib: lib,
            }))
        });
        *g = Some(loaded.clone());
        loaded
    }

    fn cf_lib() -> Option<Arc<CfLib>> {
        static CACHED: OnceLock<Mutex<Option<Option<Arc<CfLib>>>>> = OnceLock::new();
        let slot = CACHED.get_or_init(|| Mutex::new(None));
        let mut g = slot.lock().unwrap();
        if let Some(loaded) = g.as_ref() {
            return loaded.clone();
        }
        const CANDIDATES: &[&str] = &[
            "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation",
            "CoreFoundation.framework/CoreFoundation",
            "CoreFoundation",
        ];
        let lib = CANDIDATES
            .iter()
            .find_map(|p| unsafe { Library::new(*p) }.ok());
        let loaded = lib.and_then(|lib| unsafe {
            let len: Symbol<Fn_CFStringGetLength> = lib.get(b"CFStringGetLength\0").ok()?;
            let cstr: Symbol<Fn_CFStringGetCString> = lib.get(b"CFStringGetCString\0").ok()?;
            let rel: Symbol<Fn_CFRelease> = lib.get(b"CFRelease\0").ok()?;
            let l = *len;
            let c = *cstr;
            let r = *rel;
            Some(Arc::new(CfLib {
                CFStringGetLength: l,
                CFStringGetCString: c,
                CFRelease: r,
                _lib: lib,
            }))
        });
        *g = Some(loaded.clone());
        loaded
    }

    /// Read a single `u32` HAL property. Returns `None` if missing.
    unsafe fn get_u32(ca: &CaLib, object: AudioObjectID, selector: u32, scope: u32) -> Option<u32> {
        let addr = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut value: u32 = 0;
        let mut size: u32 = std::mem::size_of::<u32>() as u32;
        let r = (ca.AudioObjectGetPropertyData)(
            object,
            &addr,
            0,
            ptr::null(),
            &mut size,
            &mut value as *mut u32 as *mut c_void,
        );
        if r == 0 && size as usize == std::mem::size_of::<u32>() {
            Some(value)
        } else {
            None
        }
    }

    /// Read a CFString-typed property and convert to UTF-8. Returns
    /// `None` if missing or if the CFString → UTF-8 conversion fails.
    unsafe fn get_cfstring(
        ca: &CaLib,
        cf: &CfLib,
        object: AudioObjectID,
        selector: u32,
        scope: u32,
    ) -> Option<String> {
        let addr = AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut s: CFStringRef = ptr::null();
        let mut size: u32 = std::mem::size_of::<CFStringRef>() as u32;
        let r = (ca.AudioObjectGetPropertyData)(
            object,
            &addr,
            0,
            ptr::null(),
            &mut size,
            &mut s as *mut CFStringRef as *mut c_void,
        );
        if r != 0 || s.is_null() {
            return None;
        }
        let len = (cf.CFStringGetLength)(s);
        // UTF-8 worst case is 4× the CFString length in characters.
        let cap = (len * 4 + 1).max(64) as usize;
        let mut buf = vec![0u8; cap];
        let ok = (cf.CFStringGetCString)(s, buf.as_mut_ptr(), cap as i64, kCFStringEncodingUTF8);
        // Per the docs the caller owns the CFString that came back via
        // GetPropertyData and must release it.
        (cf.CFRelease)(s);
        if ok == 0 {
            return None;
        }
        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        std::str::from_utf8(&buf[..nul]).ok().map(String::from)
    }

    /// Wireless-headphone model substrings. These are matched
    /// case-insensitively against the device name returned by
    /// CoreAudio. Cheap heuristic — extend as new models ship.
    const HEADPHONE_NAME_HINTS: &[&str] = &[
        "airpods",
        "airpod",
        "beats",
        "powerbeats",
        "bose",
        "sony wh-",
        "sony wf-",
        "sennheiser",
        "jabra",
        "jbl",
        "skullcandy",
        "soundcore",
        "anker",
        "headphone",
        "headset",
        "earphone",
        "earbud",
        "buds",
    ];

    fn name_smells_like_headphones(name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        HEADPHONE_NAME_HINTS.iter().any(|h| lower.contains(h))
    }

    pub fn probe() -> HeadphoneStatus {
        let Some(ca) = ca_lib() else {
            return HeadphoneStatus::Unknown;
        };

        // 1. Get the default output device ID.
        let default_addr = AudioObjectPropertyAddress {
            mSelector: kAudioHardwarePropertyDefaultOutputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        };
        let mut device: AudioObjectID = 0;
        let mut size: u32 = std::mem::size_of::<AudioObjectID>() as u32;
        let r = unsafe {
            (ca.AudioObjectGetPropertyData)(
                kAudioObjectSystemObject,
                &default_addr,
                0,
                ptr::null(),
                &mut size,
                &mut device as *mut AudioObjectID as *mut c_void,
            )
        };
        if r != 0 || device == 0 {
            return HeadphoneStatus::Unknown;
        }

        // 2. Transport type — fast path for definitive yes/no answers.
        let transport = unsafe {
            get_u32(
                &ca,
                device,
                kAudioDevicePropertyTransportType,
                kAudioObjectPropertyScopeGlobal,
            )
        };

        match transport {
            Some(t)
                if t == kAudioDeviceTransportTypeHDMI
                    || t == kAudioDeviceTransportTypeDisplayPort
                    || t == kAudioDeviceTransportTypeAirPlay
                    || t == kAudioDeviceTransportTypeAVB
                    || t == kAudioDeviceTransportTypeThunderbolt =>
            {
                // Display / network output — definitely not headphones.
                HeadphoneStatus::No
            }
            Some(t) if t == kAudioDeviceTransportTypeBuiltIn => {
                // Built-in path — distinguish jack vs. speaker via the
                // active data source.
                let source = unsafe {
                    get_u32(
                        &ca,
                        device,
                        kAudioDevicePropertyDataSource,
                        kAudioObjectPropertyScopeOutput,
                    )
                };
                match source {
                    Some(s) if s == kIOAudioOutputPortSubTypeHeadphones => HeadphoneStatus::Yes,
                    Some(s) if s == kIOAudioOutputPortSubTypeInternalSpeaker => HeadphoneStatus::No,
                    // Built-in but no data-source distinction (rare —
                    // happens on Apple Silicon Mac mini's HDMI front-end
                    // sometimes) — fall back to name heuristic.
                    _ => name_probe(&ca, device).unwrap_or(HeadphoneStatus::No),
                }
            }
            Some(t)
                if t == kAudioDeviceTransportTypeBluetooth
                    || t == kAudioDeviceTransportTypeBluetoothLE =>
            {
                // BT could be a speaker too (BT speaker, car system).
                // Apply the name heuristic to disambiguate; default to
                // Yes (bluetooth-out is overwhelmingly headphones in
                // practice).
                name_probe(&ca, device).unwrap_or(HeadphoneStatus::Yes)
            }
            Some(t) if t == kAudioDeviceTransportTypeUSB => {
                // USB is split — DACs, audio interfaces, headphones.
                // Heuristic on the device name; fall back to Unknown if
                // we can't read a name (since USB isn't predominantly
                // headphones).
                name_probe(&ca, device).unwrap_or(HeadphoneStatus::Unknown)
            }
            _ => {
                // Unknown transport — try the name heuristic.
                name_probe(&ca, device).unwrap_or(HeadphoneStatus::Unknown)
            }
        }
    }

    fn name_probe(ca: &CaLib, device: AudioObjectID) -> Option<HeadphoneStatus> {
        let cf = cf_lib()?;
        let name = unsafe {
            get_cfstring(
                ca,
                &cf,
                device,
                kAudioObjectPropertyName,
                kAudioObjectPropertyScopeGlobal,
            )
        }?;
        Some(if name_smells_like_headphones(&name) {
            HeadphoneStatus::Yes
        } else {
            HeadphoneStatus::No
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn airpods_match_heuristic() {
            assert!(name_smells_like_headphones("Mark's AirPods Pro"));
            assert!(name_smells_like_headphones("Beats Studio3"));
            assert!(name_smells_like_headphones("Sony WH-1000XM5"));
            assert!(name_smells_like_headphones("USB Headset"));
        }

        #[test]
        fn external_speaker_does_not_match() {
            assert!(!name_smells_like_headphones("MacBook Pro Speakers"));
            assert!(!name_smells_like_headphones("Studio Display"));
            assert!(!name_smells_like_headphones("DENON AVR-X3700H"));
            assert!(!name_smells_like_headphones("BlackHole 2ch"));
        }

        #[test]
        fn probe_runs_without_panicking() {
            // The real probe; we don't assert a particular outcome
            // (depends on which device is the system default at test
            // time), just that it terminates and returns one of the
            // three enum values without crashing.
            let s = probe();
            match s {
                HeadphoneStatus::Yes | HeadphoneStatus::No | HeadphoneStatus::Unknown => {}
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn probe() -> HeadphoneStatus {
    imp::probe()
}

#[cfg(not(target_os = "macos"))]
pub fn probe() -> HeadphoneStatus {
    // Linux: PulseAudio sink-port introspection (`active_port` →
    // `Headphones` / `Headset`) is the right path; PipeWire exposes the
    // same through `pw-cli`. Wired-headphone detection on ALSA needs
    // jack-detection ioctls per-card. Out of scope this round.
    //
    // Windows: `MMDeviceEnumerator` + `IMMDevice::OpenPropertyStore`
    // exposes `PKEY_AudioEndpoint_FormFactor` (Headphones / Headset
    // values) and `PKEY_AudioEndpoint_JackSubType` for the wired path.
    // Out of scope this round.
    HeadphoneStatus::Unknown
}
