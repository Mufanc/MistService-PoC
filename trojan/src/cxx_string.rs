use anyhow::{Context, bail};
use log::warn;
use nix::libc::c_char;
use procfs::process::{MMapPath, Process};
use r3solvr::{BasicResolver, Query, SymbolResolver};
use std::ffi::c_void;
use std::sync::LazyLock;
use std::{mem, slice};

static IS_ALTERNATE: LazyLock<Option<bool>> = LazyLock::new(|| {
    let result: anyhow::Result<bool> = (|| {
        let (pathname, base) = Process::myself()?
            .maps()?
            .into_iter()
            .find_map(|map| {
                if let MMapPath::Path(pathname) = map.pathname
                    && pathname.to_string_lossy().ends_with("/libc++.so")
                {
                    Some((pathname, map.address.0 as usize))
                } else {
                    None
                }
            })
            .context("failed to find libc++ in maps")?;

        let resolver = BasicResolver::from_file(pathname)?;

        // std::__1::basic_string<char, std::__1::char_traits<char>, std::__1::allocator<char>>::basic_string(char const*)
        let ctor = resolver.lookup_symbol(
            Query::new("_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEEC2EPKc")
                .with_debugdata(true),
        )?;

        // std::__1::basic_string<char, std::__1::char_traits<char>, std::__1::allocator<char>>::~basic_string()
        let dtor = resolver.lookup_symbol(
            Query::new("_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEED2Ev")
                .with_debugdata(true),
        )?;

        let ctor_fn: extern "C" fn(*const c_void, *const c_char) =
            unsafe { mem::transmute(base + ctor.addr) };

        let dtor_fn: extern "C" fn(*const c_void) = unsafe { mem::transmute(base + dtor.addr) };

        let mut buffer = [0u8; 64];
        let mut data = [0xffu8; 23];
        data[22] = 0;

        ctor_fn(buffer.as_mut_ptr() as _, data.as_ptr() as *const c_char);
        let is_alternate = buffer[0] == 0xff;
        dtor_fn(buffer.as_mut_ptr() as _);

        Ok(is_alternate)
    })();

    if let Err(err) = &result {
        warn!("failed to determine if libc++ is alternate layout: {err:?}")
    }

    result.ok()
});

pub struct CxxStr<'a>(StrType<'a>);

enum StrType<'a> {
    LongDefault(&'a LongDefault),
    LongAlternate(&'a LongAlternate),
    ShortDefault(&'a ShortDefault),
    ShortAlternate(&'a ShortAlternate),
}

#[repr(C)]
struct LongDefault {
    capacity_with_flag: usize,
    size: usize,
    data: *const c_char,
}

#[repr(C)]
struct LongAlternate {
    data: *const c_char,
    size: usize,
    capacity_with_flag: usize,
}

#[repr(C)]
struct ShortDefault {
    size_with_flag: u8,
    data: [u8; 23],
}

#[repr(C)]
struct ShortAlternate {
    data: [u8; 23],
    size_with_flag: u8,
}

impl<'a> CxxStr<'a> {
    pub unsafe fn from_ptr(ptr: *const c_void) -> anyhow::Result<CxxStr<'a>> {
        match &*IS_ALTERNATE {
            Some(is_alternate) => {
                let data = unsafe { slice::from_raw_parts(ptr as *const u8, 24) };

                let is_long = if *is_alternate {
                    ((data[23] >> 7) & 1) != 0
                } else {
                    (data[0] & 1) != 0
                };

                let str = unsafe {
                    match (is_long, is_alternate) {
                        (true, false) => {
                            CxxStr(StrType::LongDefault(&*(ptr as *const LongDefault)))
                        }
                        (true, true) => {
                            CxxStr(StrType::LongAlternate(&*(ptr as *const LongAlternate)))
                        }
                        (false, false) => {
                            CxxStr(StrType::ShortDefault(&*(ptr as *const ShortDefault)))
                        }
                        (false, true) => {
                            CxxStr(StrType::ShortAlternate(&*(ptr as *const ShortAlternate)))
                        }
                    }
                };

                Ok(str)
            }
            None => bail!("failed to determine if libc++ is alternate layout"),
        }
    }
}

impl CxxStr<'_> {
    pub fn len(&self) -> usize {
        match self.0 {
            StrType::LongDefault(str) => str.size,
            StrType::LongAlternate(str) => str.size,
            StrType::ShortDefault(str) => (str.size_with_flag >> 1) as usize,
            StrType::ShortAlternate(str) => (str.size_with_flag & 0x7f) as usize,
        }
    }

    pub fn to_str(&self) -> &str {
        let length = self.len();

        unsafe {
            match self.0 {
                StrType::LongDefault(str) => {
                    std::str::from_utf8_unchecked(slice::from_raw_parts(str.data, length))
                }
                StrType::LongAlternate(str) => {
                    std::str::from_utf8_unchecked(slice::from_raw_parts(str.data, length))
                }
                StrType::ShortDefault(str) => std::str::from_utf8_unchecked(&str.data[..length]),
                StrType::ShortAlternate(str) => std::str::from_utf8_unchecked(&str.data[..length]),
            }
        }
    }
}
