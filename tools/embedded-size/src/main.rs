use std::ffi::{CStr, CString};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let tx = CString::new(a.get(2).cloned().unwrap_or_default()).unwrap();
    let p: *mut std::ffi::c_char = unsafe {
        match a.get(1).map(String::as_str) {
            #[cfg(feature = "eth")]
            Some("ethereum") => embedded_size::vs_ethereum(tx.as_ptr()),
            #[cfg(feature = "tron")]
            Some("tron") => embedded_size::vs_tron(tx.as_ptr()),
            #[cfg(feature = "unspec")]
            Some("unspecified") => embedded_size::vs_unspecified(tx.as_ptr()),
            #[cfg(feature = "solana")]
            Some("solana") => embedded_size::vs_solana(tx.as_ptr()),
            #[cfg(feature = "sui")]
            Some("sui") => embedded_size::vs_sui(tx.as_ptr()),
            #[cfg(feature = "near")]
            Some("near") => embedded_size::vs_near(tx.as_ptr()),
            _ => std::ptr::null_mut(),
        }
    };
    if !p.is_null() {
        println!("{}", unsafe { CStr::from_ptr(p) }.to_string_lossy());
        unsafe { embedded_size::vs_free(p) }
    }
}
