// Sound devices for the sound panel: the outputs and inputs Windows has, choosing the default one, the microphone's level.
use std::{cell::RefCell, ffi::c_void, ptr::null_mut};
use windows::{
    Win32::{
        Foundation::PROPERTYKEY,
        Media::Audio::{DEVICE_STATE_ACTIVE, Endpoints::IAudioEndpointVolume, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eConsole, eRender},
        System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ},
    },
    core::{GUID, IUnknown, Interface},
};

pub struct Dev {
    pub id: Vec<u16>, // the endpoint id, zero-terminated
    pub name: String, // "Speakers (Realtek(R) Audio)"
    pub default: bool,
}

unsafe fn enumerator() -> Option<IMMDeviceEnumerator> {
    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()
}

unsafe fn id_of(d: &IMMDevice) -> Option<Vec<u16>> {
    let p = d.GetId().ok()?;
    let v = p.as_wide().iter().copied().chain([0]).collect();
    CoTaskMemFree(Some(p.0 as *const c_void));
    Some(v)
}

unsafe fn name_of(d: &IMMDevice) -> String {
    const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 }; // PKEY_Device_FriendlyName
    let Ok(store) = d.OpenPropertyStore(STGM_READ) else { return String::new() };
    let Ok(v) = store.GetValue(&FRIENDLY_NAME) else { return String::new() };
    // PROPVARIANT: the type at offset 0, the value at 8; VT_LPWSTR (31) holds a pointer to the text (freed when `v` drops)
    let raw = &v as *const _ as *const u8;
    let p = *(raw.add(8) as *const *const u16);
    if *(raw as *const u16) != 31 || p.is_null() {
        return String::new();
    }
    let n = (0..).take_while(|&i| *p.add(i) != 0).count();
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// The working outputs (or inputs, `capture`) in the order Windows lists them.
pub unsafe fn devices(capture: bool) -> Vec<Dev> {
    let Some(en) = enumerator() else { return vec![] };
    let flow = if capture { eCapture } else { eRender };
    let def = en.GetDefaultAudioEndpoint(flow, eConsole).ok().and_then(|d| id_of(&d));
    let Ok(col) = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { return vec![] };
    (0..col.GetCount().unwrap_or(0))
        .filter_map(|i| {
            let d = col.Item(i).ok()?;
            let id = id_of(&d)?;
            Some(Dev { name: name_of(&d), default: def.as_ref() == Some(&id), id })
        })
        .collect()
}

/// Makes the device the default one for everything (sounds, media, calls), as "Set as default device" in the sound
/// settings does. Windows has no documented call for it: this is the IPolicyConfig interface its own settings use.
pub unsafe fn set_default(id: &[u16]) -> bool {
    const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    const IPOLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    let Ok(unk) = CoCreateInstance::<_, IUnknown>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) else { return false };
    let mut pc: *mut c_void = null_mut();
    if unk.query(&IPOLICY_CONFIG, &mut pc).is_err() || pc.is_null() {
        return false;
    }
    // the interface is in no header: after IUnknown's three slots come ten format / property methods, then
    // SetDefaultEndpoint(device id, role) in slot 13. Slot 2 is Release.
    let vt = *(pc as *const *const usize);
    let set: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32 = std::mem::transmute(*vt.add(13));
    let ok = (0..3).fold(true, |ok, role| set(pc, id.as_ptr(), role) >= 0 && ok); // eConsole, eMultimedia, eCommunications
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vt.add(2));
    release(pc);
    MIC.set(None);
    ok
}

thread_local! {
    static MIC: RefCell<Option<IAudioEndpointVolume>> = const { RefCell::new(None) }; // the default microphone, looked up once per panel
}

/// Forgets the cached microphone (the panel opened, or the default device changed).
pub fn reset() {
    MIC.set(None);
}

unsafe fn mic_ep() -> Option<IAudioEndpointVolume> {
    MIC.with_borrow_mut(|m| {
        if m.is_none() {
            *m = enumerator().and_then(|en| en.GetDefaultAudioEndpoint(eCapture, eConsole).ok()).and_then(|d| d.Activate(CLSCTX_ALL, None).ok());
        }
        m.clone()
    })
}

/// (level 0..100, muted) of the default microphone; None when there is none.
pub unsafe fn mic() -> Option<(i32, bool)> {
    let v = mic_ep().and_then(|e| Some(((e.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as i32, e.GetMute().ok()?.as_bool())));
    if v.is_none() {
        MIC.set(None); // unplugged: look it up again next time
    }
    v
}

pub unsafe fn set_mic(v: i32) {
    if let Some(e) = mic_ep() {
        let _ = e.SetMasterVolumeLevelScalar(v.clamp(0, 100) as f32 / 100.0, std::ptr::null());
        let _ = e.SetMute(false, std::ptr::null());
    }
}

pub unsafe fn toggle_mic() {
    if let Some(e) = mic_ep() {
        let muted = e.GetMute().map(|m| m.as_bool()).unwrap_or(false);
        let _ = e.SetMute(!muted, std::ptr::null());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    #[test]
    fn default_output_can_be_set_to_itself() {
        // real system check of the undocumented call: choosing the device that already is the default changes nothing.
        // Skipped where there is no sound device (CI).
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let outs = devices(false);
            let Some(cur) = outs.iter().find(|d| d.default) else { return };
            assert!(!cur.name.is_empty());
            assert!(set_default(&cur.id));
            let after = devices(false);
            assert_eq!(after.iter().find(|d| d.default).map(|d| d.id.clone()), Some(cur.id.clone()));
        }
    }

    #[test]
    fn inputs_are_listed_with_at_most_one_default() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            assert!(devices(true).iter().filter(|d| d.default).count() <= 1);
        }
    }
}
