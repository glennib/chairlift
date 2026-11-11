//! `chairlift::format` on arbitrary input, in both modes: no panic, the
//! output always passes the verifier (a failure is a bug), and formatting the
//! output again changes nothing.

#![no_main]

use chairlift::Error;
use chairlift::Options;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    for jinja in [false, true] {
        let options = Options::default().with_jinja(jinja);
        match chairlift::format(source, &options) {
            Ok(once) => {
                let twice = chairlift::format(&once, &options);
                assert_eq!(twice.as_ref(), Ok(&once), "not idempotent (jinja: {jinja})");
            }
            Err(Error::Lex(_) | Error::Template(_)) => {}
            Err(e) => panic!("jinja: {jinja}: {e}"),
        }
    }
});
