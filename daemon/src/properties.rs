use anyhow::bail;
use std::ffi::{CStr, CString, c_char};

const PROP_VALUE_MAX: usize = 92;

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> u32;
}

pub fn get(name: &str) -> anyhow::Result<Box<str>> {
    let name = CString::new(name)?;
    let mut buffer = [0u8; PROP_VALUE_MAX + 1];

    let len = unsafe { __system_property_get(name.as_ptr(), buffer.as_mut_ptr() as _) };

    if len == 0 {
        bail!("failed to get property value")
    }

    Ok(CStr::from_bytes_until_nul(&buffer)?
        .to_string_lossy()
        .into())
}
