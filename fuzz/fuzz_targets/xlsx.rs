#![no_main]

use libfuzzer_sys::fuzz_target;
use tabmux::formats::FormatId;
use tabmux::formats::{SheetSelection, xlsx};
use tabmux::model::SourceRef;

fuzz_target!(|data: &[u8]| {
    let source = SourceRef::new("fuzz.xlsx", FormatId::Xlsx);
    let _ = xlsx::read(
        &source,
        data,
        &xlsx::ReadOptions {
            has_header: true,
            sheets: SheetSelection::All,
        },
    );
});
