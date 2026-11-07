//! Reference/PoC only: a minimal C-ABI wrapper used to measure how small an
//! embedded build of the parser can be. Each chain is behind a feature so a
//! build links only what it calls. See tools/embedded-size/README.md.

use std::ffi::{CStr, CString, c_char};
use visualsign::vsptrait::VisualSignOptions;

fn to_json(
    r: Result<visualsign::SignablePayload, visualsign::vsptrait::VisualSignError>,
) -> *mut c_char {
    let s = match r {
        Ok(p) => serde_json::to_string(&p).unwrap_or_default(),
        Err(e) => format!("error: {e}"),
    };
    CString::new(s)
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

unsafe fn input<'a>(p: *const c_char) -> &'a str {
    unsafe { CStr::from_ptr(p) }.to_str().unwrap_or("")
}

#[cfg(feature = "eth")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_ethereum(tx: *const c_char) -> *mut c_char {
    to_json(visualsign_ethereum::transaction_string_to_visual_sign(
        unsafe { input(tx) },
        VisualSignOptions::default(),
    ))
}
#[cfg(feature = "tron")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_tron(tx: *const c_char) -> *mut c_char {
    to_json(visualsign_tron::transaction_string_to_visual_sign(
        unsafe { input(tx) },
        VisualSignOptions::default(),
    ))
}
#[cfg(feature = "unspec")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_unspecified(tx: *const c_char) -> *mut c_char {
    to_json(visualsign_unspecified::transaction_string_to_visual_sign(
        unsafe { input(tx) },
        VisualSignOptions::default(),
    ))
}
#[cfg(feature = "solana")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_solana(tx: *const c_char) -> *mut c_char {
    to_json(visualsign_solana::transaction_string_to_visual_sign(
        unsafe { input(tx) },
        VisualSignOptions::default(),
    ))
}
#[cfg(feature = "sui")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_sui(tx: *const c_char) -> *mut c_char {
    to_json(visualsign_sui::transaction_string_to_visual_sign(
        unsafe { input(tx) },
        VisualSignOptions::default(),
    ))
}
#[cfg(feature = "near")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_near(tx: *const c_char) -> *mut c_char {
    use visualsign::vsptrait::VisualSignConverterFromString;
    to_json(
        visualsign_near::NearVisualSignConverter::default()
            .to_visual_sign_payload_from_string(unsafe { input(tx) }, VisualSignOptions::default())
            .map(|r| r.payload),
    )
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vs_free(p: *mut c_char) {
    if !p.is_null() {
        drop(unsafe { CString::from_raw(p) });
    }
}
