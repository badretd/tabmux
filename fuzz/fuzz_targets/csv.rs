#![no_main]

use libfuzzer_sys::fuzz_target;
use tabmux::formats::FormatId;
use tabmux::formats::csv::ReadOptions;
use tabmux::model::SourceRef;

fuzz_target!(|data: &[u8]| {
    let source = SourceRef::new("fuzz.csv", FormatId::Csv);
    let _ = tabmux::formats::csv::read(&source, data, false, &ReadOptions::default());
});
