//! `chairlift::format` on arbitrary input, in both modes and at a width
//! from 10 to 120 that the input decides: no panic, the output always passes
//! the verifier (a failure is a bug), and formatting the output again changes
//! nothing.

#![no_main]

use chairlift::Error;
use chairlift::Options;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    // From the bytes' sum, so that every byte of the input stays its text.
    let width = 10 + u16::from(data.iter().fold(0u8, |sum, &b| sum.wrapping_add(b)) % 111);
    for jinja in [false, true] {
        let options = Options::default().with_jinja(jinja).with_line_width(width);
        match chairlift::format(source, &options) {
            Ok(once) => {
                let twice = chairlift::format(&once, &options);
                assert_eq!(
                    twice.as_ref(),
                    Ok(&once),
                    "not idempotent (jinja: {jinja}, width {width})"
                );
            }
            Err(Error::Lex(_) | Error::Template(_)) => {}
            Err(e) => panic!("jinja: {jinja}, width {width}: {e}"),
        }
    }
});
