use std::ptr::null_mut;

#[link(name = "Normaliz")]
extern "system" {
    fn IdnToAscii(
        dwFlags: u32,
        lpUnicodeCharStr: *const u16,
        cchUnicodeChar: u32,
        lpASCIICharStr: *mut u16,
        cchASCIIChar: u32,
    ) -> u32;
}

pub(crate) fn encode(unicode_str: *const u16, unicode_str_len: u32) -> Vec<u16> {
    let len = unsafe { IdnToAscii(0, unicode_str, unicode_str_len, null_mut(), 0) };

    let mut output = vec![0u16; (len as usize) + 1];

    unsafe {
        IdnToAscii(
            0,
            unicode_str,
            unicode_str_len,
            output.as_mut_ptr(),
            output.len() as u32,
        );
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    #[test]
    fn happy_path() {
        let string: Vec<u16> = OsStr::new("Gerührtes Käseküchlein mit Äpfeln🍰")
            .encode_wide()
            .chain(once(0))
            .collect();

        let output = encode(string.as_ptr(), string.len() as u32);

        assert_eq!(
            String::from_utf16_lossy(&output).as_str(),
            "xn--gerhrtes ksekchlein mit pfeln-9pco95ela766944b\0\0"
        );
    }
}
