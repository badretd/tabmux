#![no_main]

use libfuzzer_sys::fuzz_target;
use tabmux::formats::FormatId;
use tabmux::formats::html::{self, ReadOptions};
use tabmux::model::SourceRef;

fuzz_target!(|data: &[u8]| {
    let source = SourceRef::new("fuzz.html", FormatId::Html);
    let _ = html::read(
        &source,
        data,
        &ReadOptions {
            sheets: tabmux::formats::SheetSelection::All,
            ..ReadOptions::default()
        },
    );
});
